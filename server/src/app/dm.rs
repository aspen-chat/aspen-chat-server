//! Direct messages: DMs between two people and group DMs among up to `MAX_RECIPIENTS`. Both are
//! channels that belong to no community; their people are rows of `dm_recipient`, and what
//! happens in them is published to each recipient alone (`app::events`). Only recipients may
//! read or write a DM, or a thread in one; to anyone else it does not exist. A DM may only be
//! started with, or joined by, people who share a community with the one starting or adding,
//! and never with two people who have a block between them (`app::block`). A holder of Message
//! any user (`app::deployment`) is held to neither: they may start a DM with anyone, or add
//! anyone, whatever communities they share and whoever blocked them, though two others with a
//! block between them still cannot be brought together.

use crate::api::message_enum::server_event::{ChannelEvent, ServerEvent};
use crate::app::channel::ChannelType;
use crate::app::channel::{Channel, record};
use crate::app::context::GlobalServerContext;
use crate::app::events::dm_recipients;
use crate::app::{self, ChannelId, EventScope, UserId, publish_event};
use crate::database::schema::{channel, community_user, dm_recipient, message, user};
use crate::t;
use chrono::Utc;
use diesel::prelude::*;
use diesel::upsert::excluded;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::{HashMap, HashSet};

/// The most people a group DM holds, its creator included. Every message in a DM is published
/// once per recipient, so this bounds a DM's publishing cost.
pub const MAX_RECIPIENTS: usize = 10;

/// The `dm_key` of the one-to-one DM between two people: their ids in order, so the pair has
/// one key whichever of them starts it.
pub fn pair_key(a: UserId, b: UserId) -> String {
    let (low, high) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    format!("{}:{}", low.0, high.0)
}

/// Refuses the users in `others` who share no community with `user`.
async fn ensure_shared_community(
    conn: &mut AsyncPgConnection,
    user: UserId,
    others: &[UserId],
) -> app::Result<()> {
    let communities: Vec<crate::app::CommunityId> = community_user::table
        .select(community_user::community)
        .filter(community_user::user.eq(user))
        .load(conn)
        .await?;
    let reachable: HashSet<UserId> = community_user::table
        .select(community_user::user)
        .filter(
            community_user::user
                .eq_any(others)
                .and(community_user::community.eq_any(&communities)),
        )
        .distinct()
        .load::<UserId>(conn)
        .await?
        .into_iter()
        .collect();
    if others.iter().all(|other| reachable.contains(other)) {
        Ok(())
    } else {
        Err(app::Error::Validation(t!("dmNeedsSharedCommunity")))
    }
}

/// Whether `user` holds Message any user, which reaches anyone past shared communities and
/// blocks.
async fn messages_anyone(conn: &mut AsyncPgConnection, user: UserId) -> app::Result<bool> {
    Ok(app::deployment::deployment_access(conn, user)
        .await?
        .has(app::deployment::DeploymentPermission::MessageAnyUser))
}

/// Refuses a DM with the system account, whose notices are the only DMs it is in.
async fn refuse_system_account(conn: &mut AsyncPgConnection, others: &[UserId]) -> app::Result<()> {
    let system: bool = diesel::select(diesel::dsl::exists(
        user::table.filter(user::id.eq_any(others).and(user::system)),
    ))
    .get_result(conn)
    .await?;
    if system {
        return Err(app::Error::Validation(t!("systemAccountNoDm")));
    }
    Ok(())
}

fn new_dm(ty: ChannelType, dm_key: Option<String>) -> Channel {
    Channel {
        id: ChannelId::new(),
        community: None,
        parent_category: None,
        name: String::new(),
        ty,
        sort_index: 0,
        deleted_at: None,
        parent_channel: None,
        starter_message: None,
        reply_count: 0,
        last_reply_at: None,
        dm_key,
        plugin_type: None,
    }
}

