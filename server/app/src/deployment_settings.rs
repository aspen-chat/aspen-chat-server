//! The deployment's settings: how it presents itself (a display name and icon, which the
//! sign-in screens welcome people with and the system account and authenticators are called
//! by), the policies its administrators set, and its federation gates. They are the one row of
//! `deployment_settings`, which a migration inserts, so every read finds it.
//!
//! Anyone may read the profile, signed in or not; changing it and the policies takes Manage
//! deployment settings, and changing the gates Manage federation ([`update_as`]). The terminal
//! changes any of them ([`update`]).
//!
//! Every API server keeps a copy ([`SettingsCache`]) that requests read, so a setting costs no
//! query. A change bumps `revision` and, before its transaction commits, writes that revision
//! to the NATS key-value bucket `aspen_settings` ([`ring`]); each server watches the bucket
//! ([`spawn_watcher`]) and reads the row again until it holds that revision, which it does as
//! soon as the change commits. A change rolled back never reaches the row, so a server waiting
//! for it gives up after [`CATCH_UP`] and keeps what the row holds.
//!
//! What a change does to what is already open is done once, by whoever made it, after it
//! commits ([`after_change`]): calls are rechecked when file transfers are turned on or off,
//! and the users of other deployments a closing immigration gate no longer admits are signed
//! out. Each server's open event streams follow their copy: a stream whose account owes a
//! second factor the deployment now requires is closed (`api::event_stream`).

use crate::IconId;
use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission};
use crate::events::Publishing;
use crate::federation::{Domain, FederationPolicy, Gate, MigrationRules};
use crate::icon::Icon;
use crate::t;
use aspen_schema::{deployment_settings, icon, user};
use async_nats::jetstream::kv::{Config as KvConfig, Operation, Store};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// The longest a deployment's display name may be, in characters.
pub const DISPLAY_NAME_MAX_CHARS: usize = 64;

/// What the deployment is called where it has not said: by the system account, and to
/// authenticator apps and passkey prompts.
pub const DEFAULT_NAME: &str = "Aspen";

/// The bucket each change's revision is written to.
const BUCKET: &str = "aspen_settings";
const KEY: &str = "revision";

/// How long a server told of a revision keeps reading the row for it before deciding the change
/// rolled back.
const CATCH_UP: Duration = Duration::from_secs(10);
const CATCH_UP_STEP: Duration = Duration::from_millis(20);

/// The deployment's settings as the row holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentSettings {
    /// How many changes the row has seen.
    pub revision: i64,
    /// What the deployment calls itself; `None` when it has not said.
    pub display_name: Option<String>,
    pub icon: Option<IconId>,
    /// The domain this deployment is known by among deployments, pinned the first time a
    /// server starts with an `https` `public_url` ([`pin_domain`]).
    pub federation_domain: Option<Domain>,
    /// Whether creating an account takes a registration invite (`app::registration_invite`).
    pub registration_invite_required: bool,
    /// Whether every person's account must have a second factor (`app::two_factor`).
    pub require_two_factor: bool,
    /// Whether people may make bots. Bots already made keep working either way.
    pub bots_enabled: bool,
    /// The most bots one person may own.
    pub bots_max_per_user: u32,
    /// How many members a community gains before Mention everyone is taken from its everyone
    /// role (`app::everyone_limit`); 0 never.
    pub everyone_mention_limit: u32,
    /// The most custom emoji one community may hold (`app::custom_emoji`).
    pub custom_emoji_limit: u32,
    /// How many GiB one person may upload in any 24 hours (`app::upload_quota`); 0 sets no limit.
    pub upload_quota_gib: u32,
    /// Whether people may offer files to one another in calls. Off, no join token grants
    /// Transfer files, whatever the channel's permissions say.
    pub file_transfers: bool,
    /// Whether registering takes an email address (`app::email`).
    pub email_required: bool,
    /// Whether an account with an unverified email address must verify it before using the
    /// deployment.
    pub email_verification_required: bool,
    /// Whether the deployment has a newsletter its users may subscribe to.
    pub newsletter_enabled: bool,
    pub federation: FederationPolicy,
}

