//! Categories a user has muted, for themself alone, for a while or until they lift it. Every
//! channel in a muted category is muted for them, those moved into it later too, unless the
//! channel's own setting says otherwise (`app::channel_mute`): its own mute, or kept unmuted.
//! Only a category the user may learn of can be muted. Every change is published to the user's
//! own subject as `categoryMuteChanged`, so their other devices follow; a mute that runs out
//! ends on each device by its own clock, with no event, as a channel's does.

use crate::channel_mute::{active, mute_end};
use crate::context::GlobalServerContext;
use crate::visibility::Visibility;
use crate::{CategoryId, CommunityId, EventScope, UserId, publish_event};
use aspen_schema::{category, category_mute};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

/// A category the user has muted, and until when; `None` until they unmute it.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = category_mute)]
pub struct CategoryMute {
    pub category: CategoryId,
    pub until: Option<DateTime<Utc>>,
}

/// Mutes `category_id` for the user, for `duration_seconds` or, with `None`, until they unmute
/// it, replacing any mute already on it. Returns the mute and whether one was already in force.
pub async fn mute(
    state: &GlobalServerContext,
    user: UserId,
    category_id: CategoryId,
    duration_seconds: Option<u32>,
) -> crate::Result<(CategoryMute, bool)> {
    let now = Utc::now();
    let until = mute_end(duration_seconds, now)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            crate::category::viewed_category(conn.as_mut(), user, category_id).await?;
            let previous: Option<Option<DateTime<Utc>>> = category_mute::table
                .select(category_mute::until)
                .filter(
                    category_mute::user
                        .eq(user)
                        .and(category_mute::category.eq(category_id)),
                )
                .for_update()
                .first(conn.as_mut())
                .await
                .optional()?;
            diesel::insert_into(category_mute::table)
                .values((
                    category_mute::user.eq(user),
                    category_mute::category.eq(category_id),
                    category_mute::until.eq(until),
                ))
                .on_conflict((category_mute::user, category_mute::category))
                .do_update()
                .set(category_mute::until.eq(until))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(user),
                &ServerEvent::CategoryMuteChanged {
                    category: category_id,
                    muted: true,
                    until,
                },
            )
            .await?;
            let existed = previous.is_some_and(|until| active(until, now));
            Ok::<_, crate::Error>((
                CategoryMute {
                    category: category_id,
                    until,
                },
                existed,
            ))
        }
        .scope_boxed()
    })
    .await
}

/// Lifts the user's mute of `category_id`, if there is one.
pub async fn unmute(
    state: &GlobalServerContext,
    user: UserId,
    category_id: CategoryId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                category_mute::table.filter(
                    category_mute::user
                        .eq(user)
                        .and(category_mute::category.eq(category_id)),
                ),
            )
            .execute(conn.as_mut())
            .await?;
            if removed > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(user),
                    &ServerEvent::CategoryMuteChanged {
                        category: category_id,
                        muted: false,
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

/// The mutes in force of `visible`'s user of the categories of its communities they may still
/// learn of (`Visibility::can_view_category`).
pub async fn read_category_mutes(
    state: &GlobalServerContext,
    visible: &Visibility,
) -> crate::Result<Vec<CategoryMute>> {
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now();
    let rows: Vec<(CategoryMute, CommunityId)> = category_mute::table
        .inner_join(category::table)
        .select((CategoryMute::as_select(), category::community))
        .filter(category_mute::user.eq(visible.user()))
        .filter(
            category_mute::until
                .is_null()
                .or(category_mute::until.gt(now)),
        )
        .filter(category::community.eq_any(visible.communities().to_vec()))
        .filter(category::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    Ok(rows
        .into_iter()
        .filter(|(mute, community)| visible.can_view_category(*community, mute.category))
        .map(|(mute, _)| mute)
        .collect())
}
