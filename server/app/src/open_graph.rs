//! What a link to the web client previews as where it is shared (`api::web_client` writes it
//! into the page as Open Graph tags). Every page previews as the deployment, its display name
//! and icon, except an invite of this deployment's that still works, which previews as its
//! community's name and icon: what anyone holding the link sees once they sign in, shown to
//! them, and to the services that unfurl links, before they do.

use crate::context::GlobalServerContext;
use crate::deployment_settings;
use crate::icon::Icon;
use aspen_schema::{community, icon, invite};
use chrono::Utc;
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, NullableExpressionMethods,
    OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel_async::RunQueryDsl;

/// Something a page can preview as: a deployment or a community.
pub struct Subject {
    /// The name it goes by; `None` for a deployment that has not given one.
    pub name: Option<String>,
    /// Its icon, once its upload is confirmed.
    pub icon: Option<Icon>,
}

/// What a page previews as.
pub struct Preview {
    pub deployment: Subject,
    /// The community an invite leads to, for an invite page.
    pub community: Option<Subject>,
}

/// The preview of every page but an invite's.
pub async fn of_deployment(state: &GlobalServerContext) -> crate::Result<Preview> {
    let read = deployment_settings::read_with_icon(state).await?;
    Ok(Preview {
        deployment: Subject {
            name: read.settings.display_name,
            icon: read.icon,
        },
        community: None,
    })
}

/// The preview of every page but an invite's from what this server already holds: the
/// deployment's name, without its icon, which takes a read.
pub fn by_name(state: &GlobalServerContext) -> Preview {
    Preview {
        deployment: Subject {
            name: state.settings().display_name.clone(),
            icon: None,
        },
        community: None,
    }
}

/// The preview of the page of the invite `code`: its community, while the invite is neither
/// revoked nor expired and the community is not deleted, and otherwise the deployment, so a
/// link that no longer works previews as no community at all.
pub async fn of_invite(state: &GlobalServerContext, code: &str) -> crate::Result<Preview> {
    let mut preview = of_deployment(state).await?;
    let mut conn = state.connection_pool.get().await?;
    let found: Option<(String, Option<Icon>)> = invite::table
        .inner_join(community::table)
        .left_join(
            icon::table.on(icon::id
                .nullable()
                .eq(community::icon)
                .and(icon::ready_at.is_not_null())),
        )
        .filter(
            invite::code
                .eq(code)
                .and(invite::deleted_at.is_null())
                .and(
                    invite::expires_at
                        .is_null()
                        .or(invite::expires_at.gt(Utc::now())),
                )
                .and(community::deleted_at.is_null()),
        )
        .select((community::name, Option::<Icon>::as_select()))
        .first(conn.as_mut())
        .await
        .optional()?;
    preview.community = found.map(|(name, icon)| Subject {
        name: Some(name),
        icon,
    });
    Ok(preview)
}
