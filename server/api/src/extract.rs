//! Extractors and responders shared by every handler.
//!
//! The wrappers keep axum's names (`Json`, `Path`, `Query`) on purpose: utoipa recognises
//! handler arguments by the type's final path segment, so handlers documented with
//! `#[utoipa::path]` keep their automatically inferred request bodies and path parameters while
//! gaining Problem Details rejections instead of axum's plain-text defaults.

use crate::error::{ApiError, ProblemCode};
pub use aspen_wire::double_option;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts};
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct Json<T>(pub T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct Path<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct Query<T>(pub T);

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        ApiError::new(ProblemCode::BadRequest).with_detail(rejection.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        ApiError::new(ProblemCode::BadRequest).with_detail(rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        ApiError::new(ProblemCode::BadRequest).with_detail(rejection.body_text())
    }
}

/// `201 Created` with a `Location` header pointing at the new resource and the resource itself
/// as the JSON body.
pub struct Created<T> {
    pub location: String,
    pub body: T,
}

impl<T> Created<T> {
    pub fn new(location: impl Into<String>, body: T) -> Self {
        Self {
            location: location.into(),
            body,
        }
    }
}

impl<T: Serialize> IntoResponse for Created<T> {
    fn into_response(self) -> Response {
        (
            StatusCode::CREATED,
            [(LOCATION, self.location)],
            axum::Json(self.body),
        )
            .into_response()
    }
}

/// `204 No Content`.
pub struct NoContent;

impl IntoResponse for NoContent {
    fn into_response(self) -> Response {
        StatusCode::NO_CONTENT.into_response()
    }
}
