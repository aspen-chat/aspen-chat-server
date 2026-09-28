//! Federation over HTTP: this deployment's published document at `/.well-known/aspen`, and
//! the Administration Dashboard's directory of other deployments under `/admin/federation`,
//! which takes Manage federation (`app::federation`).

use crate::api::admin::AdminUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_ADMIN, double_option};
use crate::app::federation::{
    self, ContactOutcome, DeploymentDocument, Direction, Domain, FederationList, Gates, Origin,
    Subject,
};
use crate::app::{self, UserId, deployment::DeploymentPermission};
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
/// `[federation] domain`. Served outside `/api/v1`, at the path every deployment uses, and
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

/// This deployment's part in federation, as `[federation]` in aspen.toml sets it.
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
    /// The lists the gates read, in the order the dashboard shows them.
    pub lists_in_force: Vec<FederationList>,
    /// The document this deployment publishes, as other deployments read it; `null` without a
    /// domain.
    pub document: Option<DeploymentDocument>,
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
    let config = &state.config.federation;
    let key = federation::current_key(
        state
            .connection_pool
            .get()
            .await
            .map_err(app::Error::from)?
            .as_mut(),
    )
    .await?;
    Ok(Json(FederationOverview {
        domain: federation::own_domain(config).map(String::from),
        key_fingerprint: key.as_ref().map(|k| federation::fingerprint(&k.public_key)),
        key_created_at: key.map(|k| k.created_at),
        users: (&config.users).into(),
        bots: (&config.bots).into(),
        users_shared_list: config.users.shared_list,
        bots_shared_list: config.bots.shared_list,
        lists_in_force: FederationList::all_in_force(config),
        document: federation::document(&state).await?,
    }))
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
}

impl FederatedDeployment {
    fn new(state: &GlobalServerContext, listed: federation::Listed) -> Self {
        let config = &state.config.federation;
        let admits =
            |subject, direction| federation::admits(config, subject, direction, &listed.lists);
        let admission = Admission {
            users_emigration: admits(Subject::Users, Direction::Emigration),
            users_immigration: admits(Subject::Users, Direction::Immigration),
            bots_emigration: admits(Subject::Bots, Direction::Emigration),
            bots_immigration: admits(Subject::Bots, Direction::Immigration),
        };
        let d = listed.deployment;
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
        ApiError::new(ProblemCode::Validation).with_detail(rust_i18n::t!("federationNotADomain"))
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

/// Puts a known deployment on a list. Any list may be edited; only those `[federation]` puts in
/// force are read.
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
