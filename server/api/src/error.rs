//! Error responses for the REST API.
//!
//! Every non-2xx response the API produces is an RFC 9457 Problem Details document served as
//! `application/problem+json`. Handlers return [`ApiResult`] and convert failures through
//! [`ApiError`]; the [`From<app::Error>`] impl covers the common cases (not found, validation,
//! authorization, internal) so most handlers need no explicit error mapping at all. Failures
//! that deserve a more specific [`ProblemCode`] (a username that is already taken, for example)
//! are mapped in the handler that knows the context.

use crate::t;
use aspen_app as app;
pub use aspen_app::PasswordRequirement;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use diesel::result::DatabaseErrorKind;
use serde::Serialize;
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

pub const PROBLEM_JSON: HeaderValue = HeaderValue::from_static("application/problem+json");

/// Stable, machine-readable identifier for a failure. Clients branch on this value; `title` and
/// `detail` are localized prose meant for display only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProblemCode {
    /// The request could not be parsed: malformed JSON, a path segment that is not a UUID, an
    /// unknown query parameter, and so on.
    BadRequest,
    /// The request parsed but violates a business rule. `detail` explains which one.
    Validation,
    /// No valid session token was presented.
    Unauthorized,
    /// The session is valid but is not permitted to perform this action.
    Forbidden,
    /// The addressed resource does not exist (or has been deleted).
    NotFound,
    /// The request conflicts with existing state.
    Conflict,
    /// Voting: the poll has closed.
    PollClosed,
    /// Login: the username or password is wrong.
    InvalidCredentials,
    /// The presented refresh token or sign-in ticket is unknown, expired, or used.
    InvalidToken,
    /// A password, authenticator code, or recovery code presented to verify the caller was
    /// wrong or already used.
    VerificationFailed,
    /// Security settings: the session must verify its user again (`POST
    /// /auth/reauthenticate`) before this change.
    ReauthenticationRequired,
    /// The server requires a second factor this account has not added; the session may only
    /// add one, or sign out.
    TwoFactorEnrollmentRequired,
    /// The deployment requires a verified email address, and this account's is not verified;
    /// the session may only verify, change, or resend it (`/users/@me/email`), or sign out.
    EmailVerificationRequired,
    /// Password reset: the email address given is not the one the account has.
    EmailMismatch,
    /// Password reset cannot start for this account: no account has that username, or it has no
    /// verified email address. `detail` says which, and what to do.
    PasswordResetUnavailable,
    /// Password reset: the reset is unknown, has expired, or is finished. Start again.
    PasswordResetExpired,
    /// Too many wrong codes or passwords recently; try again later.
    TooManyAttempts,
    /// Too many requests to this endpoint recently. `Retry-After` says how many seconds to
    /// wait.
    RateLimited,
    /// The server requires a second factor, so the account's last one cannot be removed.
    LastSecondFactor,
    /// The authenticator's response to a passkey ceremony did not verify.
    PasskeyRejected,
    /// A sign-in code (`/auth/device-links`) is unknown, has expired, or was already used to
    /// sign in. Make a new one.
    DeviceLinkExpired,
    /// A sign-in code was already scanned by another device. Whoever scanned it may be
    /// waiting for the signed-in device to confirm them, which it should not.
    DeviceLinkUsed,
    /// Passkeys are not configured on this server.
    PasskeysUnavailable,
    /// Registration: the requested username is already in use.
    UsernameTaken,
    /// A custom emoji of that name is in the community already.
    CustomEmojiNameTaken,
    /// Invite creation: the requested custom code is already in use.
    InviteCodeTaken,
    /// Registration: this server takes new accounts only with an invite, and none was given.
    RegistrationInviteRequired,
    /// Registration: the invite given does not exist, has expired, was revoked, or is used up.
    RegistrationInviteInvalid,
    /// Only the deployment's administrators may use the Administration Dashboard.
    AdminRequired,
    /// A block stands between the caller and someone they would message, whichever of them
    /// made it; the problem does not say which.
    Blocked,
    /// The caller is banned from the community (`app::ban`); `detail` carries the reason.
    Banned,
    /// The account is banned from this deployment (`app::user_ban`) and cannot sign in;
    /// `detail` carries the reason and when the ban ends, if it does.
    DeploymentBanned,
    /// Reporting: the caller has already reported this, and their report awaits review.
    AlreadyReported,
    /// Password change: the current password did not match.
    OldPasswordIncorrect,
    /// Password change: the new password fails a requirement named in `requirement`.
    PasswordRequirementsNotMet,
    /// The server has too much of this kind of work queued, as when many people sign in at
    /// once, or every database connection stays taken for `database_pool_wait_seconds`.
    /// `Retry-After` says how many seconds to wait.
    ServerBusy,
    /// Another deployment could not be reached, or did not answer as a deployment does.
    /// `detail` says which.
    DeploymentUnreachable,
    /// Federation does not allow this crossing: a gate is closed to that deployment, this
    /// deployment takes no part in federation, or the account cannot travel. `detail` says
    /// which.
    FederationRefused,
    /// Signing in abroad, or a statement another deployment sent: it is malformed, forged,
    /// expired, already used, or not for this deployment. `detail` says which, and what to do.
    AssertionInvalid,
    /// Signing in abroad: this deployment requires two factors, and the sign-in at home used a
    /// password alone. Sign in at home with a second factor or a passkey first.
    StrongerSignInRequired,
    /// A plugin of this deployment refused what was about to be saved; `detail` is its reason,
    /// in the reader's language.
    PluginRefused,
    /// A plugin of this deployment that must decide what is about to be saved could not;
    /// `detail` names it. Retrying shortly may succeed.
    PluginUnavailable,
    /// Something failed on the server. Retrying later may succeed.
    Internal,
}