impl DeploymentSettings {
    /// What the deployment is called: its display name, or [`DEFAULT_NAME`].
    pub fn name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(DEFAULT_NAME)
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = deployment_settings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct SettingsRow {
    revision: i64,
    display_name: Option<String>,
    icon: Option<IconId>,
    federation_domain: Option<Domain>,
    registration_invite_required: bool,
    require_two_factor: bool,
    bots_enabled: bool,
    bots_max_per_user: i32,
    everyone_mention_limit: i32,
    custom_emoji_limit: i32,
    upload_quota_gib: i32,
    file_transfers: bool,
    email_required: bool,
    email_verification_required: bool,
    newsletter_enabled: bool,
    users_emigration: Gate,
    users_immigration: Gate,
    users_shared_list: bool,
    users_immigration_invite_required: bool,
    bots_emigration: Gate,
    bots_immigration: Gate,
    bots_shared_list: bool,
    bots_immigration_invite_required: bool,
}

impl From<SettingsRow> for DeploymentSettings {
    fn from(row: SettingsRow) -> Self {
        // The columns' checks keep them from being negative.
        let count = |n: i32| u32::try_from(n).unwrap_or(0);
        Self {
            revision: row.revision,
            display_name: row.display_name,
            icon: row.icon,
            federation_domain: row.federation_domain,
            registration_invite_required: row.registration_invite_required,
            require_two_factor: row.require_two_factor,
            bots_enabled: row.bots_enabled,
            bots_max_per_user: count(row.bots_max_per_user),
            everyone_mention_limit: count(row.everyone_mention_limit),
            custom_emoji_limit: count(row.custom_emoji_limit),
            upload_quota_gib: count(row.upload_quota_gib),
            file_transfers: row.file_transfers,
            email_required: row.email_required,
            email_verification_required: row.email_verification_required,
            newsletter_enabled: row.newsletter_enabled,
            federation: FederationPolicy {
                users: MigrationRules {
                    emigration: row.users_emigration,
                    immigration: row.users_immigration,
                    shared_list: row.users_shared_list,
                    immigration_invite_required: row.users_immigration_invite_required,
                },
                bots: MigrationRules {
                    emigration: row.bots_emigration,
                    immigration: row.bots_immigration,
                    shared_list: row.bots_shared_list,
                    immigration_invite_required: row.bots_immigration_invite_required,
                },
            },
        }
    }
}

/// A change to the settings, with merge-patch semantics: an absent field is unchanged, and
/// `Some(None)` clears a field that may be empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, AsChangeset)]
#[diesel(table_name = deployment_settings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SettingsChange {
    pub display_name: Option<Option<String>>,
    pub icon: Option<Option<IconId>>,
    pub registration_invite_required: Option<bool>,
    pub require_two_factor: Option<bool>,
    pub bots_enabled: Option<bool>,
    // Counts are stored as `INTEGER`; `update` writes them from the settings it checked.
    #[diesel(skip_update)]
    pub bots_max_per_user: Option<u32>,
    #[diesel(skip_update)]
    pub everyone_mention_limit: Option<u32>,
    #[diesel(skip_update)]
    pub custom_emoji_limit: Option<u32>,
    #[diesel(skip_update)]
    pub upload_quota_gib: Option<u32>,
    pub file_transfers: Option<bool>,
    pub email_required: Option<bool>,
    pub email_verification_required: Option<bool>,
    pub newsletter_enabled: Option<bool>,
    pub users_emigration: Option<Gate>,
    pub users_immigration: Option<Gate>,
    pub users_shared_list: Option<bool>,
    pub users_immigration_invite_required: Option<bool>,
    pub bots_emigration: Option<Gate>,
    pub bots_immigration: Option<Gate>,
    pub bots_shared_list: Option<bool>,
    pub bots_immigration_invite_required: Option<bool>,
}

