//! Federation over HTTP: this deployment's published document at `/.well-known/aspen`, the
//! Administration Dashboard's gates and directory of other deployments under
//! `/admin/federation`, which take Manage federation, and signing in abroad: assertions for this
//! deployment's users, the sign-in of other deployments' users here, and the avatars other
//! deployments copy (`app::federation`).

use crate::api::admin::AdminUser;
use crate::api::auth::{LoginResponse, SessionUser};
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::double_option;
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::{API_PREFIX, TAG_ADMIN, TAG_AUTH, TAG_ICONS, TAG_USERS};
use crate::app::context::GlobalServerContext;
use crate::app::deployment_settings::SettingsChange;
use crate::app::federation::abroad::{self, ForeignDeployment, Issued};
use crate::app::federation::protocol::{Protocol, Software};
use crate::app::federation::{
    self, ContactOutcome, DeploymentDocument, Direction, Domain, FederationList, Gate, Gates,
    Origin, Subject,
};
use crate::app::{self, IconId, UserId, deployment::DeploymentPermission};
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// How long other deployments may keep this deployment's document before reading it again.
const DOCUMENT_MAX_AGE_SECONDS: u32 = 300;

/// This deployment's document: its domain, key, and gates. `404` when it has no
/// federation domain (an `http` `public_url`). Served outside `/api/v1`, at the path every deployment uses, and
/// read by other deployments rather than clients, so it is not in the OpenAPI document.
pub async fn well_known(State(state): State<GlobalServerContext>) -> ApiResult<Response> {
    match federation::document(&state).await? {
        Some(document) => {
            let mut response = axum::Json(document).into_response();
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_str(&format!("public, max-age={DOCUMENT_MAX_AGE_SECONDS}"))
                    .expect("a valid header value"),
            );
            Ok(response)
        }
        None => Err(ApiError::new(ProblemCode::NotFound)),
    }
}

/// This deployment's part in federation: its domain and key, from `[federation]` in aspen.toml,
/// and its gates, which are deployment settings.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationOverview {
    /// This deployment's domain; `null` when it takes no part in federation.
    pub domain: Option<String>,
    /// The fingerprint of the key it signs with (`SHA256:` and base64), for administrators of
    /// other deployments to compare; `null` before the key is made.
    pub key_fingerprint: Option<String>,
    pub key_created_at: Option<DateTime<Utc>>,
    pub users: Gates,
    pub bots: Gates,
    /// Whether users' two directions read one list.
    pub users_shared_list: bool,
    /// Whether bots' two directions read one list.
    pub bots_shared_list: bool,
    /// Whether a user of another deployment arriving here for the first time needs a
    /// registration invite.
    pub users_immigration_invite_required: bool,
    /// Whether a bot of another deployment arriving here for the first time needs one.
    pub bots_immigration_invite_required: bool,
    /// The lists the gates read, in the order the dashboard shows them.
    pub lists_in_force: Vec<FederationList>,
    /// The document this deployment publishes, as other deployments read it; `null` without a
    /// domain.
    pub document: Option<DeploymentDocument>,
    pub protocol: Protocol,
    pub software: Software,
}

#[utoipa::path(
    get,
    path = "/admin/federation",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = FederationOverview),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_federation(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<FederationOverview>> {
    access.require(DeploymentPermission::ManageFederation)?;
    Ok(Json(overview(&state).await?))
}

async fn overview(state: &GlobalServerContext) -> ApiResult<FederationOverview> {
    let config = &state.config.federation;
    let policy = state.settings().federation;
    let key = federation::current_key(
        state
            .connection_pool
            .get()
            .await
            .map_err(app::Error::from)?
            .as_mut(),
    )
    .await?;
    Ok(FederationOverview {
        domain: federation::own_domain(config).map(String::from),
        key_fingerprint: key.as_ref().map(|k| federation::fingerprint(&k.public_key)),
        key_created_at: key.map(|k| k.created_at),
        users: (&policy.users).into(),
        bots: (&policy.bots).into(),
        users_shared_list: policy.users.shared_list,
        bots_shared_list: policy.bots.shared_list,
        users_immigration_invite_required: policy.users.immigration_invite_required,
        bots_immigration_invite_required: policy.bots.immigration_invite_required,
        lists_in_force: FederationList::all_in_force(&policy),
        document: federation::document(state).await?,
        protocol: Protocol::with_plugins(&state.plugins),
        software: Software::ours(),
    })
}

/// A change to the gates. An absent field is unchanged.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FederationUpdateRequest {
    pub users_emigration: Option<Gate>,
    pub users_immigration: Option<Gate>,
    /// Both of users' gates must then read the same kind of list.
    pub users_shared_list: Option<bool>,
    pub users_immigration_invite_required: Option<bool>,
    pub bots_emigration: Option<Gate>,
    pub bots_immigration: Option<Gate>,
    pub bots_shared_list: Option<bool>,
    pub bots_immigration_invite_required: Option<bool>,
}

