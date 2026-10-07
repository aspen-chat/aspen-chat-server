//! The deployment's newsletter, when its administrators turn on the setting
//! `newsletter_enabled`: posts written in Markdown by holders of Send newsletters, sent to every
//! account that subscribed at a verified address.
//!
//! A post is a draft until it is sent: it may be edited, deleted, and sent as a test to its
//! sender's own verified address. Sending fixes it and marks it sent; the outbox's senders then
//! queue its mail a batch of subscribers at a time ([`queue_some`]), in order of their ids, so
//! a newsletter to a million subscribers is never one transaction, and a server stopping midway
//! leaves the rest to the next. Each piece is checked again as it is sent: an account that
//! unsubscribed or lost its verified address meanwhile receives nothing. Sent posts are kept,
//! as the newsletter's archive.

use super::outbox::{self, Mail};
pub use crate::NewsletterPostId;
use crate::UserId;
use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission};
use crate::t;
use aspen_schema::{email_outbox, newsletter_post, user, user_email};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use pulldown_cmark::{CowStr, Event};

/// The longest subject, in characters.
pub const SUBJECT_MAX_CHARS: usize = 200;
/// The longest body, in characters.
pub const BODY_MAX_CHARS: usize = 100_000;
/// How many subscribers' mail one step of queueing a post writes.
const QUEUE_BATCH: i64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = newsletter_post)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewsletterPost {
    pub id: NewsletterPostId,
    pub subject: String,
    /// Markdown, as messages are written.
    pub body: String,
    pub author: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub sent_by: Option<UserId>,
    /// When every subscriber's mail had been queued.
    pub queued_at: Option<DateTime<Utc>>,
    /// How many subscribers its mail has been queued for.
    pub recipients: i64,
}

/// `body`, a post's Markdown, as HTML for mail. HTML written into the Markdown is shown as the
/// text it is, never passed through.
pub fn html(body: &str) -> String {
    let events = crate::markdown::parser(body).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::Start(pulldown_cmark::Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) if !safe_link(&dest_url) => Event::Start(pulldown_cmark::Tag::Link {
            link_type,
            dest_url: CowStr::Borrowed(""),
            title,
            id,
        }),
        other => other,
    });
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, events);
    out
}

/// Whether a link may stay in mail: the web's and mail's schemes, never `javascript:` and its
/// like.
fn safe_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

fn validate(subject: &str, body: &str) -> crate::Result<()> {
    if subject.trim().is_empty() || subject.chars().count() > SUBJECT_MAX_CHARS {
        return Err(crate::Error::Validation(t!(
            "newsletterSubjectLength",
            max = SUBJECT_MAX_CHARS
        )));
    }
    if body.trim().is_empty() || body.chars().count() > BODY_MAX_CHARS {
        return Err(crate::Error::Validation(t!(
            "newsletterBodyLength",
            max = BODY_MAX_CHARS
        )));
    }
    Ok(())
}

/// Every post, the newest first.
pub async fn list_posts(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
) -> crate::Result<Vec<NewsletterPost>> {
    access.require(DeploymentPermission::SendNewsletters)?;
    let mut conn = state.connection_pool.get().await?;
    Ok(newsletter_post::table
        .select(NewsletterPost::as_select())
        .order_by(newsletter_post::id.desc())
        .load(conn.as_mut())
        .await?)
}

/// The post `id`, for its mail; `None` once it is deleted.
pub(super) async fn read_post(
    state: &GlobalServerContext,
    id: NewsletterPostId,
) -> crate::Result<Option<NewsletterPost>> {
    let mut conn = state.connection_pool.get().await?;
    Ok(newsletter_post::table
        .select(NewsletterPost::as_select())
        .filter(newsletter_post::id.eq(id))
        .first(conn.as_mut())
        .await
        .optional()?)
}

pub async fn get_post(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: NewsletterPostId,
) -> crate::Result<NewsletterPost> {
    access.require(DeploymentPermission::SendNewsletters)?;
    read_post(state, id)
        .await?
        .ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))
}