impl ProblemCode {
    fn default_status(self) -> StatusCode {
        match self {
            ProblemCode::BadRequest | ProblemCode::Validation => StatusCode::BAD_REQUEST,
            ProblemCode::Unauthorized
            | ProblemCode::InvalidCredentials
            | ProblemCode::InvalidToken => StatusCode::UNAUTHORIZED,
            ProblemCode::Forbidden
            | ProblemCode::OldPasswordIncorrect
            | ProblemCode::VerificationFailed
            | ProblemCode::ReauthenticationRequired
            | ProblemCode::TwoFactorEnrollmentRequired
            | ProblemCode::EmailVerificationRequired
            | ProblemCode::EmailMismatch
            | ProblemCode::RegistrationInviteRequired
            | ProblemCode::RegistrationInviteInvalid
            | ProblemCode::AdminRequired
            | ProblemCode::Blocked
            | ProblemCode::Banned
            | ProblemCode::DeploymentBanned => StatusCode::FORBIDDEN,
            ProblemCode::TooManyAttempts | ProblemCode::RateLimited => {
                StatusCode::TOO_MANY_REQUESTS
            }
            ProblemCode::PasskeyRejected => StatusCode::BAD_REQUEST,
            ProblemCode::PasskeysUnavailable => StatusCode::NOT_FOUND,
            ProblemCode::PasswordResetUnavailable | ProblemCode::PasswordResetExpired => {
                StatusCode::NOT_FOUND
            }
            ProblemCode::DeviceLinkExpired => StatusCode::NOT_FOUND,
            ProblemCode::DeviceLinkUsed => StatusCode::CONFLICT,
            ProblemCode::NotFound => StatusCode::NOT_FOUND,
            ProblemCode::Conflict
            | ProblemCode::PollClosed
            | ProblemCode::UsernameTaken
            | ProblemCode::CustomEmojiNameTaken
            | ProblemCode::InviteCodeTaken
            | ProblemCode::AlreadyReported
            | ProblemCode::LastSecondFactor => StatusCode::CONFLICT,
            ProblemCode::PasswordRequirementsNotMet => StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ServerBusy => StatusCode::SERVICE_UNAVAILABLE,
            ProblemCode::DeploymentUnreachable => StatusCode::BAD_GATEWAY,
            ProblemCode::FederationRefused | ProblemCode::StrongerSignInRequired => {
                StatusCode::FORBIDDEN
            }
            ProblemCode::AssertionInvalid => StatusCode::UNAUTHORIZED,
            ProblemCode::PluginRefused => StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::PluginUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ProblemCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn title(self) -> Cow<'static, str> {
        match self {
            ProblemCode::BadRequest => t!("problemBadRequest"),
            ProblemCode::Validation => t!("problemValidation"),
            ProblemCode::Unauthorized => t!("problemUnauthorized"),
            ProblemCode::Forbidden => t!("problemForbidden"),
            ProblemCode::NotFound => t!("problemNotFound"),
            ProblemCode::Conflict => t!("problemConflict"),
            ProblemCode::PollClosed => t!("problemPollClosed"),
            ProblemCode::InvalidCredentials => t!("problemInvalidCredentials"),
            ProblemCode::InvalidToken => t!("problemInvalidToken"),
            ProblemCode::VerificationFailed => t!("problemVerificationFailed"),
            ProblemCode::ReauthenticationRequired => t!("problemReauthenticationRequired"),
            ProblemCode::TwoFactorEnrollmentRequired => t!("problemTwoFactorEnrollmentRequired"),
            ProblemCode::EmailVerificationRequired => t!("problemEmailVerificationRequired"),
            ProblemCode::EmailMismatch => t!("problemEmailMismatch"),
            ProblemCode::PasswordResetUnavailable => t!("problemPasswordResetUnavailable"),
            ProblemCode::PasswordResetExpired => t!("problemPasswordResetExpired"),
            ProblemCode::TooManyAttempts => t!("problemTooManyAttempts"),
            ProblemCode::RateLimited => t!("problemRateLimited"),
            ProblemCode::LastSecondFactor => t!("problemLastSecondFactor"),
            ProblemCode::PasskeyRejected => t!("problemPasskeyRejected"),
            ProblemCode::PasskeysUnavailable => t!("problemPasskeysUnavailable"),
            ProblemCode::DeviceLinkExpired => t!("problemDeviceLinkExpired"),
            ProblemCode::DeviceLinkUsed => t!("problemDeviceLinkUsed"),
            ProblemCode::UsernameTaken => t!("usernameAlreadyTaken"),
            ProblemCode::CustomEmojiNameTaken => t!("customEmojiNameTaken"),
            ProblemCode::InviteCodeTaken => t!("problemInviteCodeTaken"),
            ProblemCode::RegistrationInviteRequired => t!("problemRegistrationInviteRequired"),
            ProblemCode::RegistrationInviteInvalid => t!("problemRegistrationInviteInvalid"),
            ProblemCode::AdminRequired => t!("problemAdminRequired"),
            ProblemCode::Blocked => t!("problemBlocked"),
            ProblemCode::Banned => t!("problemBanned"),
            ProblemCode::DeploymentBanned => t!("problemDeploymentBanned"),
            ProblemCode::AlreadyReported => t!("problemAlreadyReported"),
            ProblemCode::OldPasswordIncorrect => t!("problemOldPasswordIncorrect"),
            ProblemCode::PasswordRequirementsNotMet => t!("problemPasswordRequirementsNotMet"),
            ProblemCode::ServerBusy => t!("problemServerBusy"),
            ProblemCode::DeploymentUnreachable => t!("problemDeploymentUnreachable"),
            ProblemCode::FederationRefused => t!("problemFederationRefused"),
            ProblemCode::AssertionInvalid => t!("problemAssertionInvalid"),
            ProblemCode::StrongerSignInRequired => t!("problemStrongerSignInRequired"),
            ProblemCode::PluginRefused => t!("problemPluginRefused"),
            ProblemCode::PluginUnavailable => t!("problemPluginUnavailable"),
            ProblemCode::Internal => t!("tryAgainLater"),
        }
    }
}

/// RFC 9457 Problem Details body. Served with `Content-Type: application/problem+json`.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub code: ProblemCode,
    /// Localized one-line summary of the problem class.
    pub title: Cow<'static, str>,
    /// The HTTP status code of the response carrying this body.
    #[schema(minimum = 400, maximum = 599)]
    pub status: u16,
    /// Localized explanation specific to this occurrence, when the server has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Cow<'static, str>>,
    /// Set only when `code` is `passwordRequirementsNotMet`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<PasswordRequirement>,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    problem: Problem,
    /// Sent as `Retry-After`, in whole seconds rounded up.
    retry_after: Option<std::time::Duration>,
}

