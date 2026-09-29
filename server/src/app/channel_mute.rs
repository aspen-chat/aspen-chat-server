//! Channels a user has muted, for themself alone: a text channel or a DM, for a while or until
//! they lift it. A muted channel is dimmed in the user's lists and never shows as unread there,
//! though its read position is kept, so what arrived meanwhile is unread once the mute ends.
//! Every change is published to the user's own subject as `channelMuteChanged`, so their other
//! devices follow; a mute that runs out ends on each device by its own clock, with no event.

use crate::api::ChannelType;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::{
    self, ChannelId, CommunityId, EventScope, GlobalServerContext, UserId, publish_event,
};
use crate::database::schema::{channel, channel_mute};
use crate::t;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

/// The longest a timed mute may last: a year. Longer is an indefinite mute.
pub const MAX_MUTE_SECONDS: u32 = 366 * 24 * 60 * 60;

/// A channel the user has muted, and until when; `None` until they unmute it.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = channel_mute)]
pub struct ChannelMute {
    pub channel: ChannelId,
    pub until: Option<DateTime<Utc>>,
}

/// Whether a mute ending at `until` is still in force at `now`.
fn active(until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    until.is_none_or(|until| until > now)
}

/// Mutes `channel_id` for the user, for `duration_seconds` or, with `None`, until they unmute
/// it, replacing any mute already on it. Returns the mute and whether one was already in force.
pub async fn mute(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
    duration_seconds: Option<u32>,
) -> app::Result<(ChannelMute, bool)> {
    if duration_seconds.is_some_and(|s| s == 0 || s > MAX_MUTE_SECONDS) {
        return Err(app::Error::Validation(t!(
            "channelMuteDuration",
            max = MAX_MUTE_SECONDS / (24 * 60 * 60)
        )));
    }
    let mut conn = state.connection_pool.get().await?;
    crate::app::permissions::channel_access(state, conn.as_mut(), user, channel_id).await?;
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    if !matches!(
        ty,
        ChannelType::Text | ChannelType::Dm | ChannelType::GroupDm
    ) {
        return Err(app::Error::Validation(t!("channelMuteKind")));
    }
    let now = Utc::now();
    let until = duration_seconds.map(|s| now + chrono::Duration::seconds(i64::from(s)));
    conn.transaction(|conn| {
        async move {
            let previous: Option<Option<DateTime<Utc>>> = channel_mute::table
                .select(channel_mute::until)
                .filter(
                    channel_mute::user
                        .eq(user)
                        .and(channel_mute::channel.eq(channel_id)),
                )
                .for_update()
                .first(conn.as_mut())
                .await
                .optional()?;
            diesel::insert_into(channel_mute::table)
                .values((
                    channel_mute::user.eq(user),
                    channel_mute::channel.eq(channel_id),
                    channel_mute::until.eq(until),
                ))
                .on_conflict((channel_mute::user, channel_mute::channel))
                .do_update()
                .set(channel_mute::until.eq(until))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(user),
                &ServerEvent::ChannelMuteChanged {
                    channel: channel_id,
                    muted: true,
                    until,
                },
            )
            .await?;
            let existed = previous.is_some_and(|until| active(until, now));
            Ok::<_, app::Error>((
                ChannelMute {
                    channel: channel_id,
                    until,
                },
                existed,
            ))
        }
        .scope_boxed()
    })
    .await
}

/// Lifts the user's mute of `channel_id`, if there is one.
pub async fn unmute(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                channel_mute::table.filter(
                    channel_mute::user
                        .eq(user)
                        .and(channel_mute::channel.eq(channel_id)),
                ),
            )
            .execute(conn.as_mut())
            .await?;
            if removed > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(user),
                    &ServerEvent::ChannelMuteChanged {
                        channel: channel_id,
                        muted: false,
                        until: None,
                    },
                )
                .await?;
            }
            Ok::<_, app::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// The user's mutes in force of the listed channels and of every channel in the listed
/// communities, in one query.
pub async fn read_mutes(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
    communities: &[CommunityId],
) -> app::Result<Vec<ChannelMute>> {
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now();
    let mut query = channel_mute::table
        .inner_join(channel::table)
        .select(ChannelMute::as_select())
        .filter(channel_mute::user.eq(user))
        .filter(
            channel_mute::until
                .is_null()
                .or(channel_mute::until.gt(now)),
        )
        .into_boxed();
    query = query.filter(
        channel::id
            .eq_any(channels.to_vec())
            .or(channel::community.eq_any(communities.to_vec())),
    );
    Ok(query.load(conn.as_mut()).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mute_lasts_until_its_end_or_for_good() {
        let now = Utc::now();
        assert!(active(None, now));
        assert!(active(Some(now + chrono::Duration::seconds(1)), now));
        assert!(!active(Some(now), now));
        assert!(!active(Some(now - chrono::Duration::seconds(1)), now));
    }
}
