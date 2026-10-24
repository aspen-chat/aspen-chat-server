//! Signing in one device from another (`app::device_link`): one device shows a QR code, and a
//! user code beside it, and the other scans the one or types the other, whichever of the two is
//! signed in.

use crate::auth::{EnrollingSessionUser, LoginResponse, SessionUser};
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::{Created, Json, NoContent, Path};
use crate::icon::{Icon, icon_to_api};
use crate::rate_limit::Failures;
use crate::{API_PREFIX, TAG_AUTH};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::device_link::{self, Claim, Progress};
use aspen_app::two_factor::Caller;
use aspen_app::{Loadable, UserId};
use axum::Extension;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which side started a device link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum DeviceLinkKind {
    /// Started by a device that is not signed in; a signed-in phone scans its code.
    Request,
    /// Started by a signed-in device; a phone that is not signed in scans its code.
    Offer,
}

impl From<device_link::Kind> for DeviceLinkKind {
    fn from(kind: device_link::Kind) -> Self {
        match kind {
            device_link::Kind::Request => Self::Request,
            device_link::Kind::Offer => Self::Offer,
        }
    }
}

/// Starting a device link. Signed in, it offers the caller's account and needs neither field;
/// signed out, it asks for a sign-in and needs both.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkRequest {
    /// What the device calls itself ("Firefox on Linux"), which the signed-in device shows
    /// before confirming it; 1 to 64 characters.
    #[serde(default)]
    pub device_name: Option<String>,
    /// The SHA-256 of a secret verifier only this device holds, unpadded base64url (PKCE's
    /// `S256`); the sign-in is claimed with the verifier.
    #[serde(default)]
    pub code_challenge: Option<String>,
    /// Whether to make a user code too (`code`), for the other device to type where it cannot
    /// scan the QR code. Starting a link, a code also lets it last five minutes rather than one.
    /// Scanning, it is ignored.
    #[serde(default)]
    pub with_code: bool,
}

/// A device link just started.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLink {
    /// Secret: the QR code carries it, and whoever holds it can scan the link.
    pub id: String,
    pub kind: DeviceLinkKind,
    /// When the code stops working unless it is scanned.
    pub expires_at: DateTime<Utc>,
    /// The user code, when `withCode` asked for one: eight letters in two groups (`BCDF-GHJK`),
    /// shown beside the QR code for the other device to type at
    /// `POST /auth/device-link-entries` instead. Secret, as the id is, until `expiresAt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(required = false)]
    pub code: Option<String>,
}

/// Refuses a session that may not give its account away while it owes the server a second
/// factor, as `SessionUser` refuses it everything else.
fn giving_caller(
    state: &GlobalServerContext,
    session: Option<&EnrollingSessionUser>,
) -> ApiResult<Option<Caller>> {
    match session {
        Some(EnrollingSessionUser(SessionUser { caller, .. })) => {
            if caller.enrollment_required(&state.settings()) {
                return Err(ApiError::new(ProblemCode::TwoFactorEnrollmentRequired));
            }
            Ok(Some(caller.clone()))
        }
        None => Ok(None),
    }
}

/// Starts a device link and returns the id its QR code carries, and with `withCode` a user code
/// to type instead. Signed in, it is an `offer` of the caller's account, for a device that is
/// not signed in; signed out, a `request`, for a signed-in device to grant. The code lasts a
/// minute unless scanned, or five with a user code.
#[utoipa::path(
    post,
    path = "/auth/device-links",
    tag = TAG_AUTH,
    request_body = DeviceLinkRequest,
    security((), ("bearerAuth" = [])),
    responses(
        (status = CREATED, body = DeviceLink, headers(("Location" = String, description = "URL of the link"))),
        (status = BAD_REQUEST, description = "`validation`: a request without a valid device name or code challenge", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a bot, or a user of another deployment; `twoFactorEnrollmentRequired`; `reauthenticationRequired`: a signed-in giver verifies again first", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`serverBusy`: no free user code was found; try again", body = Problem),
    )
)]
pub async fn start_device_link(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    Json(request): Json<DeviceLinkRequest>,
) -> ApiResult<Created<DeviceLink>> {
    let caller = giving_caller(&state, session.as_ref())?;
    let started = device_link::start(
        &state,
        caller.as_ref(),
        request.device_name,
        request.code_challenge,
        request.with_code,
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/auth/device-links/{}", started.id),
        DeviceLink {
            id: started.id,
            kind: started.kind.into(),
            expires_at: started.expires_at,
            code: started.code,
        },
    ))
}

