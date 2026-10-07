//! Email (`app::email`): an account's address and what it receives there at
//! `/users/{user}/email`, the user themself only; resetting a forgotten password at
//! `/auth/password-resets`; the newsletter's posts at `/admin/newsletter`; and the page every
//! unsubscribe link opens, served at `/email/unsubscribe` outside `/api/v1`.

use crate::admin::AdminUser;
use crate::auth::EnrollingSessionUser;
use crate::error::{ApiError, ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path};
use crate::t;
use crate::user::{UserRef, not_your_account};
use crate::{API_PREFIX, TAG_ADMIN, TAG_AUTH, TAG_USERS};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::email::newsletter::{self, NewsletterPost, NewsletterPostId};
use aspen_app::email::outbox::List;
use aspen_app::{UserId, email};
use axum::extract::{Query as AxumQuery, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An account's email address and what it receives there.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailAccount {
    /// `null` while the account has none.
    pub address: Option<String>,
    pub verified: bool,
    /// Whether the profile shows the address, once it is verified (`User.publicEmail`).
    pub shown: bool,
    /// Whether it receives the deployment's newsletter.
    pub newsletter: bool,
    /// Whether it receives a daily digest of what it has not read.
    pub digest: bool,
    /// The IANA time zone, and the hour of the day there (0 to 23), the digest is sent at.
    pub digest_time_zone: String,
    pub digest_hour: u8,
    /// When the next digest is due; `null` while there is none.
    pub digest_next_at: Option<DateTime<Utc>>,
}

impl From<Option<email::EmailAccount>> for EmailAccount {
    fn from(account: Option<email::EmailAccount>) -> Self {
        match account {
            Some(account) => Self {
                verified: account.verified(),
                address: Some(account.address),
                shown: account.shown,
                newsletter: account.newsletter,
                digest: account.digest,
                digest_time_zone: account.digest_time_zone,
                digest_hour: u8::try_from(account.digest_hour).unwrap_or(0),
                digest_next_at: account.digest_next_at,
            },
            None => Self {
                address: None,
                verified: false,
                shown: false,
                newsletter: false,
                digest: false,
                digest_time_zone: "UTC".to_string(),
                digest_hour: 8,
                digest_next_at: None,
            },
        }
    }
}

/// A change to what the account receives and shows. An absent field is unchanged.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmailAccountUpdateRequest {
    pub shown: Option<bool>,
    /// Subscribing needs the deployment to have a newsletter.
    pub newsletter: Option<bool>,
    pub digest: Option<bool>,
    /// An IANA time zone name, such as `Europe/Berlin`.
    pub digest_time_zone: Option<String>,
    /// 0 to 23.
    pub digest_hour: Option<u8>,
}