/// Writes a draft.
pub async fn create_post(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    subject: &str,
    body: &str,
) -> crate::Result<NewsletterPost> {
    access.require(DeploymentPermission::SendNewsletters)?;
    validate(subject, body)?;
    let mut conn = state.connection_pool.get().await?;
    Ok(diesel::insert_into(newsletter_post::table)
        .values((
            newsletter_post::id.eq(NewsletterPostId::new()),
            newsletter_post::subject.eq(subject.trim()),
            newsletter_post::body.eq(body),
            newsletter_post::author.eq(access.user),
        ))
        .returning(NewsletterPost::as_returning())
        .get_result(conn.as_mut())
        .await?)
}

/// Changes a draft; a post already sent is refused.
pub async fn update_post(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: NewsletterPostId,
    subject: Option<&str>,
    body: Option<&str>,
) -> crate::Result<NewsletterPost> {
    access.require(DeploymentPermission::SendNewsletters)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let post: NewsletterPost = newsletter_post::table
                .select(NewsletterPost::as_select())
                .filter(newsletter_post::id.eq(id))
                .for_update()
                .first(conn)
                .await?;
            if post.sent_at.is_some() {
                return Err(crate::Error::Conflict(t!("newsletterAlreadySent")));
            }
            let subject = subject.map_or(post.subject.as_str(), str::trim);
            let body = body.unwrap_or(&post.body);
            validate(subject, body)?;
            Ok(
                diesel::update(newsletter_post::table.filter(newsletter_post::id.eq(id)))
                    .set((
                        newsletter_post::subject.eq(subject),
                        newsletter_post::body.eq(body),
                        newsletter_post::updated_at.eq(diesel::dsl::now),
                    ))
                    .returning(NewsletterPost::as_returning())
                    .get_result(conn)
                    .await?,
            )
        }
        .scope_boxed()
    })
    .await
}

/// Deletes a draft; a sent post stays in the archive.
pub async fn delete_post(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: NewsletterPostId,
) -> crate::Result<()> {
    access.require(DeploymentPermission::SendNewsletters)?;
    let mut conn = state.connection_pool.get().await?;
    let post: NewsletterPost = newsletter_post::table
        .select(NewsletterPost::as_select())
        .filter(newsletter_post::id.eq(id))
        .first(conn.as_mut())
        .await?;
    if post.sent_at.is_some() {
        return Err(crate::Error::Conflict(t!("newsletterAlreadySent")));
    }
    diesel::delete(
        newsletter_post::table
            .filter(newsletter_post::id.eq(id))
            .filter(newsletter_post::sent_at.is_null()),
    )
    .execute(conn.as_mut())
    .await?;
    Ok(())
}

/// Refuses sending while the deployment has no newsletter or cannot send mail.
fn ensure_enabled(state: &GlobalServerContext) -> crate::Result<()> {
    if !super::available(state) {
        return Err(crate::Error::Validation(t!("emailUnavailable")));
    }
    if !state.settings().newsletter_enabled {
        return Err(crate::Error::Validation(t!("newsletterOff")));
    }
    Ok(())
}

/// Sends the post to its sender alone, at their verified address, as it would read.
pub async fn send_test(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: NewsletterPostId,
) -> crate::Result<()> {
    access.require(DeploymentPermission::SendNewsletters)?;
    if !super::available(state) {
        return Err(crate::Error::Validation(t!("emailUnavailable")));
    }
    let mut conn = state.connection_pool.get().await?;
    let verified: bool = user_email::table
        .select(user_email::verified_at.is_not_null())
        .filter(user_email::user.eq(access.user))
        .first(conn.as_mut())
        .await
        .optional()?
        .unwrap_or(false);
    if !verified {
        return Err(crate::Error::Validation(t!("newsletterTestNeedsAddress")));
    }
    // The post must exist; its mail reads it as it is when sent.
    let _: NewsletterPostId = newsletter_post::table
        .select(newsletter_post::id)
        .filter(newsletter_post::id.eq(id))
        .first(conn.as_mut())
        .await?;
    outbox::queue(
        conn.as_mut(),
        access.user,
        None,
        &Mail::Newsletter {
            post: id,
            test: true,
        },
    )
    .await?;
    super::wake(state).await;
    Ok(())
}

