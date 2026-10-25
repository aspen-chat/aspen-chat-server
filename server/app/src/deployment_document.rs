//! What the deployment publishes for anyone to read, signed in or not: its Privacy Policy and its
//! Terms of Use, each Markdown its administrators write in the Administration Dashboard, or the
//! terminal (`aspen-chat-server documents`). Aspen gives no default for either: a document the
//! deployment has not published has no row, and the apps say so where they would show it.
//!
//! Reading one needs nothing; publishing or withdrawing one takes Manage deployment settings.
//! No event carries them: the apps read a document when they open it.

use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission};
use crate::t;
use aspen_schema::deployment_document;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::Text;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The longest a document may be, in characters; the column's check holds it too.
pub const MAX_CHARS: usize = 65_536;

/// Which document.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = Text)]
pub enum DocumentKind {
    PrivacyPolicy,
    TermsOfUse,
}

crate::wire_name_traits!(DocumentKind);
crate::text_sql_traits!(DocumentKind);

/// A published document.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = deployment_document)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Document {
    pub kind: DocumentKind,
    /// Markdown, as its administrators wrote it.
    pub body: String,
    pub updated_at: DateTime<Utc>,
}

/// The document as published, or `None` where the deployment has not published one.
pub async fn read(
    state: &GlobalServerContext,
    kind: DocumentKind,
) -> crate::Result<Option<Document>> {
    let mut conn = state.connection_pool.get().await?;
    load(conn.as_mut(), kind).await
}

/// [`read`], on a connection of the caller's.
pub async fn load(
    conn: &mut AsyncPgConnection,
    kind: DocumentKind,
) -> crate::Result<Option<Document>> {
    Ok(deployment_document::table
        .find(kind)
        .select(Document::as_select())
        .first(conn)
        .await
        .optional()?)
}

/// Publishes `body` as the document, in place of any published before, for someone with Manage
/// deployment settings.
pub async fn publish_as(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    kind: DocumentKind,
    body: String,
) -> crate::Result<Document> {
    access.require(DeploymentPermission::ManageDeploymentSettings)?;
    let mut conn = state.connection_pool.get().await?;
    publish(conn.as_mut(), kind, body).await
}

/// Publishes `body` as the document, in place of any published before. A body that is blank, or
/// longer than [`MAX_CHARS`], is refused.
pub async fn publish(
    conn: &mut AsyncPgConnection,
    kind: DocumentKind,
    body: String,
) -> crate::Result<Document> {
    if body.trim().is_empty() {
        return Err(crate::Error::Validation(t!("documentBlank")));
    }
    if body.chars().count() > MAX_CHARS {
        return Err(crate::Error::Validation(t!(
            "documentTooLong",
            max = MAX_CHARS
        )));
    }
    Ok(diesel::insert_into(deployment_document::table)
        .values((
            deployment_document::kind.eq(kind),
            deployment_document::body.eq(&body),
        ))
        .on_conflict(deployment_document::kind)
        .do_update()
        .set((
            deployment_document::body.eq(&body),
            deployment_document::updated_at.eq(diesel::dsl::now),
        ))
        .returning(Document::as_returning())
        .get_result(conn)
        .await?)
}

/// Withdraws the document, for someone with Manage deployment settings, leaving the deployment
/// without one. Withdrawing one never published does nothing.
pub async fn withdraw_as(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    kind: DocumentKind,
) -> crate::Result<()> {
    access.require(DeploymentPermission::ManageDeploymentSettings)?;
    let mut conn = state.connection_pool.get().await?;
    withdraw(conn.as_mut(), kind).await?;
    Ok(())
}

/// Withdraws the document; whether there was one to withdraw.
pub async fn withdraw(conn: &mut AsyncPgConnection, kind: DocumentKind) -> crate::Result<bool> {
    let removed = diesel::delete(deployment_document::table.find(kind))
        .execute(conn)
        .await?;
    Ok(removed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel_async::AsyncConnection;

    #[tokio::test]
    async fn a_document_is_published_replaced_and_withdrawn() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        let kind = DocumentKind::TermsOfUse;
        withdraw(&mut conn, kind).await.unwrap();
        assert_eq!(load(&mut conn, kind).await.unwrap(), None);

        let first = publish(&mut conn, kind, "# Terms\n\nBe kind.".into())
            .await
            .unwrap();
        let second = publish(&mut conn, kind, "Be kinder.".into()).await.unwrap();
        assert_eq!(load(&mut conn, kind).await.unwrap(), Some(second.clone()));
        assert!(second.updated_at >= first.updated_at);
        for refused in [" \n\t".to_string(), "字".repeat(MAX_CHARS + 1)] {
            assert!(matches!(
                publish(&mut conn, kind, refused).await,
                Err(crate::Error::Validation(_))
            ));
        }
        // As many characters as allowed are taken, however many bytes they are.
        publish(&mut conn, kind, "字".repeat(MAX_CHARS))
            .await
            .unwrap();

        assert!(withdraw(&mut conn, kind).await.unwrap());
        assert!(!withdraw(&mut conn, kind).await.unwrap());
        assert_eq!(load(&mut conn, kind).await.unwrap(), None);
    }
}