/// A DM, and whether this call made it. With one other person it is their one-to-one DM, made
/// on first use and returned as it is afterwards; with more it is a new group DM. A block
/// between any two of the people refuses a DM that does not exist yet, while a one-to-one DM
/// made before the block is still returned, to be read.
pub async fn open_dm(
    state: &GlobalServerContext,
    caller: UserId,
    with: Vec<UserId>,
) -> app::Result<(Channel, Vec<UserId>, bool)> {
    let mut others: Vec<UserId> = Vec::new();
    for user in with {
        if user != caller && !others.contains(&user) {
            others.push(user);
        }
    }
    if others.is_empty() {
        return Err(app::Error::Validation(t!("dmNeedsRecipient")));
    }
    if others.len() + 1 > MAX_RECIPIENTS {
        return Err(app::Error::Validation(t!(
            "dmTooManyRecipients",
            max = MAX_RECIPIENTS
        )));
    }
    let mut conn = state.connection_pool.get().await?;
    refuse_system_account(conn.as_mut(), &others).await?;
    let anyone = messages_anyone(conn.as_mut(), caller).await?;
    if !anyone {
        ensure_shared_community(conn.as_mut(), caller, &others).await?;
    }
    let mut everyone = others.clone();
    everyone.push(caller);
    // A DM is this deployment's to host only with one of its own users in it; people who all
    // belong elsewhere talk on a deployment of theirs.
    let natives: i64 = user::table
        .filter(user::id.eq_any(&everyone))
        .filter(user::home_domain.is_null())
        .count()
        .get_result(conn.as_mut())
        .await?;
    if natives == 0 {
        return Err(app::Error::FederationRefused(t!("dmNeedsNative")));
    }
    let blocked =
        app::block::any_between(conn.as_mut(), if anyone { &others } else { &everyone }).await?;
    let opened = conn
        .transaction(|conn| {
            async move {
                if blocked {
                    if others.len() > 1 {
                        return Err(app::Error::Blocked);
                    }
                    let existing: Channel = channel::table
                        .select(Channel::as_select())
                        .filter(channel::dm_key.eq(pair_key(caller, others[0])))
                        .first(conn.as_mut())
                        .await
                        .optional()?
                        .ok_or(app::Error::Blocked)?;
                    let recipients = dm_recipients(conn.as_mut(), existing.id).await?;
                    return Ok((existing, recipients, false));
                }
                insert_dm(state, conn.as_mut(), caller, &others).await
            }
            .scope_boxed()
        })
        .await?;
    let (dm, recipients, created) = &opened;
    if *created {
        app::federation::notices::announce_dm(state, dm.id, recipients.clone(), caller);
    }
    Ok(opened)
}