impl From<FederationUpdateRequest> for SettingsChange {
    fn from(request: FederationUpdateRequest) -> Self {
        Self {
            users_emigration: request.users_emigration,
            users_immigration: request.users_immigration,
            users_shared_list: request.users_shared_list,
            users_immigration_invite_required: request.users_immigration_invite_required,
            bots_emigration: request.bots_emigration,
            bots_immigration: request.bots_immigration,
            bots_shared_list: request.bots_shared_list,
            bots_immigration_invite_required: request.bots_immigration_invite_required,
            ..Self::default()
        }
    }
}

/// Changes the gates, for every server at once. Users of other deployments whose homes an
/// immigration gate no longer admits are signed out. Takes Manage federation.
#[utoipa::path(
    patch,
    path = "/admin/federation",
    tag = TAG_ADMIN,
    request_body = FederationUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = FederationOverview),
        (status = BAD_REQUEST, description = "A gate opened on a deployment without a domain, or a shared list whose gates read different kinds of list", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_federation(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<FederationUpdateRequest>,
) -> ApiResult<Json<FederationOverview>> {
    app::deployment_settings::update_as(&state, &access, request.into()).await?;
    Ok(Json(overview(&state).await?))
}

/// Who may cross between this deployment and another, as the gates and its lists decide now.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Admission {
    /// This deployment's users may use it.
    pub users_emigration: bool,
    /// Its users may use this deployment.
    pub users_immigration: bool,
    pub bots_emigration: bool,
    pub bots_immigration: bool,
}

/// Another deployment this one knows.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederatedDeployment {
    #[schema(value_type = String)]
    pub domain: Domain,
    pub origin: Origin,
    /// The administrator who added it; `null` when it was added from the terminal, recorded on
    /// first contact, or its adder's account is gone.
    pub added_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub note: Option<String>,
    /// The key pinned when it was first contacted, in unpadded base64url; `null` until then.
    pub public_key: Option<String>,
    pub public_key_fingerprint: Option<String>,
    pub first_contact_at: Option<DateTime<Utc>>,
    pub last_contact_at: Option<DateTime<Utc>>,
    /// A key it presented other than the pinned one, refused until an administrator accepts
    /// it.
    pub offered_key: Option<String>,
    pub offered_key_fingerprint: Option<String>,
    pub offered_key_at: Option<DateTime<Utc>>,
    /// Every list it is on, those not in force included.
    pub lists: Vec<FederationList>,
    pub admission: Admission,
    /// The protocol it said it speaks when last contacted; `null` before any contact.
    pub protocol: Option<Protocol>,
    /// The software it said it runs, for people to read.
    pub software: Option<Software>,
    /// Whether it and this deployment speak a protocol version in common. One contacted before
    /// it said which it speaks speaks the first version.
    pub compatible: bool,
}