impl ApiError {
    pub fn new(code: ProblemCode) -> Self {
        let status = code.default_status();
        Self {
            status,
            problem: Problem {
                code,
                title: code.title(),
                status: status.as_u16(),
                detail: None,
                requirement: None,
            },
            retry_after: None,
        }
    }

    pub fn with_retry_after(mut self, retry_after: std::time::Duration) -> Self {
        self.retry_after = Some(retry_after);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<Cow<'static, str>>) -> Self {
        self.problem.detail = Some(detail.into());
        self
    }

    pub fn password_requirement(requirement: PasswordRequirement) -> Self {
        let mut e = Self::new(ProblemCode::PasswordRequirementsNotMet);
        e.problem.requirement = Some(requirement);
        e
    }
}

/// How long a client told `serverBusy` waits before trying again.
const BUSY_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

impl From<app::Error> for ApiError {
    fn from(e: app::Error) -> Self {
        match e {
            app::Error::Diesel(diesel::result::Error::NotFound) => Self::new(ProblemCode::NotFound),
            app::Error::Diesel(diesel::result::Error::DatabaseError(
                DatabaseErrorKind::UniqueViolation,
                _,
            )) => Self::new(ProblemCode::Conflict),
            app::Error::Validation(reason) => {
                Self::new(ProblemCode::Validation).with_detail(reason)
            }
            app::Error::PasswordRequirement(requirement) => Self::password_requirement(requirement)
                .with_detail(t!(
                    "passwordTooShort",
                    min = app::login::PASSWORD_MIN_LENGTH
                )),
            app::Error::Unauthorized => Self::new(ProblemCode::Forbidden),
            app::Error::Forbidden(reason) => Self::new(ProblemCode::Forbidden).with_detail(reason),
            app::Error::Unauthenticated => Self::new(ProblemCode::Unauthorized),
            app::Error::Conflict(reason) => Self::new(ProblemCode::Conflict).with_detail(reason),
            app::Error::VerificationFailed => Self::new(ProblemCode::VerificationFailed),
            app::Error::ReauthenticationRequired => {
                Self::new(ProblemCode::ReauthenticationRequired)
            }
            app::Error::TooManyAttempts => Self::new(ProblemCode::TooManyAttempts),
            app::Error::EmailMismatch => {
                Self::new(ProblemCode::EmailMismatch).with_detail(t!("emailMismatchDetail"))
            }
            app::Error::PasswordResetUnavailable(reason) => {
                Self::new(ProblemCode::PasswordResetUnavailable).with_detail(reason)
            }
            app::Error::PasswordResetExpired => Self::new(ProblemCode::PasswordResetExpired)
                .with_detail(t!("passwordResetExpiredDetail")),
            app::Error::LastSecondFactor => Self::new(ProblemCode::LastSecondFactor),
            app::Error::InvalidTicket => Self::new(ProblemCode::InvalidToken),
            app::Error::DeviceLinkExpired => {
                Self::new(ProblemCode::DeviceLinkExpired).with_detail(t!("deviceLinkExpiredDetail"))
            }
            app::Error::DeviceLinkUsed => {
                Self::new(ProblemCode::DeviceLinkUsed).with_detail(t!("deviceLinkUsedDetail"))
            }
            app::Error::PasskeysUnavailable => Self::new(ProblemCode::PasskeysUnavailable),
            app::Error::PasskeyRejected(reason) => {
                tracing::debug!(reason, "passkey rejected");
                Self::new(ProblemCode::PasskeyRejected)
            }
            app::Error::PollClosed => Self::new(ProblemCode::PollClosed),
            app::Error::RegistrationInviteRequired => {
                Self::new(ProblemCode::RegistrationInviteRequired)
            }
            app::Error::RegistrationInviteInvalid => {
                Self::new(ProblemCode::RegistrationInviteInvalid)
            }
            app::Error::AdminRequired => Self::new(ProblemCode::AdminRequired),
            app::Error::Blocked => Self::new(ProblemCode::Blocked),
            app::Error::Banned { reason } => {
                Self::new(ProblemCode::Banned).with_detail(match reason {
                    Some(reason) => t!("bannedWithReason", reason = reason),
                    None => t!("bannedNoReason"),
                })
            }
            app::Error::DeploymentBanned { reason, until } => {
                let until = until.map(|until| until.format("%Y-%m-%d %H:%M UTC").to_string());
                Self::new(ProblemCode::DeploymentBanned).with_detail(match (reason, until) {
                    (Some(reason), Some(until)) => {
                        t!(
                            "deploymentBannedUntilWithReason",
                            until = until,
                            reason = reason
                        )
                    }
                    (Some(reason), None) => t!("deploymentBannedWithReason", reason = reason),
                    (None, Some(until)) => t!("deploymentBannedUntil", until = until),
                    (None, None) => t!("deploymentBannedNoReason"),
                })
            }
            app::Error::AlreadyReported => Self::new(ProblemCode::AlreadyReported),
            app::Error::DeploymentUnreachable(detail) => {
                Self::new(ProblemCode::DeploymentUnreachable).with_detail(detail)
            }
            app::Error::FederationRefused(detail) => {
                Self::new(ProblemCode::FederationRefused).with_detail(detail)
            }
            app::Error::AssertionInvalid(reason) => {
                Self::new(ProblemCode::AssertionInvalid).with_detail(reason)
            }
            app::Error::StrongerSignInRequired => Self::new(ProblemCode::StrongerSignInRequired),
            app::Error::PluginRefused(detail) => {
                Self::new(ProblemCode::PluginRefused).with_detail(detail)
            }
            app::Error::PluginUnavailable(detail) => {
                Self::new(ProblemCode::PluginUnavailable).with_detail(detail)
            }
            app::Error::Busy => {
                Self::new(ProblemCode::ServerBusy).with_retry_after(BUSY_RETRY_AFTER)
            }
            app::Error::Deadpool(
                diesel_async::pooled_connection::deadpool::PoolError::Timeout(_),
            ) => {
                tracing::warn!(
                    "no database connection came free within database_pool_wait_seconds"
                );
                Self::new(ProblemCode::ServerBusy).with_retry_after(BUSY_RETRY_AFTER)
            }
            other => {
                error!(error = other.to_string(), "request failed");
                Self::new(ProblemCode::Internal)
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            [(CONTENT_TYPE, PROBLEM_JSON)],
            axum::Json(self.problem),
        )
            .into_response();
        if let Some(retry_after) = self.retry_after {
            let seconds = retry_after.as_millis().div_ceil(1000).max(1);
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(u64::try_from(seconds).unwrap_or(u64::MAX)),
            );
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
