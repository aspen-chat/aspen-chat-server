//! Channels a user has muted, for themself alone: a text channel or a DM, for a while or until
//! they lift it. A muted channel is dimmed in the user's lists and never shows as unread there,
//! though its read position is kept, so what arrived meanwhile is unread once the mute ends.
//! A channel in a category the user muted (`app::category_mute`) is muted too, unless its own
//! row says otherwise: muted, with its own end, or kept unmuted. Every change is published to
//! the user's own subject as `channelMuteChanged`, so their other devices follow; a mute that
//! runs out ends on each device by its own clock, with no event.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::t;
use crate::{ChannelId, CommunityId, EventScope, UserId, publish_event};
use aspen_schema::{channel, channel_mute};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

/// The longest a timed mute may last: a year. Longer is an indefinite mute.
pub const MAX_MUTE_SECONDS: u32 = 366 * 24 * 60 * 60;

/// The user's own setting for a channel: muted, and until when (`None` until they unmute it),
/// or kept unmuted (`muted` false, with no end) while its category is muted.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = channel_mute)]
pub struct ChannelMute {
    pub channel: ChannelId,
    pub until: Option<DateTime<Utc>>,
    pub muted: bool,
}

/// What the user asks of a channel: to mute it, for `duration_seconds` or until they lift it,
/// or to keep it unmuted while its category is muted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMuteChoice {
    Muted { duration_seconds: Option<u32> },
    KeptUnmuted,
}

/// SQL that is true while the user `$user` has `$channel` muted, both SQL expressions: by its
/// own row in force, or, without one, by a mute in force of its category (`category_mute`).
/// Each is found through its table's key.
macro_rules! muted_sql {
    ($user:literal, $channel:literal) => {
        concat!(
            "COALESCE((SELECT own.muted FROM channel_mute own WHERE own.\"user\" = ",
            $user,
            " AND own.channel = ",
            $channel,
            " AND (own.until IS NULL OR own.until > now())), EXISTS (SELECT 1 FROM channel mc",
            " JOIN category_mute km ON km.category = mc.parent_category WHERE mc.id = ",
            $channel,
            " AND km.\"user\" = ",
            $user,
            " AND (km.until IS NULL OR km.until > now())))"
        )
    };
}
pub(crate) use muted_sql;

/// Whether a mute ending at `until` is still in force at `now`.
pub(crate) fn active(until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    until.is_none_or(|until| until > now)
}

/// When a mute asked to last `duration_seconds` from `now` ends, refusing a length of none or
/// past `MAX_MUTE_SECONDS`.
pub(crate) fn mute_end(
    duration_seconds: Option<u32>,
    now: DateTime<Utc>,
) -> crate::Result<Option<DateTime<Utc>>> {
    if duration_seconds.is_some_and(|s| s == 0 || s > MAX_MUTE_SECONDS) {
        return Err(crate::Error::Validation(t!(
            "channelMuteDuration",
            max = MAX_MUTE_SECONDS / (24 * 60 * 60)
        )));
    }
    Ok(duration_seconds.map(|s| now + chrono::Duration::seconds(i64::from(s))))
}

/// Sets the user's own setting for `channel_id` as `choice` asks, replacing any already on it.
/// Returns the setting and whether one was already in force.
pub async fn mute(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
    choice: ChannelMuteChoice,
) -> crate::Result<(ChannelMute, bool)> {
    let now = Utc::now();
    let (muted, until) = match choice {
        ChannelMuteChoice::Muted { duration_seconds } => (true, mute_end(duration_seconds, now)?),
        ChannelMuteChoice::KeptUnmuted => (false, None),
    };
    let mut conn = state.connection_pool.get().await?;
    let ty = crate::permissions::channel_access(state, conn.as_mut(), user, channel_id)
        .await?
        .ty;
    if !matches!(
        ty,
        ChannelType::Text | ChannelType::Dm | ChannelType::GroupDm | ChannelType::Plugin
    ) {
        return Err(crate::Error::Validation(t!("channelMuteKind")));
    }
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
                    channel_mute::muted.eq(muted),
                ))
                .on_conflict((channel_mute::user, channel_mute::channel))
                .do_update()
                .set((channel_mute::until.eq(until), channel_mute::muted.eq(muted)))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(user),
                &ServerEvent::ChannelMuteChanged {
                    channel: channel_id,
                    muted: Some(muted),
                    until,
                },
            )
            .await?;
            let existed = previous.is_some_and(|until| active(until, now));
            Ok::<_, crate::Error>((
                ChannelMute {
                    channel: channel_id,
                    until,
                    muted,
                },
                existed,
            ))
        }
        .scope_boxed()
    })
    .await
}

/// Removes the user's own setting for `channel_id`, if there is one, so it follows its category.
pub async fn unmute(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
) -> crate::Result<()> {
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
                        muted: None,
                        until: None,
                    },
                )
                .await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// The user's own settings in force of the listed channels (DMs they are in).