impl FederatedDeployment {
    fn new(state: &GlobalServerContext, listed: federation::Listed) -> Self {
        let policy = state.settings().federation;
        let admits =
            |subject, direction| federation::admits(&policy, subject, direction, &listed.lists);
        let admission = Admission {
            users_emigration: admits(Subject::Users, Direction::Emigration),
            users_immigration: admits(Subject::Users, Direction::Immigration),
            bots_emigration: admits(Subject::Bots, Direction::Emigration),
            bots_immigration: admits(Subject::Bots, Direction::Immigration),
        };
        let d = listed.deployment;
        let protocol = d.protocol();
        let compatible = protocol
            .clone()
            .unwrap_or_default()
            .common_version(&Protocol::ours())
            .is_some();
        let software = d.software_name.clone().map(|name| Software {
            name,
            version: d.software_version.clone().unwrap_or_default(),
        });
        let encode = |key: &Option<Vec<u8>>| key.as_ref().map(|k| URL_SAFE_NO_PAD.encode(k));
        let print = |key: &Option<Vec<u8>>| key.as_deref().map(federation::fingerprint);
        FederatedDeployment {
            public_key: encode(&d.public_key),
            public_key_fingerprint: print(&d.public_key),
            offered_key: encode(&d.offered_key),
            offered_key_fingerprint: print(&d.offered_key),
            domain: d.domain,
            origin: d.origin,
            added_by: d.added_by,
            created_at: d.created_at,
            note: d.note,
            first_contact_at: d.first_contact_at,
            last_contact_at: d.last_contact_at,
            offered_key_at: d.offered_key_at,
            lists: listed.lists,
            admission,
            protocol,
            software,
            compatible,
        }
    }
}

/// A page of the directory.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct DeploymentListQuery {
    /// Only those whose domain contains this, ignoring case.
    #[serde(rename = "filter[name]")]
    #[param(rename = "filter[name]")]
    pub name: Option<String>,
    /// How many rows to skip, at most 100,000.
    pub offset: Option<i64>,
    /// How many to return, at most 100; 15 when absent.
    pub limit: Option<i64>,
}

/// How many rows a page of the directory holds when the request does not say.
const DEFAULT_PAGE: i64 = 15;

/// A page of the deployments this one knows, alphabetically by domain.
#[utoipa::path(
    get,
    path = "/admin/federation/deployments",
    tag = TAG_ADMIN,
    params(DeploymentListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<FederatedDeployment>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_deployments(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<DeploymentListQuery>,
) -> ApiResult<Json<Vec<FederatedDeployment>>> {
    access.require(DeploymentPermission::ManageFederation)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let page = federation::list(
        conn.as_mut(),
        query.name.as_deref(),
        query.offset.unwrap_or(0),
        query.limit.unwrap_or(DEFAULT_PAGE),
    )
    .await?;
    Ok(Json(
        page.into_iter()
            .map(|listed| FederatedDeployment::new(&state, listed))
            .collect(),
    ))
}

/// A deployment to add.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FederatedDeploymentCreateRequest {
    /// Its domain, with `:port` when it is not served on 443.
    pub domain: String,
    /// What it is, to remember it by.
    #[serde(default)]
    pub note: Option<String>,
}

/// Adds a deployment to the directory, not yet contacted: contacting it
/// (`POST …/contact`) pins its key.
#[utoipa::path(
    post,
    path = "/admin/federation/deployments",
    tag = TAG_ADMIN,
    request_body = FederatedDeploymentCreateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = FederatedDeployment, headers(("Location" = String, description = "URL of the new entry"))),
        (status = BAD_REQUEST, description = "`validation`: not a domain, this deployment's own, or a note too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = CONFLICT, description = "Already known", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_deployment(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Json(request): Json<FederatedDeploymentCreateRequest>,
) -> ApiResult<Created<FederatedDeployment>> {
    access.require(DeploymentPermission::ManageFederation)?;
    let domain = Domain::parse(&request.domain).map_err(|_| {
        ApiError::new(ProblemCode::Validation).with_detail(crate::t!("federationNotADomain"))
    })?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let listed = federation::add(
        &state.config.federation,
        conn.as_mut(),
        &domain,
        Origin::Administrator,
        Some(session.user.id),
        request.note,
    )
    .await?;
    tracing::info!(%domain, admin = %session.user.id.0, "added a deployment to the federation directory");
    Ok(Created::new(
        format!(
            "{API_PREFIX}/admin/federation/deployments/{}",
            url::form_urlencoded::byte_serialize(domain.as_str().as_bytes()).collect::<String>()
        ),
        FederatedDeployment::new(&state, listed),
    ))
}

#[utoipa::path(
    get,
    path = "/admin/federation/deployments/{domain}",
    tag = TAG_ADMIN,
    params(("domain" = String, Path, description = "Its domain, with `:port` when not 443")),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = FederatedDeployment),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_deployment(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(domain): Path<Domain>,
) -> ApiResult<Json<FederatedDeployment>> {
    access.require(DeploymentPermission::ManageFederation)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let listed = federation::get(conn.as_mut(), &domain).await?;
    Ok(Json(FederatedDeployment::new(&state, listed)))
}

