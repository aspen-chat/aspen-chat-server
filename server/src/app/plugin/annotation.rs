//! What plugins say about messages and people: one annotation of each kind per plugin and
//! subject, set again to replace it. A message's annotations are published in its channel and
//! sideloaded with its reads, so whoever may read the message sees them and nobody else does; a
//! person's reach whoever shares a community with them, as their profile does, and are read with
//! `GET /users/{user}/annotations`. Clients draw both from the plugin's catalogue, needing no
//! code of the plugin's.

use super::PluginText;
use super::host::wit;
use crate::api::message_enum::server_event::{
    MessageAnnotationEvent, ServerEvent, UserAnnotationEvent,
};
use crate::api::message_enum::{MessageAnnotation, UserAnnotation};
use crate::app::context::GlobalServerContext;
use crate::app::{self, AnnotationId, EventScope, MessageId, UserId, publish_event};
use crate::database::schema::{message_annotation, user, user_annotation};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How much an annotation matters, which decides how a client draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Notice,
    Warning,
}

app::wire_name_traits!(Severity);

/// The most arguments a plugin's text may fill in.
const MAX_ARGS: usize = 16;
/// The longest an argument may be, in characters.
const MAX_ARG_CHARS: usize = 500;
/// The longest a link may be, in bytes.
const MAX_LINK: usize = 2000;

/// An annotation a plugin set, checked.
#[derive(Debug, Clone)]
pub struct Annotation {
    pub kind: String,
    pub severity: Severity,
    pub label: PluginText,
    pub detail: Option<PluginText>,
    pub link: Option<String>,
}

fn check_text(text: &PluginText) -> Result<(), wit::Error> {
    if text.key.is_empty() || text.key.len() > 64 {
        return Err(wit::Error::Invalid("a text's key is 1 to 64 bytes".into()));
    }
    if text.args.len() > MAX_ARGS
        || text
            .args
            .iter()
            .any(|(k, v)| k.len() > 64 || v.chars().count() > MAX_ARG_CHARS)
    {
        return Err(wit::Error::Invalid(format!(
            "a text has at most {MAX_ARGS} arguments of at most {MAX_ARG_CHARS} characters"
        )));
    }
    Ok(())
}

impl TryFrom<wit::Annotation> for Annotation {
    type Error = wit::Error;

    fn try_from(annotation: wit::Annotation) -> Result<Self, wit::Error> {
        let kind = annotation.kind;
        if kind.is_empty()
            || kind.len() > 64
            || !kind
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(wit::Error::Invalid(
                "an annotation's kind is 1 to 64 letters, digits, dots, hyphens, and \
                 underscores"
                    .into(),
            ));
        }
        let label = PluginText::from(annotation.label);
        check_text(&label)?;
        let detail = annotation.detail.map(PluginText::from);
        if let Some(detail) = &detail {
            check_text(detail)?;
        }
        if let Some(link) = &annotation.link {
            let https = url::Url::parse(link).is_ok_and(|u| u.scheme() == "https");
            if !https || link.len() > MAX_LINK {
                return Err(wit::Error::Invalid(format!(
                    "a link is an https URL of at most {MAX_LINK} bytes"
                )));
            }
        }
        Ok(Annotation {
            kind,
            severity: match annotation.severity {
                wit::Severity::Info => Severity::Info,
                wit::Severity::Notice => Severity::Notice,
                wit::Severity::Warning => Severity::Warning,
            },
            label,
            detail,
            link: annotation.link,
        })
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = message_annotation)]
struct MessageAnnotationRow {
    id: AnnotationId,
    plugin: String,
    message: MessageId,
    kind: String,
    #[diesel(deserialize_as = String)]
    severity: SeverityText,
    label: PluginText,
    detail: Option<PluginText>,
    link: Option<String>,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = user_annotation)]
struct UserAnnotationRow {
    id: AnnotationId,
    plugin: String,
    user: UserId,
    kind: String,
    #[diesel(deserialize_as = String)]
    severity: SeverityText,
    label: PluginText,
    detail: Option<PluginText>,
    link: Option<String>,
}

/// A severity as stored, read through its wire name.
struct SeverityText(Severity);