pub async fn read_channel_mutes(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
) -> crate::Result<Vec<ChannelMute>> {
    read_mutes(state, user, channels, &[]).await
}

/// The user's own settings in force of every DM and group DM they are in, however many there are:
/// rows they made themselves, few, found from their own key, which the DM list sends whole
/// beside its pages so a DM not listed yet is known muted when it is heard from.
pub async fn read_dm_mutes(
    state: &GlobalServerContext,
    user: UserId,
) -> crate::Result<Vec<ChannelMute>> {
    use aspen_schema::dm_recipient;
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now();
    Ok(channel_mute::table
        .select(ChannelMute::as_select())
        .filter(channel_mute::user.eq(user))
        .filter(
            channel_mute::until
                .is_null()
                .or(channel_mute::until.gt(now)),
        )
        .filter(diesel::dsl::exists(
            dm_recipient::table.filter(
                dm_recipient::channel
                    .eq(channel_mute::channel)
                    .and(dm_recipient::user.eq(user)),
            ),
        ))
        .load(conn.as_mut())
        .await?)
}

/// The own settings in force of `visible`'s user of the channels they may view in its
/// communities.
pub async fn read_community_mutes(
    state: &GlobalServerContext,
    visible: &crate::visibility::Visibility,
) -> crate::Result<Vec<ChannelMute>> {
    let mut mutes = read_mutes(state, visible.user(), &[], visible.communities()).await?;
    mutes.retain(|m| visible.can_view(m.channel));
    Ok(mutes)
}

/// The user's own settings in force of the listed channels and of every channel in the listed
/// communities, in one query.
async fn read_mutes(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
    communities: &[CommunityId],
) -> crate::Result<Vec<ChannelMute>> {
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
    use diesel_async::{AsyncConnection, SimpleAsyncConnection};

    #[derive(QueryableByName)]
    struct Muted {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        muted: bool,
    }

    /// `muted_sql!` against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn a_channel_is_muted_by_its_own_setting_else_by_its_category() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        const USER: &str = "01900000-0000-7000-8000-000000000001";
        const COMMUNITY: &str = "01900000-0000-7000-8000-000000000002";
        const CATEGORY: &str = "01900000-0000-7000-8000-000000000003";
        const IN_CATEGORY: &str = "01900000-0000-7000-8000-000000000004";
        const ALONE: &str = "01900000-0000-7000-8000-000000000005";
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                 VALUES ('{USER}', 'u', '', now(), now());
             INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'c');
             INSERT INTO category (id, community, name, sort_index)
                 VALUES ('{CATEGORY}', '{COMMUNITY}', 'k', 0);
             INSERT INTO channel (id, name, ty, sort_index, community, parent_category)
                 VALUES ('{IN_CATEGORY}', 'a', 'text', 0, '{COMMUNITY}', '{CATEGORY}'),
                        ('{ALONE}', 'b', 'text', 1, '{COMMUNITY}', NULL);"#
        ))
        .await
        .unwrap();
        async fn muted(conn: &mut diesel_async::AsyncPgConnection, channel: &str) -> bool {
            diesel::sql_query(concat!("SELECT ", muted_sql!("$1", "$2"), " AS muted"))
                .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::parse_str(USER).unwrap())
                .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::parse_str(channel).unwrap())
                .get_result::<Muted>(conn)
                .await
                .unwrap()
                .muted
        }
        assert!(!muted(&mut conn, IN_CATEGORY).await);
        conn.batch_execute(&format!(
            "INSERT INTO category_mute (\"user\", category, until) VALUES ('{USER}', '{CATEGORY}', NULL)"
        ))
        .await
        .unwrap();
        assert!(muted(&mut conn, IN_CATEGORY).await, "its category's mute");
        assert!(!muted(&mut conn, ALONE).await, "none of another category's");
        conn.batch_execute(&format!(
            "INSERT INTO channel_mute (\"user\", channel, until, muted) VALUES ('{USER}', '{IN_CATEGORY}', NULL, false)"
        ))
        .await
        .unwrap();
        assert!(!muted(&mut conn, IN_CATEGORY).await, "kept unmuted");
        conn.batch_execute(&format!(
            "UPDATE channel_mute SET muted = true, until = now() - interval '1 minute' WHERE channel = '{IN_CATEGORY}'"
        ))
        .await
        .unwrap();
        assert!(
            muted(&mut conn, IN_CATEGORY).await,
            "its own mute has run out, so the category's holds"
        );
        conn.batch_execute(&format!(
            "UPDATE category_mute SET until = now() - interval '1 minute' WHERE category = '{CATEGORY}'"
        ))
        .await
        .unwrap();
        assert!(!muted(&mut conn, IN_CATEGORY).await, "both have run out");
    }

    #[test]
    fn a_mute_lasts_until_its_end_or_for_good() {
        let now = Utc::now();
        assert!(active(None, now));
        assert!(active(Some(now + chrono::Duration::seconds(1)), now));
        assert!(!active(Some(now), now));
        assert!(!active(Some(now - chrono::Duration::seconds(1)), now));
    }
}
