//! Deciding a message before it is saved (`message.create`, `message.edit`). The plugins that
//! intercept the hook where the message is run in the operator's order, before the transaction
//! that saves it opens, each seeing the text the one before left. Each may allow it, rewrite
//! its text, or refuse it with a reason the author reads in their language; a call that fails
//! or runs out of time counts as allowing or refusing as its manifest says.
//!
//! A rewrite never tags anyone or anything the author's text did not, and the plugins that
//! rewrote a message are recorded with it (`alteredBy`).

use super::host::{self, Instance, Phase, wit};
use super::manifest::{Failure, InterceptHook};
use super::registry::Running;
use super::{PluginPermission, PluginText};
use crate::app::context::GlobalServerContext;
use crate::app::events::channel_home;
use crate::app::permissions::ChannelAccess;
use crate::app::{self, AttachmentId, MessageId, UserId, mention};
use crate::t;
use std::collections::HashSet;
use std::time::Duration;

/// The longest a rewrite may make a message: the longer of the author's text and this many
/// characters.
const REWRITE_ROOM: usize = 4000;

/// A message about to be saved.
pub struct Draft<'a> {
    pub author: UserId,
    pub access: &'a ChannelAccess,
    pub content: String,
    pub attachments: &'a [AttachmentId],
    /// The message being edited, for `message.edit`.
    pub editing: Option<MessageId>,
}

/// What the plugins left of a draft: its text, and who rewrote it.
pub struct Decided {
    pub content: String,
    pub altered_by: Vec<String>,
}

/// Whether any plugin intercepts `hook` where `access`'s channel is, so the work of preparing a
/// draft is done only when one does.
pub async fn wanted(
    state: &GlobalServerContext,
    conn: &mut diesel_async::AsyncPgConnection,
    hook: InterceptHook,
    channel: app::ChannelId,
) -> app::Result<Vec<Running>> {
    if !state.plugins.any() {
        return Ok(Vec::new());
    }
    let home = channel_home(state, conn, channel).await?;
    Ok(state
        .plugins
        .running_at(conn, home)
        .await?
        .into_iter()
        .filter(|r| {
            r.plugin.manifest.hooks.intercept.contains_key(&hook)
                && r.plugin.holds(PluginPermission::MessagesRead)
        })
        .collect())
}

/// Whether `rewritten` tags only what `original` does.
fn tags_within(original: &str, rewritten: &str) -> bool {
    let before = mention::parse(original);
    let after = mention::parse(rewritten);
    after.users.iter().all(|u| before.users.contains(u))
        && after.roles.iter().all(|r| before.roles.contains(r))
        && (!after.everyone || before.everyone)
}

/// Runs `running`'s plugins on `draft` in order, returning what they left of it, or the first
/// refusal as `pluginRefused` (or `pluginUnavailable`, for a plugin that fails closed).
pub async fn decide(
    state: &GlobalServerContext,
    hook: InterceptHook,
    running: Vec<Running>,
    draft: Draft<'_>,
) -> app::Result<Decided> {
    let locale = app::locale::current();
    let mut content = draft.content;
    let mut altered_by = Vec::new();
    if running.is_empty() {
        return Ok(Decided {
            content,
            altered_by,
        });
    }
    let community = draft.access.community.as_ref().map(|c| c.community);
    let (author, place, attachments) = {
        let mut conn = state.connection_pool.get().await?;
        (
            host::person(conn.as_mut(), draft.author, community).await?,
            host::place(conn.as_mut(), draft.access.channel).await?,
            host::attachments(conn.as_mut(), draft.attachments).await?,
        )
    };
    let shown: HashSet<AttachmentId> = draft.attachments.iter().copied().collect();
    let budget = Duration::from_millis(state.config.plugins.intercept_millis);
    for running in running {
        let plugin = running.plugin.clone();
        // A principal's own messages are not its plugin's to decide.
        if plugin.principal == Some(draft.author) {
            continue;
        }
        let failure = plugin
            .manifest
            .hooks
            .intercept
            .get(&hook)
            .map_or(Failure::Open, |p| p.failure);
        let input = wit::Draft {
            author: author.clone(),
            place: place.clone(),
            content: content.clone(),
            attachments: attachments.clone(),
            editing: draft.editing.map(|id| id.0.to_string()),
        };
        let context = wit::Context {
            community: community.map(|c| c.0.to_string()),
            community_settings: running
                .community_settings
                .as_ref()
                .map(|s| serde_json::to_string(s).unwrap_or_default()),
            locale: Some(locale.to_string()),
            event_id: None,
        };
        let outcome = async {
            let mut instance = Instance::new(
                state,
                plugin.clone(),
                Phase::Intercept,
                shown.clone(),
                budget,
            )
            .await?;
            let deadline = instance.deadline;
            let verdict = host::within(
                deadline,
                instance.bindings.aspen_plugin_hooks().call_intercept(
                    &mut instance.store,
                    &input,
                    &context,
                ),
            )
            .await?;
            let deferred = std::mem::take(&mut instance.store.data_mut().call.deferred);
            Ok::<_, host::CallFailed>((verdict, deferred))
        }
        .await;
        let verdict = match outcome {
            Ok((verdict, deferred)) => {
                super::principal::run_deferred(state, plugin.clone(), deferred);
                verdict
            }
            Err(failed) => {
                tracing::warn!(
                    plugin = plugin.id,
                    "intercepting a message failed: {failed}"
                );
                match failure {
                    Failure::Open => continue,
                    Failure::Closed => {
                        return Err(app::Error::PluginUnavailable(t!(
                            "pluginUnavailableDetail",
                            plugin = plugin.name(locale)
                        )));
                    }
                }
            }
        };
        match verdict {
            wit::Verdict::Allow => {}
            wit::Verdict::Rewrite(rewritten) => {
                let room = content.chars().count().max(REWRITE_ROOM);
                let fits = rewritten.chars().count() <= room;
                if !plugin.holds(PluginPermission::MessagesRewrite)
                    || !fits
                    || !tags_within(&content, &rewritten)
                {
                    tracing::warn!(
                        plugin = plugin.id,
                        "a rewrite was refused: it lacks messages.rewrite, is too long, or tags \
                         what the text did not"
                    );
                    if failure == Failure::Closed {
                        return Err(app::Error::PluginUnavailable(t!(
                            "pluginUnavailableDetail",
                            plugin = plugin.name(locale)
                        )));
                    }
                    continue;
                }
                if rewritten != content {
                    content = rewritten;
                    altered_by.push(plugin.id.clone());
                }
            }
            wit::Verdict::Refuse(reason) => {
                if !plugin.holds(PluginPermission::MessagesRefuse) {
                    tracing::warn!(plugin = plugin.id, "a refusal without messages.refuse");
                    continue;
                }
                let reason = PluginText::from(reason);
                return Err(app::Error::PluginRefused(
                    plugin.render(locale, &reason).into(),
                ));
            }
        }
    }
    Ok(Decided {
        content,
        altered_by,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rewrite_may_drop_tags_but_never_add_them() {
        let user = uuid::Uuid::now_v7();
        let tagged = format!("hi <@{user}>");
        assert!(tags_within(&tagged, "hi there"));
        assert!(tags_within(&tagged, &tagged));
        assert!(!tags_within("hi there", &tagged));
        assert!(!tags_within("hi", "hi @everyone"));
    }
}
