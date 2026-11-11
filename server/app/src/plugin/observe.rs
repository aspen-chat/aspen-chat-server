//! Handing plugins what happened. Each observing plugin reads the event stream through a
//! durable JetStream consumer of its own, which every API server pulls from, so each event
//! reaches the plugin once however many servers run. A call that fails is delivered again, up
//! to `MAX_DELIVER` times, and only while the stream retains the event (`MAX_EVENT_AGE`).
//! Events of one channel usually arrive in order but are not guaranteed to.
//!
//! What happens in a DM is published once for each of its people; a plugin is handed the first
//! copy that reaches it (`first_copy`), and a redelivery of that copy again.

use super::host::{self, Instance, Phase, wit};
use super::manifest::ObserveHook;
use super::registry::{LoadedPlugin, Running};
use super::{Mode, PluginPermission};
use crate::context::GlobalServerContext;
use crate::events::{EVENT_ID_HEADER, channel_home};
use crate::message::Message as MessageRow;
use crate::{ASPEN_NATS_STREAM_NAME, AttachmentId, ChannelId, CommunityId, MessageId, UserId};
use aspen_schema::message;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// How many times an event is delivered to a plugin that fails to handle it.
const MAX_DELIVER: i64 = 3;
/// How many events one server hands one plugin at once.
const CONCURRENCY: usize = 8;

/// The consumer's name: the plugin's id with its dots, which NATS does not allow in a name,
/// made underscores, and a digest of the id so two ids never share one.
fn consumer_name(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(id.as_bytes());
    let short: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("plugin_{}_{short}", id.replace(['.', '-'], "_"))
}

