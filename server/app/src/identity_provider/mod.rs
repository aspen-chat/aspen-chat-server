//! OpenID Connect providers people sign in through: a company's or a school's identity provider
//! the deployment's administrators chose. Aspen is the relying party only. How it works, and why,
//! is in `docs/architecture/sign-in-and-security/identity-providers.md`.
//!
//! - [`discovery`]: calling providers, and their discovery documents and keys.
//! - [`flow`]: the authorization code flows that sign in, link, and verify again.
//! - [`identity`]: the identities linked to accounts.
//! - [`logout`]: the providers' back-channel logout.

pub mod discovery;
pub mod flow;
pub mod identity;
pub mod logout;

use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission};
use crate::t;
use crate::{IdentityProviderId, aspen_config::AspenConfig};
use aspen_schema::{identity_provider, user_identity};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

/// The most providers a deployment offers at once.
pub const MAX_PROVIDERS: i64 = 20;
/// The longest name a provider's button shows.
pub const MAX_NAME_CHARS: usize = 64;
/// The longest issuer, client id, or client secret kept, matching the table's checks.
const MAX_SETTING_CHARS: usize = 1024;
/// Where providers send the browser back with a code, under `public_url`, outside `/api/v1`.
pub const CALLBACK_PATH: &str = "/auth/oidc/callback";
/// Where providers post logout tokens, under `public_url`, outside `/api/v1`.
pub const BACK_CHANNEL_LOGOUT_PATH: &str = "/auth/oidc/back-channel-logout";
/// How far the dashboard counts a provider's linked accounts; past it, it shows "1000+".
pub const LINKED_COUNT_CAP: i64 = 1000;
/// The links a removed provider's job deletes in one step.
const FORGET_BATCH: i64 = 1000;

/// A provider as the database keeps it, its secret included.
#[derive(Clone, Queryable, Selectable)]
#[diesel(table_name = identity_provider)]
pub struct Provider {
    pub id: IdentityProviderId,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub enabled: bool,
    pub skip_invite: bool,
    pub enforces_two_factor: bool,
    pub trust_email: bool,
    pub created_at: DateTime<Utc>,
    pub removed_at: Option<DateTime<Utc>>,
}

/// A provider as its administrators see it: everything but the secret.
#[derive(Clone, Debug, Serialize)]
pub struct ProviderSettings {
    pub id: IdentityProviderId,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub enabled: bool,
    pub skip_invite: bool,
    pub enforces_two_factor: bool,
    pub trust_email: bool,
    pub created_at: DateTime<Utc>,
    /// The accounts linked to it, counted up to [`LINKED_COUNT_CAP`] + 1.
    pub linked_accounts: i64,
}

impl ProviderSettings {
    fn of(provider: Provider, linked_accounts: i64) -> Self {
        Self {
            id: provider.id,
            name: provider.name,
            issuer: provider.issuer,
            client_id: provider.client_id,
            enabled: provider.enabled,
            skip_invite: provider.skip_invite,
            enforces_two_factor: provider.enforces_two_factor,
            trust_email: provider.trust_email,
            created_at: provider.created_at,
            linked_accounts,
        }
    }
}

/// A provider a sign-in screen offers.
#[derive(Clone, Debug, Serialize)]
pub struct Offered {
    pub id: IdentityProviderId,
    pub name: String,
}

pub struct NewProvider {
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub enabled: bool,
    pub skip_invite: bool,
    pub enforces_two_factor: bool,
    pub trust_email: bool,
}

/// Changes to a provider; `None` leaves a setting as it is.
#[derive(Default)]
pub struct ProviderChanges {
    pub name: Option<String>,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub enabled: Option<bool>,
    pub skip_invite: Option<bool>,
    pub enforces_two_factor: Option<bool>,
    pub trust_email: Option<bool>,
}

fn validate_name(name: &str) -> crate::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(crate::Error::Validation(t!(
            "identityProviderNameLength",
            max = MAX_NAME_CHARS
        )));
    }
    Ok(name.to_string())
}