/// A change to a deployment's entry.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FederatedDeploymentUpdateRequest {
    /// `null` clears it.
    #[serde(default, deserialize_with = "double_option")]
    pub note: Option<Option<String>>,
}

#[utoipa::path(
    patch,
    path = "/admin/federation/deployments/{domain}",
    tag = TAG_ADMIN,
    params(("domain" = String, Path)),
    request_body = FederatedDeploymentUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = FederatedDeployment),
        (status = BAD_REQUEST, description = "`validation`: a note too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_deployment(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(domain): Path<Domain>,
    Json(request): Json<FederatedDeploymentUpdateRequest>,
) -> ApiResult<Json<FederatedDeployment>> {
    access.require(DeploymentPermission::ManageFederation)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let listed = match request.note {
        Some(note) => federation::set_note(conn.as_mut(), &domain, note).await?,
        None => federation::get(conn.as_mut(), &domain).await?,
    };
    Ok(Json(FederatedDeployment::new(&state, listed)))
}

/// Forgets a deployment: its pinned key and the lists it is on. If it is contacted again, its
/// key is pinned afresh.
#[utoipa::path(
    delete,
    path = "/admin/federation/deployments/{domain}",
    tag = TAG_ADMIN,
    params(("domain" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Forgotten"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_deployment(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path(domain): Path<Domain>,
) -> ApiResult<NoContent> {
    access.require(DeploymentPermission::ManageFederation)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    federation::remove(conn.as_mut(), &domain).await?;
    tracing::info!(%domain, admin = %session.user.id.0, "forgot a deployment");
    Ok(NoContent)
}

/// What contacting a deployment found, and its entry since.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactResult {
    pub outcome: ContactOutcome,
    pub deployment: FederatedDeployment,
}

/// Reads a known deployment's document now: its key is pinned if none is, confirmed if it is
/// the pinned one, and otherwise refused and kept as its offered key.
#[utoipa::path(
    post,
    path = "/admin/federation/deployments/{domain}/contact",
    tag = TAG_ADMIN,
    params(("domain" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ContactResult),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = BAD_GATEWAY, description = "`deploymentUnreachable`: it did not answer, or not as a deployment does", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn contact_deployment(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path(domain): Path<Domain>,
) -> ApiResult<Json<ContactResult>> {
    access.require(DeploymentPermission::ManageFederation)?;
    // Only known deployments: contacting a stranger would record it.
    federation::get(
        state
            .connection_pool
            .get()
            .await
            .map_err(app::Error::from)?
            .as_mut(),
        &domain,
    )
    .await?;
    let (listed, outcome) = federation::contact(&state, &domain).await?;
    tracing::info!(%domain, ?outcome, admin = %session.user.id.0, "contacted a deployment");
    Ok(Json(ContactResult {
        outcome,
        deployment: FederatedDeployment::new(&state, listed),
    }))
}

/// The key being accepted.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptKeyRequest {
    /// The offered key the administrator was shown, in unpadded base64url; a different key
    /// offered since is not accepted.
    pub public_key: String,
}

/// Accepts the key a deployment offered in place of its pinned one, which is pinned from then
/// on. Only after confirming the change with the deployment's administrators, since a key that
/// changes unannounced may mean someone else answers at its domain.
#[utoipa::path(
    put,
    path = "/admin/federation/deployments/{domain}/key",
    tag = TAG_ADMIN,
    params(("domain" = String, Path)),
    request_body = AcceptKeyRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = FederatedDeployment),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "It offers no key, or a different one", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn accept_key(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path(domain): Path<Domain>,
    Json(request): Json<AcceptKeyRequest>,
) -> ApiResult<Json<FederatedDeployment>> {
    access.require(DeploymentPermission::ManageFederation)?;
    let key = URL_SAFE_NO_PAD
        .decode(request.public_key.trim())
        .map_err(|_| ApiError::new(ProblemCode::BadRequest))?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let listed = federation::accept_key(conn.as_mut(), &domain, &key).await?;
    tracing::warn!(%domain, admin = %session.user.id.0, "accepted a deployment's new key");
    Ok(Json(FederatedDeployment::new(&state, listed)))
}

