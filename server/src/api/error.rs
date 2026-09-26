//! Error responses for the REST API.
//!
//! Every non-2xx response the API produces is an RFC 9457 Problem Details document served as
//! `application/problem+json`. Handlers return [`ApiResult`] and convert failures through
//! [`ApiError`]; the [`From<app::Error>`] impl covers the common cases (not found, validation,
//! authorization, internal) so most handlers need no explicit error mapping at all. Failures
//! that deserve a more specific [`ProblemCode`] (a username that is already taken, for example)
//! are mapped in the handler that knows the context.

use crate::app;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use diesel::result::DatabaseErrorKind;
use rust_i18n::t;
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
    /// The presented refresh token is unknown or expired.
    InvalidToken,
    /// Registration: the requested username is already in use.
    UsernameTaken,
    /// Invite creation: the requested custom code is already in use.
    InviteCodeTaken,
    /// Password change: the current password did not match.
    OldPasswordIncorrect,
    /// Password change: the new password fails a requirement named in `requirement`.
    PasswordRequirementsNotMet,
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
            ProblemCode::Forbidden | ProblemCode::OldPasswordIncorrect => StatusCode::FORBIDDEN,
            ProblemCode::NotFound => StatusCode::NOT_FOUND,
            ProblemCode::Conflict
            | ProblemCode::PollClosed
            | ProblemCode::UsernameTaken
            | ProblemCode::InviteCodeTaken => StatusCode::CONFLICT,
            ProblemCode::PasswordRequirementsNotMet => StatusCode::UNPROCESSABLE_ENTITY,
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
            ProblemCode::UsernameTaken => t!("usernameAlreadyTaken"),
            ProblemCode::InviteCodeTaken => t!("problemInviteCodeTaken"),
            ProblemCode::OldPasswordIncorrect => t!("problemOldPasswordIncorrect"),
            ProblemCode::PasswordRequirementsNotMet => t!("problemPasswordRequirementsNotMet"),
            ProblemCode::Internal => t!("tryAgainLater"),
        }
    }
}

/// A password rule the new password failed to satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PasswordRequirement {
    /// The password is shorter than the minimum length.
    Length,
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
        }
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
            app::Error::PollClosed => Self::new(ProblemCode::PollClosed),
            other => {
                error!(error = other.to_string(), "request failed");
                Self::new(ProblemCode::Internal)
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            [(CONTENT_TYPE, PROBLEM_JSON)],
            axum::Json(self.problem),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