/// `value` trimmed, or a refusal naming `field`, a label as the reader calls it.
fn validate_setting(value: &str, field: std::borrow::Cow<'static, str>) -> crate::Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_SETTING_CHARS {
        return Err(crate::Error::Validation(t!(
            "identityProviderSettingLength",
            field = field,
            max = MAX_SETTING_CHARS
        )));
    }
    Ok(value.to_string())
}

/// The providers a sign-in screen offers: enabled and not removed, by name.
pub async fn offered(conn: &mut AsyncPgConnection) -> crate::Result<Vec<Offered>> {
    let rows: Vec<(IdentityProviderId, String)> = identity_provider::table
        .select((identity_provider::id, identity_provider::name))
        .filter(identity_provider::enabled)
        .filter(identity_provider::removed_at.is_null())
        .order((identity_provider::name.asc(), identity_provider::id.asc()))
        .limit(MAX_PROVIDERS)
        .load(conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| Offered { id, name })
        .collect())
}

/// A provider that is not removed, whether or not it is enabled.
pub async fn find(
    conn: &mut AsyncPgConnection,
    id: IdentityProviderId,
) -> crate::Result<Option<Provider>> {
    Ok(identity_provider::table
        .select(Provider::as_select())
        .filter(identity_provider::id.eq(id))
        .filter(identity_provider::removed_at.is_null())
        .first(conn)
        .await
        .optional()?)
}

/// A provider that offers sign-in now, or `identityProviderUnavailable`.
pub async fn usable(
    conn: &mut AsyncPgConnection,
    id: IdentityProviderId,
) -> crate::Result<Provider> {
    find(conn, id)
        .await?
        .filter(|provider| provider.enabled)
        .ok_or_else(|| crate::Error::IdentityProviderUnavailable)
}

/// Every provider that is not removed, with how many accounts each has linked, for its
/// administrators.
pub async fn list(
    access: &DeploymentAccess,
    conn: &mut AsyncPgConnection,
) -> crate::Result<Vec<ProviderSettings>> {
    access.require(DeploymentPermission::ManageIdentityProviders)?;
    list_in(conn).await
}

/// [`list`] without a permission check, for the terminal.
pub async fn list_in(conn: &mut AsyncPgConnection) -> crate::Result<Vec<ProviderSettings>> {
    let providers: Vec<Provider> = identity_provider::table
        .select(Provider::as_select())
        .filter(identity_provider::removed_at.is_null())
        .order((identity_provider::name.asc(), identity_provider::id.asc()))
        .limit(MAX_PROVIDERS)
        .load(conn)
        .await?;
    let mut settings = Vec::with_capacity(providers.len());
    for provider in providers {
        let linked = linked_accounts(conn, provider.id).await?;
        settings.push(ProviderSettings::of(provider, linked));
    }
    Ok(settings)
}

