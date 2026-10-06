//! Categories a user has collapsed in their channel list, for themself alone. Each change is
//! published to the user's own subject as `categoryCollapseChanged`, so their other devices
//! fold or unfold the same category.

use crate::api::message_enum::server_event::ServerEvent;
use crate::app::context::GlobalServerContext;
use crate::app::visibility::Visibility;
use crate::app::{self, CategoryId, CommunityId, EventScope, UserId, publish_event};
use crate::database::schema::{category, category_collapse};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

/// Collapses `category_id` for the user. Returns whether it was collapsed already.
pub async fn collapse(
    state: &GlobalServerContext,
    user: UserId,
    category_id: CategoryId,
) -> app::Result<bool> {
    set(state, user, category_id, true).await
}

/// Expands `category_id` for the user.
pub async fn expand(
    state: &GlobalServerContext,
    user: UserId,
    category_id: CategoryId,
) -> app::Result<()> {
    set(state, user, category_id, false).await.map(|_| ())
}

/// Sets whether the user has `category_id` collapsed, and returns whether they had before.
async fn set(
    state: &GlobalServerContext,
    user: UserId,
    category_id: CategoryId,
    collapsed: bool,
) -> app::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // Only a category the user may learn of; any other is not found.
            app::category::viewed_category(conn.as_mut(), user, category_id).await?;
            let changed = if collapsed {
                diesel::insert_into(category_collapse::table)
                    .values((
                        category_collapse::user.eq(user),
                        category_collapse::category.eq(category_id),
                    ))
                    .on_conflict_do_nothing()
                    .execute(conn.as_mut())
                    .await?
            } else {
                diesel::delete(
                    category_collapse::table.filter(
                        category_collapse::user
                            .eq(user)
                            .and(category_collapse::category.eq(category_id)),
                    ),
                )
                .execute(conn.as_mut())
                .await?
            };
            if changed > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(user),
                    &ServerEvent::CategoryCollapseChanged {
                        category: category_id,
                        collapsed,
                    },
                )
                .await?;
            }
            // Whether it was collapsed before: collapsing changed nothing if it was, and
            // expanding changed something only if it was.
            Ok::<_, app::Error>((changed > 0) != collapsed)
        }
        .scope_boxed()
    })
    .await
}

/// The categories of the communities `visible` covers that its user has collapsed and may
/// still learn of (`Visibility::can_view_category`).
pub async fn read_collapsed(
    state: &GlobalServerContext,
    visible: &Visibility,
) -> app::Result<Vec<CategoryId>> {
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<(CategoryId, CommunityId)> = category_collapse::table
        .inner_join(category::table)
        .select((category_collapse::category, category::community))
        .filter(category_collapse::user.eq(visible.user()))
        .filter(category::community.eq_any(visible.communities().to_vec()))
        .filter(category::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    Ok(rows
        .into_iter()
        .filter(|(category, community)| visible.can_view_category(*community, *category))
        .map(|(category, _)| category)
        .collect())
}