/// Sends the post to every subscriber. It can be sent once, and is fixed from then on.
pub async fn send(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: NewsletterPostId,
) -> crate::Result<NewsletterPost> {
    access.require(DeploymentPermission::SendNewsletters)?;
    ensure_enabled(state)?;
    let mut conn = state.connection_pool.get().await?;
    let sent: Option<NewsletterPost> = diesel::update(
        newsletter_post::table
            .filter(newsletter_post::id.eq(id))
            .filter(newsletter_post::sent_at.is_null()),
    )
    .set((
        newsletter_post::sent_at.eq(diesel::dsl::now),
        newsletter_post::sent_by.eq(access.user),
    ))
    .returning(NewsletterPost::as_returning())
    .get_result(conn.as_mut())
    .await
    .optional()?;
    let Some(sent) = sent else {
        // Gone, or sent already.
        let _: NewsletterPostId = newsletter_post::table
            .select(newsletter_post::id)
            .filter(newsletter_post::id.eq(id))
            .first(conn.as_mut())
            .await?;
        return Err(crate::Error::Conflict(t!("newsletterAlreadySent")));
    };
    super::wake(state).await;
    tracing::info!(post = %id.0, sender = %access.user.0, "a newsletter post is being sent");
    Ok(sent)
}

/// Queues the next batch of subscribers' mail for one post being sent, if any is. Several
/// servers may run it at once: each takes a different post, or waits its turn for the same.
pub(super) async fn queue_some(state: &GlobalServerContext) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let post: Option<(NewsletterPostId, Option<uuid::Uuid>)> = newsletter_post::table
                .select((newsletter_post::id, newsletter_post::queued_through))
                .filter(newsletter_post::sent_at.is_not_null())
                .filter(newsletter_post::queued_at.is_null())
                .order_by(newsletter_post::sent_at)
                .for_update()
                .skip_locked()
                .first(conn)
                .await
                .optional()?;
            let Some((post, through)) = post else {
                return Ok(());
            };
            let mut subscribers = user_email::table
                .inner_join(user::table)
                .select(user_email::user)
                .filter(user_email::newsletter)
                .filter(user_email::verified_at.is_not_null())
                .filter(user::deleted_at.is_null())
                .filter(diesel::dsl::not(crate::user_ban::banned()))
                .order_by(user_email::user)
                .limit(QUEUE_BATCH)
                .into_boxed();
            if let Some(through) = through {
                subscribers = subscribers.filter(user_email::user.gt(UserId(through)));
            }
            let batch: Vec<UserId> = subscribers.load(conn).await?;
            let mail = serde_json::to_value(Mail::Newsletter { post, test: false })?;
            let rows: Vec<_> = batch
                .iter()
                .map(|subscriber| {
                    (
                        email_outbox::id.eq(uuid::Uuid::now_v7()),
                        email_outbox::priority.eq(0i16),
                        email_outbox::user.eq(*subscriber),
                        email_outbox::mail.eq(mail.clone()),
                    )
                })
                .collect();
            if !rows.is_empty() {
                diesel::insert_into(email_outbox::table)
                    .values(rows)
                    .execute(conn)
                    .await?;
            }
            let done = (batch.len() as i64) < QUEUE_BATCH;
            diesel::update(newsletter_post::table.filter(newsletter_post::id.eq(post)))
                .set((
                    newsletter_post::queued_through.eq(batch.last().map(|last| last.0).or(through)),
                    newsletter_post::recipients
                        .eq(newsletter_post::recipients + batch.len() as i64),
                    newsletter_post::queued_at.eq(done.then(Utc::now)),
                ))
                .execute(conn)
                .await?;
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_html_without_letting_html_through() {
        let out = html("# Hello\n\n**bold** <script>alert(1)</script> [x](javascript:alert(1))");
        assert!(out.contains("<h1>Hello</h1>"), "{out}");
        assert!(out.contains("<strong>bold</strong>"));
        assert!(!out.contains("<script>"), "{out}");
        assert!(!out.contains("javascript:"), "{out}");
    }

    #[test]
    fn web_links_stay() {
        assert!(html("[site](https://example.org)").contains("href=\"https://example.org\""));
    }
}