/// Handles events for `plugin` for as long as it runs here, starting again after a failure.
pub async fn run(state: GlobalServerContext, plugin: Arc<LoadedPlugin>) {
    loop {
        if let Err(e) = consume(&state, &plugin).await {
            tracing::error!(plugin = plugin.id, "observing stopped: {e}; starting again");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn consume(state: &GlobalServerContext, plugin: &Arc<LoadedPlugin>) -> anyhow::Result<()> {
    use async_nats::jetstream::consumer::{AckPolicy, DeliverPolicy, pull};
    let stream = state
        .nats_context
        .get_stream(ASPEN_NATS_STREAM_NAME)
        .await?;
    let name = consumer_name(&plugin.id);
    let consumer = stream
        .get_or_create_consumer(
            &name,
            pull::Config {
                durable_name: Some(name.clone()),
                deliver_policy: DeliverPolicy::New,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_millis(state.config.plugins.observe_millis)
                    + Duration::from_secs(5),
                max_deliver: MAX_DELIVER,
                filter_subject: format!("{}.>", crate::events::SUBJECT_ROOT),
                ..Default::default()
            },
        )
        .await?;
    consumer
        .messages()
        .await?
        .for_each_concurrent(CONCURRENCY, |received| async move {
            let Ok(received) = received else {
                return;
            };
            let event_id = received
                .headers
                .as_ref()
                .and_then(|h| h.get(EVENT_ID_HEADER))
                .map(|v| v.as_str().to_string());
            let outcome = handle(
                state,
                plugin,
                &received.subject,
                event_id.as_deref(),
                &received.payload,
            )
            .await;
            let ack = match outcome {
                Ok(()) => received.ack().await,
                Err(e) => {
                    tracing::warn!(plugin = plugin.id, "observing an event failed: {e}");
                    received
                        .ack_with(async_nats::jetstream::AckKind::Nak(Some(
                            Duration::from_secs(1),
                        )))
                        .await
                }
            };
            if let Err(e) = ack {
                tracing::warn!(plugin = plugin.id, "acknowledging an event failed: {e}");
            }
        })
        .await;
    Ok(())
}

/// What an observer reads of an event to decide whether its plugin is told of it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Seen {
    server_event: String,
    #[serde(rename = "type")]
    change: Option<String>,
    id: Option<MessageId>,
    content: Option<String>,
    /// On a message's update, present when its attachments changed: those it keeps.
    attachments: Option<Vec<AttachmentId>>,
    /// On a message's update that edited it, when the edit was made, as the row holds it.
    edited_at: Option<DateTime<Utc>>,
    // `botCommandInvoked`
    invocation: Option<MessageId>,
    channel: Option<ChannelId>,
    invoker: Option<UserId>,
    bot: Option<UserId>,
    command: Option<String>,
    arguments: Option<serde_json::Value>,
    // `communityPlugin`
    community: Option<CommunityId>,
    plugin: Option<String>,
    enabled: Option<bool>,
}

/// Whether this copy of an event is the one `plugin` handles: the first to reach it, or a
/// redelivery of that one.
async fn first_copy(
    state: &GlobalServerContext,
    plugin: &str,
    event_id: Option<&str>,
    subject: &str,
) -> crate::Result<bool> {
    use fred::prelude::KeysInterface;
    // Only what is published to each of a DM's people comes in copies.
    let (Some(event_id), Some(crate::events::SubjectOwner::User(_))) =
        (event_id, crate::events::subject_owner(subject))
    else {
        return Ok(true);
    };
    let key = format!("plugin:{plugin}:seen:{event_id}");
    let set: Option<String> = state
        .valkey
        .set(
            &key,
            subject,
            Some(fred::types::Expiration::EX(600)),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await?;
    if set.is_some() {
        return Ok(true);
    }
    let holder: Option<String> = state.valkey.get(&key).await?;
    Ok(holder.as_deref() == Some(subject))
}

/// Whether a row last edited at `stored` holds the edit an event says was made at `published`,
/// or a later one. An event that names no edit asks nothing of the row.
fn holds_edit(stored: Option<DateTime<Utc>>, published: Option<DateTime<Utc>>) -> bool {
    published.is_none_or(|published| stored.is_some_and(|stored| stored >= published))
}

/// Whether a message holding the attachments `stored` shows the change an event made, which
/// left it `published`, or a later one. A posted message only ever loses attachments, so it
/// does once it holds none the event does not list. An event that lists none asks nothing.
fn holds_attachments(stored: &[AttachmentId], published: Option<&[AttachmentId]>) -> bool {
    published.is_none_or(|published| stored.iter().all(|held| published.contains(held)))
}

/// What a message's update says changed, which its row and attachments show once the
/// transaction that published it has committed.
#[derive(Default, Clone, Copy)]
struct Change<'a> {
    /// When the edit was made, of an update that edited its text.
    edited_at: Option<DateTime<Utc>>,
    /// The attachments it keeps, of an update that took one off.
    attachments: Option<&'a [AttachmentId]>,
}

/// The message `id`, waiting a moment for the transaction that wrote it to commit: events are
/// published before it does. A message being made is not there until then, and one being
/// changed is there as it was before: with the text from before an edit, and with the
/// attachment a removal takes off. `None` when there is no such message; an error when the
/// `change` has not shown in time, so the event is delivered again rather than handled with
/// what the message held before.
async fn message_row(
    state: &GlobalServerContext,
    id: MessageId,
    change: Change<'_>,
) -> anyhow::Result<Option<MessageRow>> {
    let mut exists = false;
    for _ in 0..20 {
        let mut conn = state.connection_pool.get().await?;
        let found: Option<MessageRow> = message::table
            .select(MessageRow::as_select())
            .filter(message::id.eq(id))
            .first(conn.as_mut())
            .await
            .optional()?;
        exists = found.is_some();
        if let Some(row) = found
            && holds_edit(row.edited_at, change.edited_at)
        {
            let shown = match change.attachments {
                Some(_) => {
                    let stored = crate::message::attachments_of(conn.as_mut(), id).await?;
                    holds_attachments(&stored, change.attachments)
                }
                None => true,
            };
            if shown {
                return Ok(Some(row));
            }
        }
        drop(conn);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::ensure!(!exists, "the change to message {} has not committed", id.0);
    Ok(None)
}

/// Whether a `communityPlugin` event (its `type` and `enabled`) says a community turned a
/// plugin of `mode` on (`true`) or off (`false`); `None` when it did neither. An update carries
/// `enabled` only when it changed. A community's first record of a plugin is a `create` that
/// carries it whole: that is a plugin turned on only when it is one communities turn on and the
/// record says it is on, since a record made by configuring one that is off says off, and a
/// plugin that runs everywhere was never off.
fn toggled(change: Option<&str>, enabled: Option<bool>, mode: Mode) -> Option<bool> {
    match (change, enabled, mode) {
        (Some("update"), enabled, _) => enabled,
        (Some("create"), Some(true), Mode::OptIn) => Some(true),
        _ => None,
    }
}

/// Where `channel` is, when `plugin` runs there.
async fn running_at(
    state: &GlobalServerContext,
    plugin: &str,
    channel: ChannelId,
) -> crate::Result<Option<Running>> {
    let mut conn = state.connection_pool.get().await?;
    let home = channel_home(state, conn.as_mut(), channel).await?;
    state.plugins.runs_at(conn.as_mut(), plugin, home).await
}

async fn handle(
    state: &GlobalServerContext,
    plugin: &Arc<LoadedPlugin>,
    subject: &str,
    event_id: Option<&str>,
    payload: &[u8],
) -> anyhow::Result<()> {
    let Ok(seen) = serde_json::from_slice::<Seen>(payload) else {
        return Ok(());
    };
    let hooks = &plugin.manifest.hooks.observe;
    let reads = plugin.holds(PluginPermission::MessagesRead);
    let wants = |hook: ObserveHook| hooks.contains(&hook);
    let message_hook = match (seen.server_event.as_str(), seen.change.as_deref()) {
        ("message", Some("create")) if reads && wants(ObserveHook::MessageCreate) => {
            Some(ObserveHook::MessageCreate)
        }
        ("message", Some("update"))
            if reads
                && wants(ObserveHook::MessageEdit)
                && (seen.content.is_some() || seen.attachments.is_some()) =>
        {
            Some(ObserveHook::MessageEdit)
        }
        ("message", Some("delete")) if reads && wants(ObserveHook::MessageDelete) => {
            Some(ObserveHook::MessageDelete)
        }
        _ => None,
    };
    let (observed, running, shown) = if let Some(hook) = message_hook {
        let Some(id) = seen.id else {
            return Ok(());
        };
        // Every observer reads every event, so where the event was published decides first,
        // before anything is read: a community the plugin does not run in, or a DM to a plugin
        // not granted them, is passed over at the cost of the subject alone.
        match crate::events::subject_owner(subject) {
            Some(crate::events::SubjectOwner::Community(community)) => {
                if !state.plugins.runs_in(state, plugin, community).await? {
                    return Ok(());
                }
            }
            Some(crate::events::SubjectOwner::User(_)) if !plugin.holds(PluginPermission::Dms) => {
                return Ok(());
            }
            // A profile subject carries what is about a user, never a message.
            Some(crate::events::SubjectOwner::Profile(_)) => return Ok(()),
            Some(crate::events::SubjectOwner::User(_)) | None => {}
        }
        if !first_copy(state, &plugin.id, event_id, subject).await? {
            return Ok(());
        }
        // Only an update's fields say what changed; a new message's are the message whole.
        let change = if hook == ObserveHook::MessageEdit {
            Change {
                edited_at: seen.edited_at,
                attachments: seen.attachments.as_deref(),
            }
        } else {
            Change::default()
        };
        let Some(row) = message_row(state, id, change).await? else {
            return Ok(());
        };
        // A principal's own messages are not its plugin's to observe.
        if plugin.principal == Some(*row.author.id()) {
            return Ok(());
        }
        let Some(running) = running_at(state, &plugin.id, *row.channel.id()).await? else {
            return Ok(());
        };
        let mut conn = state.connection_pool.get().await?;
        if hook == ObserveHook::MessageDelete {
            let place = host::place(conn.as_mut(), *row.channel.id()).await?;
            (
                wit::Observed::MessageDeleted(wit::DeletedMessage {
                    id: id.0.to_string(),
                    place,
                }),
                running,
                Vec::new(),
            )
        } else {
            let (record, attachments) = host::message_record(conn.as_mut(), &row).await?;
            let observed = if hook == ObserveHook::MessageCreate {
                wit::Observed::MessageCreated(record)
            } else {
                wit::Observed::MessageEdited(record)
            };
            (observed, running, attachments)
        }
    } else if seen.server_event == "botCommandInvoked" {
        let (Some(invocation), Some(channel), Some(invoker), Some(bot), Some(name)) = (
            seen.invocation,
            seen.channel,
            seen.invoker,
            seen.bot,
            seen.command.clone(),
        ) else {
            return Ok(());
        };
        if plugin.principal != Some(bot) || !wants(ObserveHook::CommandInvoke) {
            return Ok(());
        }
        let Some(running) = running_at(state, &plugin.id, channel).await? else {
            return Ok(());
        };
        let mut conn = state.connection_pool.get().await?;
        let place = host::place(conn.as_mut(), channel).await?;
        // The invoker's roles are those of the community the command was sent in.
        let community = seen.community;
        let invoker = host::person(conn.as_mut(), invoker, community).await?;
        (
            wit::Observed::CommandInvoked(wit::Command {
                invocation: invocation.0.to_string(),
                place,
                invoker,
                name,
                arguments: seen
                    .arguments
                    .as_ref()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "[]".into()),
            }),
            running,
            Vec::new(),
        )
    } else if seen.server_event == "communityPlugin" && seen.plugin.as_deref() == Some(&plugin.id) {
        let Some(community) = seen.community else {
            return Ok(());
        };
        let observed = match toggled(seen.change.as_deref(), seen.enabled, plugin.mode) {
            Some(true) if wants(ObserveHook::PluginEnable) => {
                wit::Observed::PluginEnabled(community.0.to_string())
            }
            Some(false) if wants(ObserveHook::PluginDisable) => {
                wit::Observed::PluginDisabled(community.0.to_string())
            }
            _ => return Ok(()),
        };
        let mut conn = state.connection_pool.get().await?;
        let running = state
            .plugins
            .running_in_community(conn.as_mut(), community)
            .await?
            .into_iter()
            .find(|r| r.plugin.id == plugin.id)
            .unwrap_or(Running {
                plugin: plugin.clone(),
                community_settings: None,
            });
        (observed, running, Vec::new())
    } else {
        return Ok(());
    };
    let community = match &observed {
        wit::Observed::MessageCreated(m) | wit::Observed::MessageEdited(m) => {
            m.place.community.clone()
        }
        wit::Observed::MessageDeleted(m) => m.place.community.clone(),
        wit::Observed::CommandInvoked(c) => c.place.community.clone(),
        wit::Observed::PluginEnabled(c) | wit::Observed::PluginDisabled(c) => Some(c.clone()),
        wit::Observed::TimerFired(_) => None,
    };
    deliver(state, &running, observed, community, event_id, shown).await
}

/// Hands `observed` to `running`'s plugin, with the community it happened in and that
/// community's settings, and the attachments it may read; an error when its call failed, which
/// is delivered again.
pub(super) async fn deliver(
    state: &GlobalServerContext,
    running: &Running,
    observed: wit::Observed,
    community: Option<String>,
    event_id: Option<&str>,
    shown: Vec<AttachmentId>,
) -> anyhow::Result<()> {
    let context = wit::Context {
        community,
        community_settings: running
            .community_settings
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default()),
        locale: None,
        event_id: event_id.map(str::to_string),
    };
    let budget = Duration::from_millis(state.config.plugins.observe_millis);
    let mut instance = Instance::new(
        state,
        running.plugin.clone(),
        Phase::Observe,
        shown.into_iter().collect(),
        budget,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let deadline = instance.deadline;
    let answer = host::within(
        deadline,
        instance.bindings.aspen_plugin_hooks().call_observe(
            &mut instance.store,
            &observed,
            &context,
        ),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    answer.map_err(|e| anyhow::anyhow!("the plugin said: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_names_are_distinct_and_dotless() {
        let a = consumer_name("org.example.a-b");
        let b = consumer_name("org.example.a.b");
        assert!(!a.contains('.') && !b.contains('.'));
        assert_ne!(a, b);
    }

    #[test]
    fn a_plugin_is_told_it_was_turned_on_or_off_only_when_it_was() {
        for mode in [Mode::OptIn, Mode::Everywhere] {
            assert_eq!(toggled(Some("update"), Some(true), mode), Some(true));
            assert_eq!(toggled(Some("update"), Some(false), mode), Some(false));
            // Its settings changed there.
            assert_eq!(toggled(Some("update"), None, mode), None);
            // Configured where it has not been turned on.
            assert_eq!(toggled(Some("create"), Some(false), mode), None);
            assert_eq!(toggled(Some("delete"), None, mode), None);
        }
        assert_eq!(toggled(Some("create"), Some(true), Mode::OptIn), Some(true));
        // First configured where it always ran.
        assert_eq!(toggled(Some("create"), Some(true), Mode::Everywhere), None);
    }

    /// A removal's update names no edit, only the attachments the message keeps: the message
    /// shows it once it holds none but those, the removed one being there until the commit.
    #[test]
    fn a_removed_attachment_is_awaited_by_the_ones_its_event_keeps() {
        use aspen_wire::message_enum::server_event::{MessageEvent, ServerEvent};
        let file = |n: u128| AttachmentId(uuid::Uuid::from_u128(n));
        let event = ServerEvent::Message(MessageEvent::Update {
            id: MessageId::new(),
            content: None,
            attachments: Some(vec![file(1), file(3)]),
            edited_at: None,
            link_previews: None,
            thread: None,
            mentions: None,
            linked_messages: None,
            altered_by: None,
            card: None,
            echo: None,
        });
        let seen: Seen = serde_json::from_slice(&serde_json::to_vec(&event).unwrap()).unwrap();
        assert_eq!(seen.edited_at, None);
        let kept = seen.attachments.as_deref();
        assert_eq!(kept, Some([file(1), file(3)].as_slice()));

        // Before the commit the removed file is still the message's.
        assert!(!holds_attachments(&[file(1), file(2), file(3)], kept));
        assert!(holds_attachments(&[file(1), file(3)], kept));
        // A later removal has committed too.
        assert!(holds_attachments(&[file(3)], kept));
        assert!(holds_attachments(&[], kept));
        // The last file taken off leaves none.
        assert!(!holds_attachments(&[file(2)], Some(&[])));
        assert!(holds_attachments(&[], Some(&[])));
        // An update that says nothing of attachments asks nothing.
        assert!(holds_attachments(&[file(2)], None));
    }

    #[test]
    fn an_edit_is_awaited_by_the_time_its_event_carries() {
        use aspen_wire::message_enum::server_event::{MessageEvent, ServerEvent};
        // To the microsecond, as the row keeps it.
        let edited = DateTime::from_timestamp_micros(1_800_000_000_123_456).unwrap();
        let event = ServerEvent::Message(MessageEvent::Update {
            id: MessageId::new(),
            content: Some("after".to_string()),
            attachments: None,
            edited_at: Some(Some(edited)),
            link_previews: None,
            thread: None,
            mentions: None,
            linked_messages: None,
            altered_by: None,
            card: None,
            echo: None,
        });
        let seen: Seen = serde_json::from_slice(&serde_json::to_vec(&event).unwrap()).unwrap();
        assert_eq!(seen.edited_at, Some(edited));

        let before = edited - chrono::Duration::seconds(5);
        assert!(!holds_edit(None, Some(edited)));
        assert!(!holds_edit(Some(before), Some(edited)));
        assert!(holds_edit(Some(edited), Some(edited)));
        assert!(holds_edit(
            Some(edited + chrono::Duration::seconds(5)),
            Some(edited)
        ));
        assert!(holds_edit(None, None));
        assert!(holds_edit(Some(before), None));
    }
}