impl From<String> for SeverityText {
    fn from(text: String) -> Self {
        SeverityText(text.parse().unwrap_or(Severity::Info))
    }
}

impl From<MessageAnnotationRow> for MessageAnnotation {
    fn from(row: MessageAnnotationRow) -> Self {
        MessageAnnotation {
            id: row.id,
            message: row.message,
            plugin: row.plugin,
            kind: row.kind,
            severity: row.severity.0,
            label: row.label,
            detail: row.detail,
            link: row.link,
        }
    }
}

impl From<UserAnnotationRow> for UserAnnotation {
    fn from(row: UserAnnotationRow) -> Self {
        UserAnnotation {
            id: row.id,
            user: row.user,
            plugin: row.plugin,
            kind: row.kind,
            severity: row.severity.0,
            label: row.label,
            detail: row.detail,
            link: row.link,
        }
    }
}

/// Sets `plugin`'s annotation of its kind on `message`, replacing one it set before, and
/// announces it in the message's channel.
pub async fn set_on_message(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    plugin: &str,
    message: MessageId,
    annotation: Annotation,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            let existing: Option<AnnotationId> = message_annotation::table
                .select(message_annotation::id)
                .filter(
                    message_annotation::message
                        .eq(message)
                        .and(message_annotation::plugin.eq(plugin))
                        .and(message_annotation::kind.eq(&annotation.kind)),
                )
                .for_update()
                .first(conn)
                .await
                .optional()?;
            let severity = annotation.severity.to_string();
            let event = match existing {
                Some(id) => {
                    diesel::update(message_annotation::table.filter(message_annotation::id.eq(id)))
                        .set((
                            message_annotation::severity.eq(&severity),
                            message_annotation::label.eq(&annotation.label),
                            message_annotation::detail.eq(&annotation.detail),
                            message_annotation::link.eq(&annotation.link),
                            message_annotation::updated_at.eq(diesel::dsl::now),
                        ))
                        .execute(conn)
                        .await?;
                    MessageAnnotationEvent::Update {
                        id,
                        severity: Some(annotation.severity),
                        label: Some(annotation.label),
                        detail: Some(annotation.detail),
                        link: Some(annotation.link),
                    }
                }
                None => {
                    let id = AnnotationId::new();
                    diesel::insert_into(message_annotation::table)
                        .values((
                            message_annotation::id.eq(id),
                            message_annotation::plugin.eq(plugin),
                            message_annotation::message.eq(message),
                            message_annotation::kind.eq(&annotation.kind),
                            message_annotation::severity.eq(&severity),
                            message_annotation::label.eq(&annotation.label),
                            message_annotation::detail.eq(&annotation.detail),
                            message_annotation::link.eq(&annotation.link),
                        ))
                        .execute(conn)
                        .await?;
                    MessageAnnotationEvent::Create(MessageAnnotation {
                        id,
                        message,
                        plugin: plugin.to_string(),
                        kind: annotation.kind,
                        severity: annotation.severity,
                        label: annotation.label,
                        detail: annotation.detail,
                        link: annotation.link,
                    })
                }
            };
            publish_event(
                state,
                conn,
                EventScope::Message(message),
                &ServerEvent::MessageAnnotation(event),
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// Takes away `plugin`'s annotation of `kind` on `message`, if it set one.
pub async fn clear_on_message(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    plugin: &str,
    message: MessageId,
    kind: &str,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            let deleted: Option<AnnotationId> = diesel::delete(
                message_annotation::table.filter(
                    message_annotation::message
                        .eq(message)
                        .and(message_annotation::plugin.eq(plugin))
                        .and(message_annotation::kind.eq(kind)),
                ),
            )
            .returning(message_annotation::id)
            .get_result(conn)
            .await
            .optional()?;
            if let Some(id) = deleted {
                publish_event(
                    state,
                    conn,
                    EventScope::Message(message),
                    &ServerEvent::MessageAnnotation(MessageAnnotationEvent::Delete { id }),
                )
                .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Sets `plugin`'s annotation of its kind on `subject`'s profile, replacing one it set before,
/// and announces it wherever their profile is.
pub async fn set_on_user(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    plugin: &str,
    subject: UserId,
    annotation: Annotation,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            // The person must be there to be spoken of.
            user::table
                .select(user::id)
                .filter(user::id.eq(subject).and(user::deleted_at.is_null()))
                .first::<UserId>(conn)
                .await?;
            let existing: Option<AnnotationId> = user_annotation::table
                .select(user_annotation::id)
                .filter(
                    user_annotation::user
                        .eq(subject)
                        .and(user_annotation::plugin.eq(plugin))
                        .and(user_annotation::kind.eq(&annotation.kind)),
                )
                .for_update()
                .first(conn)
                .await
                .optional()?;
            let severity = annotation.severity.to_string();
            let event = match existing {
                Some(id) => {
                    diesel::update(user_annotation::table.filter(user_annotation::id.eq(id)))
                        .set((
                            user_annotation::severity.eq(&severity),
                            user_annotation::label.eq(&annotation.label),
                            user_annotation::detail.eq(&annotation.detail),
                            user_annotation::link.eq(&annotation.link),
                            user_annotation::updated_at.eq(diesel::dsl::now),
                        ))
                        .execute(conn)
                        .await?;
                    UserAnnotationEvent::Update {
                        id,
                        severity: Some(annotation.severity),
                        label: Some(annotation.label),
                        detail: Some(annotation.detail),
                        link: Some(annotation.link),
                    }
                }
                None => {
                    let id = AnnotationId::new();
                    diesel::insert_into(user_annotation::table)
                        .values((
                            user_annotation::id.eq(id),
                            user_annotation::plugin.eq(plugin),
                            user_annotation::user.eq(subject),
                            user_annotation::kind.eq(&annotation.kind),
                            user_annotation::severity.eq(&severity),
                            user_annotation::label.eq(&annotation.label),
                            user_annotation::detail.eq(&annotation.detail),
                            user_annotation::link.eq(&annotation.link),
                        ))
                        .execute(conn)
                        .await?;
                    UserAnnotationEvent::Create(UserAnnotation {
                        id,
                        user: subject,
                        plugin: plugin.to_string(),
                        kind: annotation.kind,
                        severity: annotation.severity,
                        label: annotation.label,
                        detail: annotation.detail,
                        link: annotation.link,
                    })
                }
            };
            publish_event(
                state,
                conn,
                EventScope::UserEverywhere(subject),
                &ServerEvent::UserAnnotation(event),
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// Takes away `plugin`'s annotation of `kind` on `subject`'s profile, if it set one.
pub async fn clear_on_user(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    plugin: &str,
    subject: UserId,
    kind: &str,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            let deleted: Option<AnnotationId> = diesel::delete(
                user_annotation::table.filter(
                    user_annotation::user
                        .eq(subject)
                        .and(user_annotation::plugin.eq(plugin))
                        .and(user_annotation::kind.eq(kind)),
                ),
            )
            .returning(user_annotation::id)
            .get_result(conn)
            .await
            .optional()?;
            if let Some(id) = deleted {
                publish_event(
                    state,
                    conn,
                    EventScope::UserEverywhere(subject),
                    &ServerEvent::UserAnnotation(UserAnnotationEvent::Delete { id }),
                )
                .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// The annotations of `messages`, which the caller has already been allowed to read.
pub async fn of_messages(
    state: &GlobalServerContext,
    messages: &[MessageId],
) -> app::Result<Vec<MessageAnnotation>> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    Ok(message_annotation::table
        .select(MessageAnnotationRow::as_select())
        .filter(message_annotation::message.eq_any(messages))
        .order(message_annotation::id.asc())
        .load(conn.as_mut())
        .await?
        .into_iter()
        .map(MessageAnnotation::from)
        .collect())
}

/// The annotations of `subject`'s profile.
pub async fn of_user(
    state: &GlobalServerContext,
    subject: UserId,
) -> app::Result<Vec<UserAnnotation>> {
    let mut conn = state.connection_pool.get().await?;
    Ok(user_annotation::table
        .select(UserAnnotationRow::as_select())
        .filter(user_annotation::user.eq(subject))
        .order(user_annotation::id.asc())
        .load(conn.as_mut())
        .await?
        .into_iter()
        .map(UserAnnotation::from)
        .collect())
}
