//! How the deployment presents itself to people: the display name and icon its sign-in screen
//! welcomes them with. Anyone may read them, signed in or not, since the sign-in screen shows
//! them before anyone has an account; changing them takes Manage federation, under which the
//! deployment's identity among deployments already sits.
//!
//! They are the one row of `deployment_profile`, which the migration that makes the table also
//! inserts, so every read finds it.

use crate::app;
use crate::app::IconId;
use crate::app::context::GlobalServerContext;
use crate::app::deployment::{DeploymentAccess, DeploymentPermission};
use crate::app::icon::Icon;
use crate::database::schema::{deployment_profile, icon};
use crate::t;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

/// The longest a deployment's display name may be, in characters.
pub const DISPLAY_NAME_MAX_CHARS: usize = 64;

/// How the deployment presents itself. Either part is `None` when it has not been set.
#[derive(Debug, Clone)]
pub struct DeploymentProfile {
    pub display_name: Option<String>,
    /// The icon, only once its upload is confirmed.
    pub icon: Option<Icon>,
}

/// A change to the profile, with merge-patch semantics: an absent field is unchanged, and
/// `Some(None)` clears it.
#[derive(Debug, Clone, Default, AsChangeset)]
#[diesel(table_name = deployment_profile)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeploymentProfileChange {
    pub display_name: Option<Option<String>>,
    pub icon: Option<Option<IconId>>,
}

/// The deployment's profile.
pub async fn read_profile(state: &GlobalServerContext) -> app::Result<DeploymentProfile> {
    load(state.connection_pool.get().await?.as_mut()).await
}

/// Changes the deployment's profile, for someone with Manage federation, and answers with the
/// profile as it now is. A display name is trimmed and may not be empty; an icon must be one
/// whose upload is confirmed.
pub async fn update_profile(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    mut change: DeploymentProfileChange,
) -> app::Result<DeploymentProfile> {
    access.require(DeploymentPermission::ManageFederation)?;
    if let Some(Some(name)) = &change.display_name {
        change.display_name = Some(Some(validate_display_name(name)?));
    }
    let mut conn = state.connection_pool.get().await?;
    if let Some(Some(id)) = change.icon {
        let ready: Option<IconId> = icon::table
            .select(icon::id)
            .filter(icon::id.eq(id).and(icon::ready_at.is_not_null()))
            .first(conn.as_mut())
            .await
            .optional()?;
        if ready.is_none() {
            return Err(app::Error::Validation(t!("deploymentIconMissing")));
        }
    }
    // A change that names no field would be an empty `UPDATE`, which Diesel refuses.
    if change.display_name.is_some() || change.icon.is_some() {
        diesel::update(deployment_profile::table)
            .set(&change)
            .execute(conn.as_mut())
            .await?;
    }
    load(conn.as_mut()).await
}

/// Trims a display name and checks its length.
fn validate_display_name(name: &str) -> app::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > DISPLAY_NAME_MAX_CHARS {
        return Err(app::Error::Validation(t!(
            "deploymentDisplayNameLength",
            max = DISPLAY_NAME_MAX_CHARS
        )));
    }
    Ok(name.to_string())
}

async fn load(mut conn: &AsyncPgConnection) -> app::Result<DeploymentProfile> {
    let (display_name, icon): (Option<String>, Option<Icon>) = deployment_profile::table
        .left_join(
            icon::table.on(icon::id
                .nullable()
                .eq(deployment_profile::icon)
                .and(icon::ready_at.is_not_null())),
        )
        .select((
            deployment_profile::display_name,
            Option::<Icon>::as_select(),
        ))
        .first(&mut conn)
        .await?;
    Ok(DeploymentProfile { display_name, icon })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_are_trimmed() {
        assert_eq!(
            validate_display_name("  Aspen Town ").unwrap(),
            "Aspen Town"
        );
    }

    #[test]
    fn display_names_are_counted_in_characters_of_any_script() {
        let name = "ж".repeat(DISPLAY_NAME_MAX_CHARS);
        assert_eq!(validate_display_name(&name).unwrap(), name);
        assert!(validate_display_name(&format!("{name}ж")).is_err());
    }

    #[test]
    fn blank_display_names_are_refused() {
        assert!(validate_display_name("   ").is_err());
    }
}