impl SettingsChange {
    /// Whether it changes a federation gate, which takes Manage federation.
    fn touches_federation(&self) -> bool {
        self.users_emigration.is_some()
            || self.users_immigration.is_some()
            || self.users_shared_list.is_some()
            || self.users_immigration_invite_required.is_some()
            || self.bots_emigration.is_some()
            || self.bots_immigration.is_some()
            || self.bots_shared_list.is_some()
            || self.bots_immigration_invite_required.is_some()
    }

    /// Whether it changes anything else, which takes Manage deployment settings.
    fn touches_settings(&self) -> bool {
        let federation_only = SettingsChange {
            users_emigration: self.users_emigration,
            users_immigration: self.users_immigration,
            users_shared_list: self.users_shared_list,
            users_immigration_invite_required: self.users_immigration_invite_required,
            bots_emigration: self.bots_emigration,
            bots_immigration: self.bots_immigration,
            bots_shared_list: self.bots_shared_list,
            bots_immigration_invite_required: self.bots_immigration_invite_required,
            ..SettingsChange::default()
        };
        *self != federation_only
    }

    /// The settings as they would be with this change made, for checking them whole.
    fn applied_to(&self, settings: &DeploymentSettings) -> DeploymentSettings {
        let mut next = settings.clone();
        let users = &mut next.federation.users;
        let bots = &mut next.federation.bots;
        macro_rules! apply {
            ($($field:ident => $target:expr),* $(,)?) => {
                $(if let Some(value) = self.$field.clone() {
                    $target = value;
                })*
            };
        }
        apply!(
            display_name => next.display_name,
            icon => next.icon,
            registration_invite_required => next.registration_invite_required,
            require_two_factor => next.require_two_factor,
            bots_enabled => next.bots_enabled,
            bots_max_per_user => next.bots_max_per_user,
            everyone_mention_limit => next.everyone_mention_limit,
            custom_emoji_limit => next.custom_emoji_limit,
            upload_quota_gib => next.upload_quota_gib,
            file_transfers => next.file_transfers,
            email_required => next.email_required,
            email_verification_required => next.email_verification_required,
            newsletter_enabled => next.newsletter_enabled,
            users_emigration => users.emigration,
            users_immigration => users.immigration,
            users_shared_list => users.shared_list,
            users_immigration_invite_required => users.immigration_invite_required,
            bots_emigration => bots.emigration,
            bots_immigration => bots.immigration,
            bots_shared_list => bots.shared_list,
            bots_immigration_invite_required => bots.immigration_invite_required,
        );
        next
    }
}

/// The settings with their icon, as the profile shows them.
pub struct WithIcon {
    pub settings: DeploymentSettings,
    /// The icon, only once its upload is confirmed.
    pub icon: Option<Icon>,
}

/// Each server's copy of the settings, which requests read instead of the row.
#[derive(Clone)]
pub struct SettingsCache(Arc<watch::Sender<Arc<DeploymentSettings>>>);

impl SettingsCache {
    /// A copy holding `settings`, read as the server starts.
    pub fn new(settings: DeploymentSettings) -> Self {
        Self(Arc::new(watch::Sender::new(Arc::new(settings))))
    }

    /// The settings as this server last read them.
    pub fn current(&self) -> Arc<DeploymentSettings> {
        self.0.borrow().clone()
    }

    /// A receiver told of each change this server reads.
    pub fn subscribe(&self) -> watch::Receiver<Arc<DeploymentSettings>> {
        self.0.subscribe()
    }

    /// Takes `settings` as read from the row, unless this copy already holds a later revision.
    fn offer(&self, settings: DeploymentSettings) {
        self.0.send_if_modified(|held| {
            if settings.revision < held.revision || **held == settings {
                return false;
            }
            *held = Arc::new(settings);
            true
        });
    }
}

/// The settings as the row holds them now.
pub async fn load(conn: &mut AsyncPgConnection) -> crate::Result<DeploymentSettings> {
    Ok(deployment_settings::table
        .select(SettingsRow::as_select())
        .first(conn)
        .await?
        .into())
}