/// How many accounts are linked to `provider`, counted no further than [`LINKED_COUNT_CAP`] + 1
/// so the count costs no more than that however many there are. A deployment has at most
/// [`MAX_PROVIDERS`], so counting each in turn stays bounded too.
async fn linked_accounts(
    conn: &mut AsyncPgConnection,
    provider: IdentityProviderId,
) -> crate::Result<i64> {
    let linked: i64 = diesel::sql_query(
        "SELECT count(*) AS count FROM \
         (SELECT 1 FROM user_identity WHERE provider = $1 LIMIT $2) AS linked",
    )
    .bind::<diesel::sql_types::Uuid, _>(provider.0)
    .bind::<diesel::sql_types::BigInt, _>(LINKED_COUNT_CAP + 1)
    .get_result::<Count>(conn)
    .await?
    .count;
    Ok(linked)
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// Adds a provider, once its discovery document names the issuer given.
pub async fn add(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    new: NewProvider,
) -> crate::Result<ProviderSettings> {
    access.require(DeploymentPermission::ManageIdentityProviders)?;
    let mut conn = state.connection_pool.get().await?;
    add_in(&state.config, &mut conn, new).await
}

/// [`add`] without a permission check, for the terminal.
pub async fn add_in(
    config: &AspenConfig,
    conn: &mut AsyncPgConnection,
    new: NewProvider,
) -> crate::Result<ProviderSettings> {
    let name = validate_name(&new.name)?;
    let issuer = validate_setting(&new.issuer, t!("identityProviderIssuer"))?;
    let client_id = validate_setting(&new.client_id, t!("identityProviderClientId"))?;
    let client_secret = validate_setting(&new.client_secret, t!("identityProviderClientSecret"))?;
    // The document is fetched before anything is held, so a slow provider holds no row.
    discovery::document(config, &issuer, true).await?;
    let provider = conn
        .transaction(|conn| {
            async move {
                // Counted with the table locked against other additions, so two at once cannot
                // both pass the cap.
                diesel::sql_query("LOCK TABLE identity_provider IN SHARE ROW EXCLUSIVE MODE")
                    .execute(conn)
                    .await?;
                let held: i64 = identity_provider::table
                    .filter(identity_provider::removed_at.is_null())
                    .count()
                    .get_result(conn)
                    .await?;
                if held >= MAX_PROVIDERS {
                    return Err(crate::Error::Validation(t!(
                        "identityProviderLimit",
                        max = MAX_PROVIDERS
                    )));
                }
                diesel::insert_into(identity_provider::table)
                    .values((
                        identity_provider::id.eq(IdentityProviderId::new()),
                        identity_provider::name.eq(name),
                        identity_provider::issuer.eq(issuer),
                        identity_provider::client_id.eq(client_id),
                        identity_provider::client_secret.eq(client_secret),
                        identity_provider::enabled.eq(new.enabled),
                        identity_provider::skip_invite.eq(new.skip_invite),
                        identity_provider::enforces_two_factor.eq(new.enforces_two_factor),
                        identity_provider::trust_email.eq(new.trust_email),
                    ))
                    .returning(Provider::as_returning())
                    .get_result(conn)
                    .await
                    .map_err(issuer_taken)
            }
            .scope_boxed()
        })
        .await?;
    Ok(ProviderSettings::of(provider, 0))
}

/// The issuer index's unique violation, as the person who added the provider reads it.
fn issuer_taken(error: diesel::result::Error) -> crate::Error {
    match error {
        diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        ) => crate::Error::Conflict(t!("identityProviderIssuerTaken")),
        other => other.into(),
    }
}

/// Changes a provider. A changed issuer, client id, or secret is checked against the provider's
/// discovery document first.
pub async fn update(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: IdentityProviderId,
    changes: ProviderChanges,
) -> crate::Result<ProviderSettings> {
    access.require(DeploymentPermission::ManageIdentityProviders)?;
    let mut conn = state.connection_pool.get().await?;
    update_in(&state.config, &mut conn, id, changes).await
}

/// [`update`] without a permission check, for the terminal.
pub async fn update_in(
    config: &AspenConfig,
    conn: &mut AsyncPgConnection,
    id: IdentityProviderId,
    changes: ProviderChanges,
) -> crate::Result<ProviderSettings> {
    let name = changes.name.as_deref().map(validate_name).transpose()?;
    let issuer = changes
        .issuer
        .as_deref()
        .map(|issuer| validate_setting(issuer, t!("identityProviderIssuer")))
        .transpose()?;
    let client_id = changes
        .client_id
        .as_deref()
        .map(|client_id| validate_setting(client_id, t!("identityProviderClientId")))
        .transpose()?;
    let client_secret = changes
        .client_secret
        .as_deref()
        .map(|secret| validate_setting(secret, t!("identityProviderClientSecret")))
        .transpose()?;
    if let Some(issuer) = &issuer {
        discovery::document(config, issuer, true).await?;
    }
    let target = identity_provider::table
        .filter(identity_provider::id.eq(id))
        .filter(identity_provider::removed_at.is_null());
    let provider: Provider = diesel::update(target)
        .set((
            name.map(|v| identity_provider::name.eq(v)),
            issuer.map(|v| identity_provider::issuer.eq(v)),
            client_id.map(|v| identity_provider::client_id.eq(v)),
            client_secret.map(|v| identity_provider::client_secret.eq(v)),
            changes.enabled.map(|v| identity_provider::enabled.eq(v)),
            changes
                .skip_invite
                .map(|v| identity_provider::skip_invite.eq(v)),
            changes
                .enforces_two_factor
                .map(|v| identity_provider::enforces_two_factor.eq(v)),
            changes
                .trust_email
                .map(|v| identity_provider::trust_email.eq(v)),
        ))
        .returning(Provider::as_returning())
        .get_result(conn)
        .await
        .map_err(issuer_taken)?;
    let linked = linked_accounts(conn, provider.id).await?;
    Ok(ProviderSettings::of(provider, linked))
}