/// Makes the DM between `caller` and `others` and announces it: their one-to-one DM with one
/// other person, returned as it is when it exists already, or a new group DM with more.
pub(crate) async fn insert_dm(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    caller: UserId,
    others: &[UserId],
) -> app::Result<(Channel, Vec<UserId>, bool)> {
    let (ty, key) = if others.len() == 1 {
        (ChannelType::Dm, Some(pair_key(caller, others[0])))
    } else {
        (ChannelType::GroupDm, None)
    };
    let dm = new_dm(ty, key.clone());
    // Concurrent first messages between the same two people make one DM: the second
    // insert yields to the first, and the existing one is returned.
    let inserted = diesel::insert_into(channel::table)
        .values(&dm)
        .on_conflict(channel::dm_key)
        .do_nothing()
        .execute(conn)
        .await?;
    if inserted == 0 {
        let existing: Channel = channel::table
            .select(Channel::as_select())
            .filter(channel::dm_key.eq(key))
            .first(conn)
            .await?;
        let recipients = dm_recipients(conn, existing.id).await?;
        return Ok((existing, recipients, false));
    }
    let now = Utc::now();
    let mut recipients = vec![caller];
    recipients.extend(others.iter().copied());
    diesel::insert_into(dm_recipient::table)
        .values(
            recipients
                .iter()
                .map(|user| {
                    (
                        dm_recipient::channel.eq(dm.id),
                        dm_recipient::user.eq(*user),
                        dm_recipient::joined_at.eq(now),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::ChannelDefinition {
            channel: dm.id,
            departed: None,
        },
        &ServerEvent::Channel(ChannelEvent::Create(record(&dm, recipients.clone()))),
    )
    .await?;
    Ok((dm, recipients, true))
}

/// The caller's DMs and group DMs with their recipients, the most recently active first.
pub async fn list_dms(
    state: &GlobalServerContext,
    caller: UserId,
) -> app::Result<Vec<(Channel, Vec<UserId>)>> {
    let mut conn = state.connection_pool.get().await?;
    let channels: Vec<Channel> = channel::table
        .inner_join(dm_recipient::table.on(dm_recipient::channel.eq(channel::id)))
        .select(Channel::as_select())
        .filter(
            dm_recipient::user
                .eq(caller)
                .and(channel::deleted_at.is_null()),
        )
        .load(conn.as_mut())
        .await?;
    let ids: Vec<ChannelId> = channels.iter().map(|c| c.id).collect();
    let mut recipients: HashMap<ChannelId, Vec<UserId>> = HashMap::new();
    for (dm, user) in dm_recipient::table
        .select((dm_recipient::channel, dm_recipient::user))
        .filter(dm_recipient::channel.eq_any(&ids))
        .order_by(dm_recipient::joined_at.asc())
        .load::<(ChannelId, UserId)>(conn.as_mut())
        .await?
    {
        recipients.entry(dm).or_default().push(user);
    }
    let latest: HashMap<ChannelId, chrono::DateTime<Utc>> = message::table
        .filter(
            message::channel
                .eq_any(&ids)
                .and(message::deleted_at.is_null()),
        )
        .group_by(message::channel)
        .select((message::channel, diesel::dsl::max(message::timestamp)))
        .load::<(ChannelId, Option<chrono::DateTime<Utc>>)>(conn.as_mut())
        .await?
        .into_iter()
        .filter_map(|(dm, last)| last.map(|last| (dm, last)))
        .collect();
    let mut listed: Vec<(Channel, Vec<UserId>)> = channels
        .into_iter()
        .map(|c| {
            let people = recipients.remove(&c.id).unwrap_or_default();
            (c, people)
        })
        .collect();
    // Latest message first; DMs with none after them, newest first (channel ids are UUIDv7).
    listed.sort_by_key(|(c, _)| std::cmp::Reverse((latest.get(&c.id).copied(), c.id.0)));
    Ok(listed)
}

/// Adds someone to a group DM the caller is in; whether they were added, rather than already
/// there.
pub async fn add_recipient(
    state: &GlobalServerContext,
    caller: UserId,
    dm_id: ChannelId,
    user: UserId,
) -> app::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    let dm: Channel = channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(dm_id).and(channel::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    crate::app::permissions::channel_access(state, conn.as_mut(), caller, dm_id).await?;
    if dm.ty != ChannelType::GroupDm {
        return Err(app::Error::Validation(t!("dmNotGroup")));
    }
    refuse_system_account(conn.as_mut(), &[user]).await?;
    let anyone = messages_anyone(conn.as_mut(), caller).await?;
    if !anyone {
        ensure_shared_community(conn.as_mut(), caller, &[user]).await?;
    }
    let added = conn
        .transaction(|conn| {
            async move {
                // The DM row is locked so two additions cannot both see room for one more.
                channel::table
                    .select(channel::id)
                    .filter(channel::id.eq(dm_id))
                    .for_update()
                    .first::<ChannelId>(conn.as_mut())
                    .await?;
                let recipients = dm_recipients(conn.as_mut(), dm_id).await?;
                if recipients.contains(&user) {
                    return Ok(false);
                }
                let mut joined = recipients.clone();
                joined.push(user);
                if anyone {
                    joined.retain(|person| *person != caller);
                }
                if app::block::any_between(conn.as_mut(), &joined).await? {
                    return Err(app::Error::Blocked);
                }
                if recipients.len() >= MAX_RECIPIENTS {
                    return Err(app::Error::Validation(t!(
                        "dmTooManyRecipients",
                        max = MAX_RECIPIENTS
                    )));
                }
                diesel::insert_into(dm_recipient::table)
                    .values((
                        dm_recipient::channel.eq(dm_id),
                        dm_recipient::user.eq(user),
                        dm_recipient::joined_at.eq(Utc::now()),
                    ))
                    .on_conflict((dm_recipient::channel, dm_recipient::user))
                    .do_update()
                    .set(dm_recipient::joined_at.eq(excluded(dm_recipient::joined_at)))
                    .execute(conn.as_mut())
                    .await?;
                let mut now = recipients;
                now.push(user);
                // The whole record, as a `Create`: the newcomer gains the DM, and everyone else's
                // copy is replaced by one with the new recipient in it.
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::ChannelDefinition {
                        channel: dm_id,
                        departed: None,
                    },
                    &ServerEvent::Channel(ChannelEvent::Create(record(&dm, now))),
                )
                .await?;
                Ok(true)
            }
            .scope_boxed()
        })
        .await?;
    if added {
        app::federation::notices::announce_dm(state, dm_id, vec![user], caller);
    }
    Ok(added)
}

/// Takes the caller out of a group DM. The others see them go; the caller's own clients see
/// the recipients without them, which is how they learn to drop the DM.
pub async fn leave(
    state: &GlobalServerContext,
    caller: UserId,
    dm_id: ChannelId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let dm: Channel = channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(dm_id).and(channel::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    crate::app::permissions::channel_access(state, conn.as_mut(), caller, dm_id).await?;
    if dm.ty != ChannelType::GroupDm {
        return Err(app::Error::Validation(t!("dmNotGroup")));
    }
    conn.transaction(|conn| {
        async move {
            diesel::delete(dm_recipient::table)
                .filter(
                    dm_recipient::channel
                        .eq(dm_id)
                        .and(dm_recipient::user.eq(caller)),
                )
                .execute(conn.as_mut())
                .await?;
            let remaining = dm_recipients(conn.as_mut(), dm_id).await?;
            publish_recipients(state, conn.as_mut(), dm_id, remaining, Some(caller)).await
        }
        .scope_boxed()
    })
    .await
}

/// Announces a DM's recipients after someone left, to those who remain and to `departed`.
async fn publish_recipients(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    dm: ChannelId,
    recipients: Vec<UserId>,
    departed: Option<UserId>,
) -> app::Result<()> {
    publish_event(
        state,
        conn,
        EventScope::ChannelDefinition {
            channel: dm,
            departed,
        },
        &ServerEvent::Channel(ChannelEvent::Update {
            id: dm,
            parent_category: None,
            community: None,
            name: None,
            sort_index: None,
            reply_count: None,
            last_reply_at: None,
            recipients: Some(recipients),
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_has_one_key_whoever_starts() {
        let a = UserId::new();
        let b = UserId::new();
        assert_eq!(pair_key(a, b), pair_key(b, a));
        assert_ne!(pair_key(a, b), pair_key(a, UserId::new()));
    }
}