/// The settings with their icon, for the profile.
pub async fn read_with_icon(state: &GlobalServerContext) -> crate::Result<WithIcon> {
    let mut conn = state.connection_pool.get().await?;
    let (row, icon): (SettingsRow, Option<Icon>) = deployment_settings::table
        .left_join(
            icon::table.on(icon::id
                .nullable()
                .eq(deployment_settings::icon)
                .and(icon::ready_at.is_not_null())),
        )
        .select((SettingsRow::as_select(), Option::<Icon>::as_select()))
        .first(conn.as_mut())
        .await?;
    Ok(WithIcon {
        settings: row.into(),
        icon,
    })
}

/// Records `domain` as the one this deployment is known by, the first time a server starts with
/// one, and refuses a domain other than the one recorded: other deployments pin this
/// deployment's key at its domain, so the domain may never change.
pub async fn pin_domain(
    conn: &mut AsyncPgConnection,
    domain: Option<&Domain>,
) -> crate::Result<()> {
    let pinned: Option<Domain> = deployment_settings::table
        .select(deployment_settings::federation_domain)
        .first(conn)
        .await?;
    let refuse = |message: String| Err(crate::Error::Config(config::ConfigError::Message(message)));
    match (pinned, domain) {
        (None, None) => Ok(()),
        (Some(pinned), Some(domain)) if pinned == *domain => Ok(()),
        (None, Some(domain)) => {
            diesel::update(deployment_settings::table)
                .set(deployment_settings::federation_domain.eq(domain))
                .execute(conn)
                .await?;
            tracing::info!(%domain, "pinned this deployment's federation domain");
            Ok(())
        }
        (Some(pinned), Some(domain)) => refuse(format!(
            "aspen.toml's public_url names the federation domain {domain}, but this deployment \
             is known to other deployments as {pinned}, where they pinned its key. Set \
             public_url back to https://{pinned}: a deployment's domain cannot change."
        )),
        (Some(pinned), None) => refuse(format!(
            "aspen.toml's public_url is not https, so it names no federation domain, but this \
             deployment is known to other deployments as {pinned}. Set public_url = \
             \"https://{pinned}\"."
        )),
    }
}

/// Changes the settings for someone with the permissions the change needs: Manage federation
/// for the gates, Manage deployment settings for everything else. A new icon must be one they
/// uploaded.
pub async fn update_as(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    change: SettingsChange,
) -> crate::Result<WithIcon> {
    if change.touches_federation() {
        access.require(DeploymentPermission::ManageFederation)?;
    }
    if change.touches_settings() {
        access.require(DeploymentPermission::ManageDeploymentSettings)?;
    }
    let mut conn = state.connection_pool.get().await?;
    // From the dashboard, a new icon must be one the caller uploaded; the one it has may stay.
    if let Some(Some(icon)) = change.icon {
        let current: Option<IconId> = deployment_settings::table
            .select(deployment_settings::icon)
            .first(conn.as_mut())
            .await?;
        if current != Some(icon) {
            crate::icon::require_own(
                conn.as_mut(),
                access.user,
                icon,
                t!("deploymentIconMissing"),
            )
            .await?;
        }
    }
    let changed = update(state, conn.as_mut(), crate::email::available(state), change).await?;
    state.settings.offer(changed.clone());
    drop(conn);
    read_with_icon(state).await
}