/// What scanning a device link learns.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkScan {
    pub kind: DeviceLinkKind,
    /// For a `request`: the name the device asking to sign in gave itself, to confirm.
    pub device_name: Option<String>,
    /// For an `offer`: the account this device will be signed in to, once the other device
    /// confirms it.
    pub user_name: Option<String>,
    /// For an `offer`: that account's display name, if it has one.
    pub display_name: Option<String>,
}

/// Scans a device link. A `request` is scanned by the signed-in phone that will grant it, which
/// confirms next (`PUT …/approval`); an `offer` by the phone that will sign in, giving its name
/// and code challenge as when starting one, which then claims (`POST …/claim`) while the other
/// device confirms. A code scans once; a second scan is `deviceLinkUsed`.
#[utoipa::path(
    post,
    path = "/auth/device-links/{link}/scan",
    tag = TAG_AUTH,
    params(("link" = String, Path, description = "The link's id, from its QR code")),
    request_body = DeviceLinkRequest,
    security((), ("bearerAuth" = [])),
    responses(
        (status = OK, body = DeviceLinkScan),
        (status = BAD_REQUEST, description = "`validation`: an offer scanned without a valid device name or code challenge, or by a device already signed in", body = Problem),
        (status = UNAUTHORIZED, description = "`unauthorized`: a request scanned without a session", body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a bot, or a user of another deployment; `twoFactorEnrollmentRequired`; `reauthenticationRequired`: a signed-in giver verifies again first", body = Problem),
        (status = NOT_FOUND, description = "`deviceLinkExpired`", body = Problem),
        (status = CONFLICT, description = "`deviceLinkUsed`: another device scanned it first", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn scan_device_link(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    Path(link): Path<String>,
    Json(request): Json<DeviceLinkRequest>,
) -> ApiResult<Json<DeviceLinkScan>> {
    let caller = giving_caller(&state, session.as_ref())?;
    let scanned = device_link::scan(
        &state,
        &link,
        caller.as_ref(),
        request.device_name,
        request.code_challenge,
    )
    .await?;
    Ok(Json(DeviceLinkScan::new(&state, scanned).await?))
}

impl DeviceLinkScan {
    async fn new(state: &GlobalServerContext, scanned: device_link::Scanned) -> ApiResult<Self> {
        let user = match scanned.user {
            Some(user) => Some(app::user::User::load_from_db(state, user).await?.user_pg),
            None => None,
        };
        Ok(Self {
            kind: scanned.kind.into(),
            device_name: scanned.device_name,
            user_name: user.as_ref().map(|u| u.name.clone()),
            display_name: user.and_then(|u| u.display_name),
        })
    }
}

/// Typing a device link's user code.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkEntryRequest {
    /// The user code the other device shows, in any case, with or without its hyphen and
    /// spaces.
    pub code: String,
    /// Signed out: what this device calls itself, as when starting a link.
    #[serde(default)]
    pub device_name: Option<String>,
    /// Signed out: the SHA-256 of this device's verifier, as when starting a link.
    #[serde(default)]
    pub code_challenge: Option<String>,
}

/// A device link taken by typing its user code.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkEntry {
    /// The link's id, which every later step on it names, as a scanned QR code's does. Secret.
    pub id: String,
    #[serde(flatten)]
    pub scan: DeviceLinkScan,
}

/// Takes a device link by typing the user code another device shows, in place of scanning its
/// QR code, and goes on as a scan does. Signed in, the code must be a `request`, which this
/// device confirms next (`PUT …/approval`); the device that asked must then confirm this
/// account before it is signed in. Signed out, it must be an `offer`, which this device claims
/// (`POST …/claim`) while the other confirms it. A code typed wrong counts against the
/// caller's failures and everyone's, and every way of missing (unknown, expired, cancelled, or
/// a code for a device on the other side) answers `deviceLinkCodeUnknown` alike.
#[utoipa::path(
    post,
    path = "/auth/device-link-entries",
    tag = TAG_AUTH,
    request_body = DeviceLinkEntryRequest,
    security((), ("bearerAuth" = [])),
    responses(
        (status = OK, body = DeviceLinkEntry),
        (status = BAD_REQUEST, description = "`validation`: a code that cannot be one (not eight letters of the code alphabet), or, signed out, without a valid device name or code challenge", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a bot, or a user of another deployment; `twoFactorEnrollmentRequired`; `reauthenticationRequired`: a signed-in giver verifies again first, before the code is looked at", body = Problem),
        (status = NOT_FOUND, description = "`deviceLinkCodeUnknown`: no code waiting for this device matches", body = Problem),
        (status = CONFLICT, description = "`deviceLinkUsed`: another device scanned or typed it first", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn enter_device_link_code(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    failures: Option<Extension<Failures>>,
    Json(request): Json<DeviceLinkEntryRequest>,
) -> ApiResult<Json<DeviceLinkEntry>> {
    let caller = giving_caller(&state, session.as_ref())?;
    // Each entry is counted as a failure before its code is looked at, and given back once it
    // proves not to be one, so entries sent all at once cannot all pass while none has failed
    // yet.
    if let Some(Extension(failures)) = &failures {
        failures.take(&state).await?;
    }
    let entered = device_link::enter(
        &state,
        &request.code,
        caller.as_ref(),
        request.device_name,
        request.code_challenge,
    )
    .await;
    let wrong = matches!(
        entered,
        Err(app::Error::DeviceLinkCodeUnknown | app::Error::DeviceLinkUsed)
    );
    if !wrong && let Some(Extension(failures)) = &failures {
        failures.forgive(&state).await;
    }
    let entered = entered?;
    Ok(Json(DeviceLinkEntry {
        id: entered.id,
        scan: DeviceLinkScan::new(&state, entered.scanned).await?,
    }))
}

/// Where a device link stands.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DeviceLinkProgress {
    /// No device has scanned the code yet.
    Waiting,
    /// A device scanned it and waits for the signed-in device to confirm it.
    Scanned {
        /// The name the device signing in gave itself.
        #[serde(rename = "deviceName")]
        device_name: String,
    },
    /// The signed-in device confirmed it.
    Approved,
}

impl From<Progress> for DeviceLinkProgress {
    fn from(progress: Progress) -> Self {
        match progress {
            Progress::Waiting => Self::Waiting,
            Progress::Scanned { device_name } => Self::Scanned { device_name },
            Progress::Approved => Self::Approved,
        }
    }
}

/// Where an `offer` stands, for the signed-in device that started it, which asks every couple
/// of seconds while it shows the code.
#[utoipa::path(
    get,
    path = "/auth/device-links/{link}",
    tag = TAG_AUTH,
    params(("link" = String, Path, description = "The link's id")),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = DeviceLinkProgress),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "`deviceLinkExpired`, or a link of another sign-in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_device_link(
    State(state): State<GlobalServerContext>,
    SessionUser { caller, .. }: SessionUser,
    Path(link): Path<String>,
) -> ApiResult<Json<DeviceLinkProgress>> {
    Ok(Json(
        device_link::progress(&state, &link, &caller).await?.into(),
    ))
}

/// The signed-in device confirms the device that scanned its code, or whose code it scanned,
/// letting that device claim a sign-in of this account.
#[utoipa::path(
    put,
    path = "/auth/device-links/{link}/approval",
    tag = TAG_AUTH,
    params(("link" = String, Path, description = "The link's id")),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Approved"),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "`deviceLinkExpired`, or a link of another sign-in", body = Problem),
        (status = CONFLICT, description = "`conflict`: no device has scanned the code yet", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn approve_device_link(
    State(state): State<GlobalServerContext>,
    SessionUser { caller, .. }: SessionUser,
    Path(link): Path<String>,
) -> ApiResult<NoContent> {
    device_link::approve(&state, &link, &caller).await?;
    Ok(NoContent)
}

/// Ends a device link before it is claimed: the signed-in device declining, or either device
/// giving up. Holding the id is enough.
#[utoipa::path(
    delete,
    path = "/auth/device-links/{link}",
    tag = TAG_AUTH,
    params(("link" = String, Path, description = "The link's id")),
    responses(
        (status = NO_CONTENT, description = "Ended, or already gone"),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn cancel_device_link(
    State(state): State<GlobalServerContext>,
    Path(link): Path<String>,
) -> ApiResult<NoContent> {
    device_link::cancel(&state, &link).await?;
    Ok(NoContent)
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkClaimRequest {
    /// The verifier whose SHA-256 this device gave as its code challenge.
    pub code_verifier: String,
    /// The account this device's person confirmed, once a claim answered `confirmAccount`.
    #[serde(default)]
    pub account: Option<UserId>,
}

/// The account a device is about to be signed in to, for its person to confirm.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkAccount {
    pub user_id: UserId,
    pub user_name: String,
    pub display_name: Option<String>,
    pub icon: Option<Icon>,
}

impl DeviceLinkAccount {
    async fn new(state: &GlobalServerContext, user: UserId) -> ApiResult<Self> {
        let user = app::user::User::load_from_db(state, user).await?.user_pg;
        let icon = match user.icon.as_ref().map(|icon| *icon.id()) {
            Some(icon) => match app::icon::read_icon(state, icon).await {
                Ok(icon) => Some(icon_to_api(state, icon)),
                // An icon whose upload is not confirmed is shown nowhere.
                Err(app::Error::Diesel(diesel::result::Error::NotFound)) => None,
                Err(e) => return Err(e.into()),
            },
            None => None,
        };
        Ok(Self {
            user_id: user.id,
            user_name: user.name,
            display_name: user.display_name,
            icon,
        })
    }
}

/// What a device signing in finds when it claims.
#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DeviceLinkClaim {
    /// No device has scanned the code yet.
    Waiting,
    /// Scanned, and waiting for the signed-in device to confirm.
    Scanned {
        #[serde(rename = "deviceName")]
        device_name: String,
    },
    /// The signed-in device typed the code rather than scanning it, so it may have guessed it:
    /// this device's person confirms that this is the account they mean to sign in to, and the
    /// device claims again naming it as `account`, or cancels. Only a link started `withCode`
    /// answers it.
    ConfirmAccount(DeviceLinkAccount),
    /// Signed in.
    SignedIn(LoginResponse),
}

/// The device signing in asks for its sign-in, every couple of seconds, with its verifier: the
/// link's progress until the signed-in device confirms it, then the sign-in, once. The sign-in
/// proves what the giving device's did and counts as verified when that one last was. A link
/// whose user code the signed-in device typed answers `confirmAccount` until the claim names
/// that account.
#[utoipa::path(
    post,
    path = "/auth/device-links/{link}/claim",
    tag = TAG_AUTH,
    params(("link" = String, Path, description = "The link's id")),
    request_body = DeviceLinkClaimRequest,
    responses(
        (status = OK, body = DeviceLinkClaim),
        (status = FORBIDDEN, description = "`verificationFailed`: the verifier does not match; `deploymentBanned`", body = Problem),
        (status = NOT_FOUND, description = "`deviceLinkExpired`: expired, declined, already claimed, or the giving device signed out", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn claim_device_link(
    State(state): State<GlobalServerContext>,
    Path(link): Path<String>,
    Json(request): Json<DeviceLinkClaimRequest>,
) -> ApiResult<Json<DeviceLinkClaim>> {
    Ok(Json(
        match device_link::claim(&state, &link, &request.code_verifier, request.account).await? {
            Claim::SignedIn(session) => DeviceLinkClaim::SignedIn(session.into()),
            Claim::ConfirmAccount(user) => {
                DeviceLinkClaim::ConfirmAccount(DeviceLinkAccount::new(&state, user).await?)
            }
            Claim::Pending(Progress::Scanned { device_name }) => {
                DeviceLinkClaim::Scanned { device_name }
            }
            // An approved link is claimed at once, so a claim never reports one pending.
            Claim::Pending(Progress::Waiting | Progress::Approved) => DeviceLinkClaim::Waiting,
        },
    ))
}