/// Puts a known deployment on a list. Any list may be edited; only those the gates put in force
/// are read.
#[utoipa::path(
    put,
    path = "/admin/federation/deployments/{domain}/lists/{list}",
    tag = TAG_ADMIN,
    params(("domain" = String, Path), ("list" = FederationList, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Put on the list", body = FederatedDeployment),
        (status = OK, description = "Was on it already", body = FederatedDeployment),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_to_list(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path((domain, list)): Path<(Domain, FederationList)>,
) -> ApiResult<(StatusCode, Json<FederatedDeployment>)> {
    access.require(DeploymentPermission::ManageFederation)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let added =
        federation::set_listed(conn.as_mut(), &domain, list, true, Some(session.user.id)).await?;
    if added {
        tracing::info!(%domain, %list, admin = %session.user.id.0, "put a deployment on a list");
    }
    let listed = federation::get(conn.as_mut(), &domain).await?;
    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(FederatedDeployment::new(&state, listed))))
}

/// Takes a deployment off a list. Taking one off a list it is not on is not an error.
#[utoipa::path(
    delete,
    path = "/admin/federation/deployments/{domain}/lists/{list}",
    tag = TAG_ADMIN,
    params(("domain" = String, Path), ("list" = FederationList, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Off the list"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_from_list(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path((domain, list)): Path<(Domain, FederationList)>,
) -> ApiResult<NoContent> {
    access.require(DeploymentPermission::ManageFederation)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    if federation::set_listed(conn.as_mut(), &domain, list, false, None).await? {
        tracing::info!(%domain, %list, admin = %session.user.id.0, "took a deployment off a list");
    }
    Ok(NoContent)
}

/// Where to sign in abroad.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssertionRequest {
    /// The deployment to sign in at: its domain, with `:port` when it is not served on 443.
    pub audience: String,
}

/// Signs an assertion that the caller is who they are, for one other deployment, where
/// `POST /auth/federated-sign-in` exchanges it for a session within two minutes. How the
/// caller signed in here goes with it, and so does their profile.
#[utoipa::path(
    post,
    path = "/auth/assertions",
    tag = TAG_AUTH,
    request_body = AssertionRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Issued),
        (status = BAD_REQUEST, description = "`validation`: not a domain, or this deployment's own", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`federationRefused`: this deployment takes no part in federation, its emigration gate is closed to that deployment, or the caller's account is another deployment's", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn issue_assertion(
    State(state): State<GlobalServerContext>,
    SessionUser { user, caller }: SessionUser,
    Json(request): Json<AssertionRequest>,
) -> ApiResult<Json<Issued>> {
    let audience = Domain::parse(&request.audience).map_err(|_| {
        ApiError::new(ProblemCode::Validation).with_detail(crate::t!("federationNotADomain"))
    })?;
    Ok(Json(
        abroad::issue(&state, &caller, &user, &audience).await?,
    ))
}

/// An assertion to sign in with.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederatedSignInRequest {
    /// From the home deployment's `POST /auth/assertions`.
    pub assertion: String,
    /// A registration invite, which a first arrival needs where this deployment requires one
    /// of accounts from elsewhere.
    #[serde(default)]
    pub invite_code: Option<String>,
}