/// Changes the settings, for the dashboard (through [`update_as`]) or the terminal, and answers
/// with them as they now are. A display name is trimmed and from 1 to 64 characters; an icon
/// must be one whose upload is confirmed; a gate opens only on a deployment with a domain, and a
/// shared list needs both of its gates to read the same kind of list; what needs mail turns on
/// only where `email_available`, `[email]` being configured.
pub async fn update(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    email_available: bool,
    mut change: SettingsChange,
) -> crate::Result<DeploymentSettings> {
    if let Some(Some(name)) = &change.display_name {
        change.display_name = Some(Some(validate_display_name(name)?));
    }
    if let Some(Some(id)) = change.icon {
        let ready: Option<IconId> = icon::table
            .select(icon::id)
            .filter(icon::id.eq(id).and(icon::ready_at.is_not_null()))
            .first(conn)
            .await
            .optional()?;
        if ready.is_none() {
            return Err(crate::Error::Validation(t!("deploymentIconMissing")));
        }
    }
    let (before, after) = conn
        .transaction(|conn| {
            async move {
                let before: DeploymentSettings = deployment_settings::table
                    .select(SettingsRow::as_select())
                    .for_update()
                    .first(conn)
                    .await?
                    .into();
                let wanted = change.applied_to(&before);
                validate(&wanted)?;
                let turns_on = |now: bool, then: bool| now && !then;
                if !email_available
                    && (turns_on(wanted.email_required, before.email_required)
                        || turns_on(
                            wanted.email_verification_required,
                            before.email_verification_required,
                        )
                        || turns_on(wanted.newsletter_enabled, before.newsletter_enabled))
                {
                    return Err(crate::Error::Validation(t!("emailSettingNeedsMail")));
                }
                if wanted == before {
                    return Ok((before.clone(), before));
                }
                // `validate` kept each within `INTEGER`.
                let count = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
                let revision: i64 = diesel::update(deployment_settings::table)
                    .set((
                        &change,
                        deployment_settings::bots_max_per_user.eq(count(wanted.bots_max_per_user)),
                        deployment_settings::everyone_mention_limit
                            .eq(count(wanted.everyone_mention_limit)),
                        deployment_settings::custom_emoji_limit
                            .eq(count(wanted.custom_emoji_limit)),
                        deployment_settings::upload_quota_gib.eq(count(wanted.upload_quota_gib)),
                        deployment_settings::revision.eq(deployment_settings::revision + 1),
                    ))
                    .returning(deployment_settings::revision)
                    .get_result(conn)
                    .await?;
                if wanted.name() != before.name() {
                    // The system account belongs to no community, so no event of its profile
                    // reaches anyone; those holding its record see the new name when they next
                    // read it.
                    diesel::update(user::table.filter(user::system))
                        .set(user::display_name.eq(wanted.name()))
                        .execute(conn)
                        .await?;
                }
                ring(state.nats(), revision).await?;
                Ok::<_, crate::Error>((before, DeploymentSettings { revision, ..wanted }))
            }
            .scope_boxed()
        })
        .await?;
    after_change(state, conn, &before, &after).await;
    Ok(after)
}

/// Checks the settings as a change would leave them.
fn validate(settings: &DeploymentSettings) -> crate::Result<()> {
    let largest = i32::MAX.unsigned_abs();
    if [
        settings.bots_max_per_user,
        settings.everyone_mention_limit,
        settings.custom_emoji_limit,
        settings.upload_quota_gib,
    ]
    .into_iter()
    .any(|count| count > largest)
    {
        return Err(crate::Error::Validation(t!(
            "settingCountTooLarge",
            max = largest
        )));
    }
    let policy = &settings.federation;
    if policy.enabled() && settings.federation_domain.is_none() {
        return Err(crate::Error::Validation(t!("federationGateNeedsDomain")));
    }
    for rules in [&policy.users, &policy.bots] {
        if rules.emigration == Gate::Unknown || rules.immigration == Gate::Unknown {
            return Err(crate::Error::Validation(t!("federationGateUnknown")));
        }
        let listed = |gate: Gate| matches!(gate, Gate::AllowList | Gate::BlockList);
        if rules.shared_list && !(listed(rules.emigration) && rules.emigration == rules.immigration)
        {
            return Err(crate::Error::Validation(t!("federationSharedListMismatch")));
        }
    }
    Ok(())
}