impl From<EmailAccountUpdateRequest> for email::Preferences {
    fn from(request: EmailAccountUpdateRequest) -> Self {
        Self {
            shown: request.shown,
            newsletter: request.newsletter,
            digest: request.digest,
            digest_time_zone: request.digest_time_zone,
            digest_hour: request.digest_hour,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmailAddressRequest {
    pub address: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmailVerificationRequest {
    /// The six digits mailed to the address.
    pub code: String,
}

/// The caller, refusing anyone else's account.
fn own(session: &EnrollingSessionUser, user: UserRef) -> ApiResult<UserId> {
    let id = user.resolve(&session.0);
    if id != session.0.user.id {
        return Err(not_your_account(app::Error::Unauthorized));
    }
    Ok(id)
}

/// The caller's email address and what they receive there. A session whose address the
/// deployment requires verified may read it.
#[utoipa::path(
    get,
    path = "/users/{user}/email",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = EmailAccount),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "Only the user themself may read it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_email(
    State(state): State<GlobalServerContext>,
    session: EnrollingSessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<Json<EmailAccount>> {
    let id = own(&session, user)?;
    Ok(Json(email::read(&state, id).await?.into()))
}

/// Changes what the caller receives at their address, and whether their profile shows it.
#[utoipa::path(
    patch,
    path = "/users/{user}/email",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    request_body = EmailAccountUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = EmailAccount),
        (status = BAD_REQUEST, description = "`validation`: no address yet, no newsletter to subscribe to, an unknown time zone, or an hour past 23", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "Only the user themself, a person of this deployment, may change it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_email(
    State(state): State<GlobalServerContext>,
    session: crate::auth::SessionUser,
    Path(user): Path<UserRef>,
    Json(request): Json<EmailAccountUpdateRequest>,
) -> ApiResult<Json<EmailAccount>> {
    if user.resolve(&session) != session.user.id {
        return Err(not_your_account(app::Error::Unauthorized));
    }
    let account = email::update_preferences(&state, &session.caller, request.into()).await?;
    Ok(Json(Some(account).into()))
}

/// Gives the caller's account an email address, or changes it, which needs a recently verified
/// session. The address is unverified until the code mailed to it is given; the address it
/// replaces, if verified, is told.
#[utoipa::path(
    put,
    path = "/users/{user}/email/address",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    request_body = EmailAddressRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = EmailAccount),
        (status = BAD_REQUEST, description = "`validation`: not an address, or this deployment sends no mail", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` (someone else's account, a bot's, or a visitor's from another deployment), or `reauthenticationRequired`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn set_email_address(
    State(state): State<GlobalServerContext>,
    session: EnrollingSessionUser,
    Path(user): Path<UserRef>,
    Json(request): Json<EmailAddressRequest>,
) -> ApiResult<Json<EmailAccount>> {
    own(&session, user)?;
    let account = email::set_address(&state, &session.0.caller, &request.address).await?;
    Ok(Json(Some(account).into()))
}

/// Takes the caller's email address away, which needs a recently verified session and is
/// refused while the deployment requires one.
#[utoipa::path(
    delete,
    path = "/users/{user}/email/address",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` (someone else's account, or the deployment requires an address), or `reauthenticationRequired`", body = Problem),
        (status = NOT_FOUND, description = "The account has no address", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_email_address(
    State(state): State<GlobalServerContext>,
    session: EnrollingSessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<NoContent> {
    own(&session, user)?;
    email::remove_address(&state, &session.0.caller).await?;
    Ok(NoContent)
}

/// Mails the caller's address a new verification code, replacing the one before.
#[utoipa::path(
    post,
    path = "/users/{user}/email/verification-codes",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, description = "`validation`: no address, or this deployment sends no mail", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = CONFLICT, description = "The address is verified already", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn resend_email_verification(
    State(state): State<GlobalServerContext>,
    session: EnrollingSessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<NoContent> {
    own(&session, user)?;
    email::resend_verification(&state, &session.0.caller).await?;
    Ok(NoContent)
}

/// Verifies the caller's address with the code mailed to it. Five wrong codes end the code.
#[utoipa::path(
    post,
    path = "/users/{user}/email/verification",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    request_body = EmailVerificationRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = EmailAccount),
        (status = BAD_REQUEST, description = "`validation`: the code expired, or was for an address the account no longer has", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`, or `verificationFailed`: the code is wrong", body = Problem),
        (status = TOO_MANY_REQUESTS, description = "`tooManyAttempts`: send a new code", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn verify_email(
    State(state): State<GlobalServerContext>,
    session: EnrollingSessionUser,
    Path(user): Path<UserRef>,
    Json(request): Json<EmailVerificationRequest>,
) -> ApiResult<Json<EmailAccount>> {
    own(&session, user)?;
    let account = email::verify(&state, &session.0.caller, &request.code).await?;
    Ok(Json(Some(account).into()))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordResetRequest {
    pub username: String,
}

/// A password reset begun.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordReset {
    /// Names the reset in the next steps, for half an hour.
    pub id: String,
    /// The account's address with all but the first three characters before the `@` hidden,
    /// which the person must then type whole.
    pub masked_address: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordResetCodeRequest {
    /// The account's whole email address; case does not matter.
    pub address: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordResetCompletionRequest {
    /// The eight digits mailed to the address.
    pub code: String,
    pub new_password: String,
}

/// Begins resetting a forgotten password: names the account by its username and answers with
/// its verified email address, masked. Unauthenticated.
#[utoipa::path(
    post,
    path = "/auth/password-resets",
    tag = TAG_AUTH,
    request_body = PasswordResetRequest,
    responses(
        (status = CREATED, body = PasswordReset, headers(("Location" = String, description = "URL of the reset"))),
        (status = NOT_FOUND, description = "`passwordResetUnavailable`: no account has that username, it has no verified email address, or this deployment sends no mail; `detail` says which", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn start_password_reset(
    State(state): State<GlobalServerContext>,
    Json(request): Json<PasswordResetRequest>,
) -> ApiResult<Created<PasswordReset>> {
    let started = email::reset::start(&state, request.username.trim()).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/auth/password-resets/{}", started.id),
        PasswordReset {
            id: started.id,
            masked_address: started.masked_address,
        },
    ))
}

/// Mails a reset code to the account's address when the whole address given is that address,
/// answering the same whether or not it is. Asking again sends a new code, which replaces the
/// one before; each reset takes five addresses. Unauthenticated.
#[utoipa::path(
    post,
    path = "/auth/password-resets/{reset}/codes",
    tag = TAG_AUTH,
    params(("reset" = String, Path)),
    request_body = PasswordResetCodeRequest,
    responses(
        (status = NO_CONTENT),
        (status = NOT_FOUND, description = "`passwordResetExpired`: start again", body = Problem),
        (status = TOO_MANY_REQUESTS, description = "`tooManyAttempts`: the reset took its five addresses and ended, or the account has been mailed five codes this hour; start again later", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn send_password_reset_code(
    State(state): State<GlobalServerContext>,
    Path(reset): Path<String>,
    Json(request): Json<PasswordResetCodeRequest>,
) -> ApiResult<NoContent> {
    email::reset::send_code(&state, &reset, &request.address).await?;
    Ok(NoContent)
}

/// Sets a new password with the code mailed for the reset. Every sign-in of the account ends;
/// its second factors stay. Unauthenticated.
#[utoipa::path(
    post,
    path = "/auth/password-resets/{reset}/completion",
    tag = TAG_AUTH,
    params(("reset" = String, Path)),
    request_body = PasswordResetCompletionRequest,
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, description = "`validation`: no code was sent for this reset yet", body = Problem),
        (status = FORBIDDEN, description = "`verificationFailed`: the code is wrong", body = Problem),
        (status = NOT_FOUND, description = "`passwordResetExpired`: start again", body = Problem),
        (status = UNPROCESSABLE_ENTITY, description = "`passwordRequirementsNotMet`", body = Problem),
        (status = TOO_MANY_REQUESTS, description = "`tooManyAttempts`: the reset ended; start again", body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`serverBusy`: too many password checks queued; `Retry-After` says when to try again", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn complete_password_reset(
    State(state): State<GlobalServerContext>,
    Path(reset): Path<String>,
    Json(request): Json<PasswordResetCompletionRequest>,
) -> ApiResult<NoContent> {
    email::reset::complete(&state, &reset, &request.code, &request.new_password).await?;
    Ok(NoContent)
}

/// A newsletter post, as the dashboard shows it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewsletterPostRecord {
    pub id: NewsletterPostId,
    pub subject: String,
    /// Markdown.
    pub body: String,
    /// The body as the mail shows it.
    pub html: String,
    pub author: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When it was sent; `null` while it is a draft.
    pub sent_at: Option<DateTime<Utc>>,
    pub sent_by: Option<UserId>,
    /// When every subscriber's mail had been queued; `null` until then.
    pub queued_at: Option<DateTime<Utc>>,
    /// How many subscribers its mail has been queued for.
    pub recipients: i64,
}

impl From<NewsletterPost> for NewsletterPostRecord {
    fn from(post: NewsletterPost) -> Self {
        Self {
            html: newsletter::html(&post.body),
            id: post.id,
            subject: post.subject,
            body: post.body,
            author: post.author,
            created_at: post.created_at,
            updated_at: post.updated_at,
            sent_at: post.sent_at,
            sent_by: post.sent_by,
            queued_at: post.queued_at,
            recipients: post.recipients,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewsletterPostCreateRequest {
    /// Up to 200 characters.
    pub subject: String,
    /// Markdown, up to 100000 characters.
    pub body: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewsletterPostUpdateRequest {
    pub subject: Option<String>,
    pub body: Option<String>,
}

/// Every newsletter post, the newest first. Takes Send newsletters.
#[utoipa::path(
    get,
    path = "/admin/newsletter/posts",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<NewsletterPostRecord>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_newsletter_posts(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<Vec<NewsletterPostRecord>>> {
    let posts = newsletter::list_posts(&state, &access).await?;
    Ok(Json(posts.into_iter().map(Into::into).collect()))
}

/// Writes a draft. Takes Send newsletters.
#[utoipa::path(
    post,
    path = "/admin/newsletter/posts",
    tag = TAG_ADMIN,
    request_body = NewsletterPostCreateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = NewsletterPostRecord, headers(("Location" = String, description = "URL of the post"))),
        (status = BAD_REQUEST, description = "`validation`: a subject or body empty or too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_newsletter_post(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<NewsletterPostCreateRequest>,
) -> ApiResult<Created<NewsletterPostRecord>> {
    let post = newsletter::create_post(&state, &access, &request.subject, &request.body).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/admin/newsletter/posts/{}", post.id.0),
        post.into(),
    ))
}

#[utoipa::path(
    get,
    path = "/admin/newsletter/posts/{post}",
    tag = TAG_ADMIN,
    params(("post" = NewsletterPostId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = NewsletterPostRecord),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_newsletter_post(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(post): Path<NewsletterPostId>,
) -> ApiResult<Json<NewsletterPostRecord>> {
    Ok(Json(
        newsletter::get_post(&state, &access, post).await?.into(),
    ))
}

/// Changes a draft. Takes Send newsletters.
#[utoipa::path(
    patch,
    path = "/admin/newsletter/posts/{post}",
    tag = TAG_ADMIN,
    params(("post" = NewsletterPostId, Path)),
    request_body = NewsletterPostUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = NewsletterPostRecord),
        (status = BAD_REQUEST, description = "`validation`: a subject or body empty or too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "The post was sent, and is fixed", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_newsletter_post(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(post): Path<NewsletterPostId>,
    Json(request): Json<NewsletterPostUpdateRequest>,
) -> ApiResult<Json<NewsletterPostRecord>> {
    let post = newsletter::update_post(
        &state,
        &access,
        post,
        request.subject.as_deref(),
        request.body.as_deref(),
    )
    .await?;
    Ok(Json(post.into()))
}

/// Deletes a draft. Takes Send newsletters.
#[utoipa::path(
    delete,
    path = "/admin/newsletter/posts/{post}",
    tag = TAG_ADMIN,
    params(("post" = NewsletterPostId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "The post was sent, and stays in the archive", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_newsletter_post(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(post): Path<NewsletterPostId>,
) -> ApiResult<NoContent> {
    newsletter::delete_post(&state, &access, post).await?;
    Ok(NoContent)
}

/// Mails the post to the caller alone, at their verified address, as subscribers would read it.
/// Takes Send newsletters.
#[utoipa::path(
    post,
    path = "/admin/newsletter/posts/{post}/test",
    tag = TAG_ADMIN,
    params(("post" = NewsletterPostId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, description = "`validation`: the caller has no verified address, or this deployment sends no mail", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn test_newsletter_post(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(post): Path<NewsletterPostId>,
) -> ApiResult<NoContent> {
    newsletter::send_test(&state, &access, post).await?;
    Ok(NoContent)
}

/// Sends the post to every subscriber. It is sent once, and fixed from then on; its mail goes
/// out over the following minutes. Takes Send newsletters.
#[utoipa::path(
    post,
    path = "/admin/newsletter/posts/{post}/sending",
    tag = TAG_ADMIN,
    params(("post" = NewsletterPostId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = NewsletterPostRecord),
        (status = BAD_REQUEST, description = "`validation`: the deployment has no newsletter, or sends no mail", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Send newsletters", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "The post was sent already", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn send_newsletter_post(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(post): Path<NewsletterPostId>,
) -> ApiResult<Json<NewsletterPostRecord>> {
    Ok(Json(newsletter::send(&state, &access, post).await?.into()))
}

/// What an unsubscribe link names.
#[derive(Debug, Deserialize)]
pub struct UnsubscribeQuery {
    list: String,
    token: String,
}

/// Inline style only, nothing loaded, a form posting back here, never framed.
const PAGE_POLICY: &str =
    "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'";

/// A small page, in the request's language. `action` adds a button that posts the form back.
fn page(title: &str, message: &str, action: Option<&str>) -> impl IntoResponse + use<> {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let button = action
        .map(|label| {
            format!(
                "<form method=\"post\"><button type=\"submit\" style=\"font:inherit;padding:8px \
                 16px;border-radius:6px;border:1px solid #2f6b3a;background:#2f6b3a;\
                 color:#fff;cursor:pointer\">{}</button></form>",
                escape(label)
            )
        })
        .unwrap_or_default();
    let locale = app::locale::current();
    let html = format!(
        "<!doctype html><html lang=\"{locale}\"><head><meta charset=\"utf-8\"><meta \
         name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta \
         name=\"color-scheme\" content=\"light dark\"><title>{title}</title></head><body \
         style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:0 auto;padding:2rem \
         1rem;line-height:1.5\"><h1 style=\"font-size:1.4rem\">{title}</h1><p>{message}</p>\
         {button}</body></html>",
        title = escape(title),
        message = escape(message),
    );
    (
        [
            (CONTENT_TYPE, "text/html; charset=utf-8"),
            (CACHE_CONTROL, "no-store"),
            (CONTENT_SECURITY_POLICY, PAGE_POLICY),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (REFERRER_POLICY, "no-referrer"),
        ],
        html,
    )
}

fn list_of(query: &UnsubscribeQuery) -> Option<List> {
    query.list.parse::<List>().ok()
}

/// The page an unsubscribe link opens: it asks before unsubscribing, since mail scanners follow
/// links.
pub async fn unsubscribe_page(
    State(state): State<GlobalServerContext>,
    AxumQuery(query): AxumQuery<UnsubscribeQuery>,
) -> impl IntoResponse {
    let deployment = state.settings().name().to_string();
    let Some(list) = list_of(&query) else {
        return page(&t!("unsubscribeTitle"), &t!("unsubscribeUnknown"), None).into_response();
    };
    let question = match list {
        List::Newsletter => t!(
            "unsubscribeNewsletterQuestion",
            deployment = deployment.as_str()
        ),
        List::Digest => t!(
            "unsubscribeDigestQuestion",
            deployment = deployment.as_str()
        ),
    };
    page(
        &t!("unsubscribeTitle"),
        &question,
        Some(&t!("unsubscribeConfirm")),
    )
    .into_response()
}

/// Unsubscribes: the page's button, and the one-click unsubscribe of RFC 8058 that mail
/// programs send to the `List-Unsubscribe` address.
pub async fn unsubscribe(
    State(state): State<GlobalServerContext>,
    AxumQuery(query): AxumQuery<UnsubscribeQuery>,
) -> impl IntoResponse {
    let Some(list) = list_of(&query) else {
        return page(&t!("unsubscribeTitle"), &t!("unsubscribeUnknown"), None).into_response();
    };
    match email::unsubscribe(&state, &query.token, list).await {
        Ok(true) => page(&t!("unsubscribeTitle"), &t!("unsubscribeDone"), None).into_response(),
        Ok(false) => page(&t!("unsubscribeTitle"), &t!("unsubscribeUnknown"), None).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}