/// Signs in a user of another deployment with an assertion from their home, making their
/// account here the first time. The session is an ordinary one; the account's profile and
/// sign-in security are their home's.
#[utoipa::path(
    post,
    path = "/auth/federated-sign-in",
    tag = TAG_AUTH,
    request_body = FederatedSignInRequest,
    responses(
        (status = OK, body = LoginResponse),
        (status = UNAUTHORIZED, description = "`assertionInvalid`", body = Problem),
        (status = FORBIDDEN, description = "`federationRefused`: this deployment's immigration gate is closed to the home, or it takes no part in federation; `strongerSignInRequired`; `registrationInviteRequired` or `registrationInviteInvalid` on a first arrival that needs an invite", body = Problem),
        (status = BAD_GATEWAY, description = "`deploymentUnreachable`: the home could not be reached to read its key", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn federated_sign_in(
    State(state): State<GlobalServerContext>,
    Json(request): Json<FederatedSignInRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let session =
        abroad::sign_in(&state, &request.assertion, request.invite_code.as_deref()).await?;
    Ok(Json(session.into()))
}

/// The other deployments the caller has signed in to from here, the most recently used first,
/// so each of their devices can find them.
#[utoipa::path(
    get,
    path = "/users/@me/foreign-deployments",
    tag = TAG_USERS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<ForeignDeployment>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_foreign_deployments(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<ForeignDeployment>>> {
    Ok(Json(abroad::foreign_deployments(&state, user.id).await?))
}

/// Stops using another deployment: it leaves the caller's list, so their devices stop signing
/// in there. Their account and memberships there are untouched.
#[utoipa::path(
    delete,
    path = "/users/@me/foreign-deployments/{domain}",
    tag = TAG_USERS,
    params(("domain" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Forgotten"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn forget_foreign_deployment(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(domain): Path<Domain>,
) -> ApiResult<NoContent> {
    abroad::forget_foreign_deployment(&state, user.id, &domain).await?;
    Ok(NoContent)
}

/// The avatar of one of this deployment's users, for the other deployments they sign in to,
/// which keep a copy of it. Nothing but those avatars is served here.
#[utoipa::path(
    get,
    path = "/federation/icons/{icon}",
    tag = TAG_ICONS,
    params(("icon" = IconId, Path)),
    responses(
        (status = OK, description = "The image", content_type = "image/*", body = Vec<u8>),
        (status = BAD_REQUEST, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn home_avatar(
    State(state): State<GlobalServerContext>,
    Path(icon): Path<IconId>,
) -> ApiResult<Response> {
    let (bytes, mime_type) = abroad::home_avatar(&state, icon).await?;
    // Served from the web client's own origin, so nothing in it may run there, whatever a
    // browser makes of it.
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_str(&mime_type)
                    .unwrap_or(HeaderValue::from_static("application/octet-stream")),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; sandbox"),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// A notice another deployment sends about one of this deployment's users.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoticeRequest {
    /// A compact JWS of `typ` `aspen-notice+jwt`, as `federation_schema.json` describes.
    pub notice: String,
}

/// Takes a notice from another deployment about one of this deployment's users (`spec/
/// federation.md`). Sent by deployments rather than clients. A notice of a kind this deployment
/// does not know, or about someone who no longer uses the sender, is accepted and ignored.
#[utoipa::path(
    post,
    path = "/federation/notices",
    tag = TAG_AUTH,
    request_body = NoticeRequest,
    responses(
        (status = ACCEPTED, description = "Taken"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, description = "`assertionInvalid`: the notice is malformed, forged, expired, used, or not for this deployment", body = Problem),
        (status = FORBIDDEN, description = "`federationRefused`: this deployment does not federate with the sender", body = Problem),
        (status = BAD_GATEWAY, description = "`deploymentUnreachable`: the sender could not be reached to read its key", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn receive_notice(
    State(state): State<GlobalServerContext>,
    Json(request): Json<NoticeRequest>,
) -> ApiResult<StatusCode> {
    app::federation::notices::receive_notice(&state, &request.notice).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Another deployment asks about this deployment's users signed in there.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StandingRequestBody {
    /// A compact JWS of `typ` `aspen-standing-request+jwt`, as `federation_schema.json`
    /// describes.
    pub request: String,
}

/// This deployment's answer: whether each user is still in good standing here.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StandingAnswerBody {
    /// A compact JWS of `typ` `aspen-standing+jwt`.
    pub standing: String,
}

/// Answers another deployment for this deployment's users signed in there: whether each still
/// exists and may still use it (`spec/federation.md`). Sent by deployments rather than clients.
#[utoipa::path(
    post,
    path = "/federation/standing",
    tag = TAG_AUTH,
    request_body = StandingRequestBody,
    responses(
        (status = OK, body = StandingAnswerBody),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, description = "`assertionInvalid`: the request is malformed, forged, expired, used, or not for this deployment", body = Problem),
        (status = FORBIDDEN, description = "`federationRefused`: this deployment does not federate with the asker", body = Problem),
        (status = BAD_GATEWAY, description = "`deploymentUnreachable`: the asker could not be reached to read its key", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn answer_standing(
    State(state): State<GlobalServerContext>,
    Json(request): Json<StandingRequestBody>,
) -> ApiResult<Json<StandingAnswerBody>> {
    Ok(Json(StandingAnswerBody {
        standing: app::federation::standing::answer(&state, &request.request).await?,
    }))
}