/// Brings what is already open in line with a change that has committed: the calls, when file
/// transfers were turned on or off, and the sessions of users from elsewhere, when an
/// immigration gate or the list it reads changed. A failure is logged; the next change, join,
/// or standing check brings them in line.
async fn after_change(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    before: &DeploymentSettings,
    after: &DeploymentSettings,
) {
    if before.file_transfers != after.file_transfers
        && let Err(e) = crate::voice::recheck_in(state, conn, crate::voice::Recheck::Everyone).await
    {
        tracing::error!("could not recheck calls after file transfers changed: {e}");
    }
    let immigration = |policy: &FederationPolicy| {
        [&policy.users, &policy.bots].map(|rules| (rules.immigration, rules.shared_list))
    };
    if immigration(&before.federation) != immigration(&after.federation)
        && let Err(e) = crate::federation::standing::shut_out(state, conn, &after.federation).await
    {
        tracing::error!("could not sign out the users a closed gate no longer admits: {e}");
    }
}

/// Trims a display name and checks its length.
fn validate_display_name(name: &str) -> crate::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > DISPLAY_NAME_MAX_CHARS {
        return Err(crate::Error::Validation(t!(
            "deploymentDisplayNameLength",
            max = DISPLAY_NAME_MAX_CHARS
        )));
    }
    Ok(name.to_string())
}

/// Opens the bucket, creating it on first use.
async fn bucket(context: &async_nats::jetstream::Context) -> crate::Result<Store> {
    if let Ok(store) = context.get_key_value(BUCKET).await {
        return Ok(store);
    }
    context
        .create_key_value(KvConfig {
            bucket: BUCKET.to_string(),
            description: "The revision of the deployment's settings (crate::deployment_settings)"
                .to_string(),
            history: 1,
            ..Default::default()
        })
        .await
        .map_err(crate::Error::from)
}

/// Tells every server that the settings reached `revision`.
async fn ring(context: &async_nats::jetstream::Context, revision: i64) -> crate::Result<()> {
    bucket(context)
        .await?
        .put(KEY, revision.to_string().into())
        .await?;
    Ok(())
}

/// Starts following changes to the settings, for as long as the server runs: each revision
/// written to the bucket is read from the row into this server's copy.
pub fn spawn_watcher(state: GlobalServerContext) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = follow(&state).await {
                tracing::warn!(error = %e, "following the deployment's settings failed; retrying");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn follow(state: &GlobalServerContext) -> crate::Result<()> {
    let store = bucket(&state.nats_context).await?;
    let mut updates = store.watch(KEY).await?;
    // Read once the watch is in place, so no change falls between the two.
    catch_up(state, 0).await?;
    while let Some(update) = updates.next().await {
        let entry = update?;
        if entry.operation != Operation::Put {
            continue;
        }
        let Some(revision) = std::str::from_utf8(&entry.value)
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
        else {
            tracing::error!("ignoring a malformed settings revision");
            continue;
        };
        catch_up(state, revision).await?;
    }
    Ok(())
}

/// Reads the row into this server's copy until it holds `revision`, or until [`CATCH_UP`] has
/// passed, when the change that announced it must have rolled back.
async fn catch_up(state: &GlobalServerContext, revision: i64) -> crate::Result<()> {
    let deadline = tokio::time::Instant::now() + CATCH_UP;
    loop {
        let settings = load(state.connection_pool.get().await?.as_mut()).await?;
        let caught_up = settings.revision >= revision;
        state.settings.offer(settings);
        if caught_up || tokio::time::Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep(CATCH_UP_STEP).await;
    }
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

    #[test]
    fn a_change_says_which_permissions_it_needs() {
        let gates = SettingsChange {
            users_immigration: Some(Gate::Open),
            ..SettingsChange::default()
        };
        assert!(gates.touches_federation() && !gates.touches_settings());
        let policy = SettingsChange {
            require_two_factor: Some(true),
            ..SettingsChange::default()
        };
        assert!(!policy.touches_federation() && policy.touches_settings());
        let both = SettingsChange {
            display_name: Some(None),
            bots_shared_list: Some(false),
            ..SettingsChange::default()
        };
        assert!(both.touches_federation() && both.touches_settings());
    }
}
