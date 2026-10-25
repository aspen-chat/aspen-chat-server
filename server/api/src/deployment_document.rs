//! The deployment's Privacy Policy and Terms of Use at `/deployment/documents/{document}`
//! (`app::deployment_document`), read by anyone, since the sign-in screen links to them, and
//! published or withdrawn by holders of Manage deployment settings.

use crate::TAG_DEPLOYMENT;
use crate::admin::AdminUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path};
use aspen_app::context::GlobalServerContext;
use aspen_app::deployment_document::{self, Document, DocumentKind};
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One of the deployment's documents.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublishedDocument {
    pub kind: DocumentKind,
    /// Markdown, as the deployment's administrators wrote it; `null` when the deployment has not
    /// published this document. There is no default.
    pub body: Option<String>,
    /// When it was last published; `null` with `body`.
    pub updated_at: Option<DateTime<Utc>>,
}

impl PublishedDocument {
    fn new(kind: DocumentKind, document: Option<Document>) -> Self {
        let (body, updated_at) = document
            .map(|document| (document.body, document.updated_at))
            .unzip();
        Self {
            kind,
            body,
            updated_at,
        }
    }
}

/// A document to publish in place of the one published before.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishDocumentRequest {
    /// Markdown, not blank, and at most 65,536 characters.
    pub body: String,
}

/// One of the deployment's documents, published or not. Unauthenticated.
#[utoipa::path(
    get,
    path = "/deployment/documents/{document}",
    tag = TAG_DEPLOYMENT,
    params(("document" = DocumentKind, Path)),
    responses(
        (status = OK, body = PublishedDocument),
        (status = BAD_REQUEST, description = "A document there is no such kind of", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_document(
    State(state): State<GlobalServerContext>,
    Path(kind): Path<DocumentKind>,
) -> ApiResult<Json<PublishedDocument>> {
    let document = deployment_document::read(&state, kind).await?;
    Ok(Json(PublishedDocument::new(kind, document)))
}

/// Publishes one of the deployment's documents, in place of the one published before. Takes
/// Manage deployment settings.
#[utoipa::path(
    put,
    path = "/deployment/documents/{document}",
    tag = TAG_DEPLOYMENT,
    params(("document" = DocumentKind, Path)),
    request_body = PublishDocumentRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = PublishedDocument),
        (status = BAD_REQUEST, description = "A body that is blank or too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage deployment settings", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn publish_document(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path(kind): Path<DocumentKind>,
    Json(request): Json<PublishDocumentRequest>,
) -> ApiResult<Json<PublishedDocument>> {
    let document = deployment_document::publish_as(&state, &access, kind, request.body).await?;
    tracing::info!(%kind, admin = %session.user.id.0, "published a deployment document");
    Ok(Json(PublishedDocument::new(kind, Some(document))))
}

/// Withdraws one of the deployment's documents, leaving it without one. Takes Manage deployment
/// settings.
#[utoipa::path(
    delete,
    path = "/deployment/documents/{document}",
    tag = TAG_DEPLOYMENT,
    params(("document" = DocumentKind, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Withdrawn, or never published"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage deployment settings", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn withdraw_document(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path(kind): Path<DocumentKind>,
) -> ApiResult<NoContent> {
    deployment_document::withdraw_as(&state, &access, kind).await?;
    tracing::info!(%kind, admin = %session.user.id.0, "withdrew a deployment document");
    Ok(NoContent)
}
