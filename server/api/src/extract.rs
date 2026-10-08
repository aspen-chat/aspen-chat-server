//! Extractors and responders shared by every handler.
//!
//! The wrappers keep axum's names (`Json`, `Path`, `Query`) on purpose: utoipa recognises
//! handler arguments by the type's final path segment, so handlers documented with
//! `#[utoipa::path]` keep their automatically inferred request bodies and path parameters while
//! gaining Problem Details rejections instead of axum's plain-text defaults.

use crate::error::{ApiError, ProblemCode};
use crate::t;
pub use aspen_wire::double_option;
use axum::body::{Body, Bytes};
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A JSON request body. Text holding U+0000 is refused here with `validation`, since PostgreSQL
/// stores no NUL in text and would otherwise refuse the write with an error of its own.
pub struct Json<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Json<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let (parts, body) = request.into_parts();
        let bytes = Bytes::from_request(Request::from_parts(parts.clone(), body), state)
            .await
            .map_err(|rejection| {
                ApiError::new(ProblemCode::BadRequest).with_detail(rejection.body_text())
            })?;
        if json_has_nul(&bytes) {
            return Err(nul_refused());
        }
        let request = Request::from_parts(parts, Body::from(bytes));
        let axum::Json(value) = axum::Json::<T>::from_request(request, state).await?;
        Ok(Self(value))
    }
}

/// Whether JSON text holds an escaped U+0000 (`\u0000`) in a string. JSON holds no raw control
/// characters, and backslashes appear only in strings, each starting a two-character escape or a
/// `\uXXXX`, so stepping over escapes finds every one.
fn json_has_nul(json: &[u8]) -> bool {
    let mut at = 0;
    while at < json.len() {
        if json[at] == b'\\' {
            if json[at + 1..].starts_with(b"u0000") {
                return true;
            }
            at += 2;
        } else {
            at += 1;
        }
    }
    false
}

/// Whether a URL's path or query holds `%00`, which decodes to U+0000.
fn url_has_nul(text: &str) -> bool {
    text.as_bytes()
        .windows(3)
        .any(|window| window[0] == b'%' && window[1] == b'0' && window[2] == b'0')
}

fn nul_refused() -> ApiError {
    ApiError::new(ProblemCode::Validation).with_detail(t!("textHasNul"))
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// Path parameters, refusing `%00` as `Json` refuses U+0000.
pub struct Path<T>(pub T);

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for Path<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        if url_has_nul(parts.uri.path()) {
            return Err(nul_refused());
        }
        let axum::extract::Path(value) =
            axum::extract::Path::<T>::from_request_parts(parts, state).await?;
        Ok(Self(value))
    }
}

/// Query parameters, refusing `%00` as `Json` refuses U+0000.
pub struct Query<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for Query<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        if parts.uri.query().is_some_and(url_has_nul) {
            return Err(nul_refused());
        }
        let axum::extract::Query(value) =
            axum::extract::Query::<T>::from_request_parts(parts, state).await?;
        Ok(Self(value))
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_an_escaped_nul_and_only_that() {
        assert!(json_has_nul(br#"{"content":"a\u0000b"}"#));
        assert!(json_has_nul(br#"["\\\u0000"]"#));
        // An escaped backslash followed by the text u0000 is no NUL.
        assert!(!json_has_nul(br#"{"content":"\\u0000"}"#));
        assert!(!json_has_nul(br#"{"content":"\u0001"}"#));
        assert!(!json_has_nul(br#"{"content":"plain"}"#));
        // A trailing backslash (invalid JSON, refused by the parser) does not panic.
        assert!(!json_has_nul(b"\\"));
    }

    #[test]
    fn finds_percent_zero_zero() {
        assert!(url_has_nul("filter%5Bname%5D=a%00b"));
        assert!(!url_has_nul("filter%5Bname%5D=a%2500b"));
        assert!(!url_has_nul("/channels/0190a4b2"));
    }
}