/// Removes a provider: it offers sign-in no more, and its links are deleted by a job
/// (`forgetIdentityProvider`) a batch at a time. Its row stays, so the sign-ins it made keep
/// naming it; they last until they end as any other does.
pub async fn remove(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: IdentityProviderId,
) -> crate::Result<()> {
    access.require(DeploymentPermission::ManageIdentityProviders)?;
    let mut conn = state.connection_pool.get().await?;
    remove_in(&mut conn, id).await?;
    crate::jobs::wake(state).await;
    Ok(())
}

/// [`remove`] without a permission check, for the terminal, which wakes the job runners itself.
pub async fn remove_in(conn: &mut AsyncPgConnection, id: IdentityProviderId) -> crate::Result<()> {
    conn.transaction(|conn| {
        async move {
            let removed = diesel::update(
                identity_provider::table
                    .filter(identity_provider::id.eq(id))
                    .filter(identity_provider::removed_at.is_null()),
            )
            .set((
                identity_provider::removed_at.eq(diesel::dsl::now),
                identity_provider::enabled.eq(false),
            ))
            .execute(conn)
            .await?;
            if removed == 0 {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            crate::jobs::enqueue(
                conn,
                crate::jobs::NewJob::new(
                    crate::jobs::JobKind::ForgetIdentityProvider,
                    crate::jobs::JobClass::Normal,
                    &ToForget { provider: id },
                )?
                .keyed(id.0.to_string()),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// What forgetting a removed provider is given (`jobs::JobKind::ForgetIdentityProvider`).
#[derive(Serialize, Deserialize)]
struct ToForget {
    provider: IdentityProviderId,
}

/// One step of forgetting a removed provider: a batch of its links. Nothing is announced; the
/// accounts' security settings read their links when they open.
pub async fn forget_step(
    state: &GlobalServerContext,
    job: &crate::jobs::Claimed,
) -> crate::Result<crate::jobs::Outcome> {
    let ToForget { provider } = job.payload()?;
    let mut conn = state.connection_pool.get().await?;
    let taken = diesel::sql_query(
        "DELETE FROM user_identity WHERE ctid IN \
         (SELECT ctid FROM user_identity WHERE provider = $1 LIMIT $2)",
    )
    .bind::<diesel::sql_types::Uuid, _>(provider.0)
    .bind::<diesel::sql_types::BigInt, _>(FORGET_BATCH)
    .execute(conn.as_mut())
    .await?;
    if taken as i64 >= FORGET_BATCH {
        return Ok(crate::jobs::Outcome::Progress(serde_json::Value::Null));
    }
    Ok(crate::jobs::Outcome::Done)
}

/// Whether `user` still has a way in: a password, a passkey, or an identity at a provider that
/// is not removed. Called inside the transaction that takes one away, after taking it, so the
/// answer counts what is left; it holds the account's row first, so two removals at once are
/// checked one after the other and cannot each leave the other's as the last.
pub async fn has_way_in(conn: &mut AsyncPgConnection, user: crate::UserId) -> crate::Result<bool> {
    use aspen_schema::{passkey, user as account};
    let password: String = account::table
        .select(account::password_hash)
        .filter(account::id.eq(user))
        .for_no_key_update()
        .first(conn)
        .await?;
    if !password.is_empty() {
        return Ok(true);
    }
    let passkey = diesel::select(diesel::dsl::exists(
        passkey::table.filter(passkey::user.eq(user)),
    ))
    .get_result::<bool>(conn)
    .await?;
    if passkey {
        return Ok(true);
    }
    Ok(diesel::select(diesel::dsl::exists(
        user_identity::table
            .inner_join(identity_provider::table)
            .filter(user_identity::user.eq(user))
            .filter(identity_provider::removed_at.is_null()),
    ))
    .get_result::<bool>(conn)
    .await?)
}
