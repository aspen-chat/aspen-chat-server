//! Reports (`app::report`): the categories anyone reports with, reporting a message, a
//! profile, or a nickname, and, under `/admin`, the review of what was reported, which takes Review reports,
//! and the deployment's own categories, which take Manage report categories.

use crate::API_PREFIX;
use crate::TAG_REPORTS;
use crate::admin::{AdminUser, UserBanRequest};
use crate::attachment::Attachment;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path, Query};
use crate::message_enum::{Channel, Community, Message, User};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::report::{
    ContextAnchor, ProfileAspect, ReportCase, ReportCategory, ReportCounts, ReportRequest,
    ReportStatus,
};
use aspen_app::{
    AttachmentId, ChannelId, CommunityId, MessageId, ReportCaseId, ReportCategoryId, ReportId,
    UserId,
};
use axum::extract::State;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};

/// A message as a review reads it: deleted ones too, which no one else reads.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewedMessage {
    pub message: Message,
    /// When it was deleted; `null` while it stands.
    pub deleted_at: Option<DateTime<Utc>>,
    /// The attachments taken off it, which a review alone reads, among the response's
    /// `attachments`; absent when there are none, and for anyone but a reviewer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_attachments: Vec<AttachmentId>,
}

impl From<app::report::ReviewedMessage> for ReviewedMessage {
    fn from(reviewed: app::report::ReviewedMessage) -> Self {
        Self {
            message: Message::from(reviewed.message),
            deleted_at: reviewed.deleted_at,
            removed_attachments: reviewed.removed_attachments,
        }
    }
}

/// A report as its reporter writes it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageReportRequest {
    /// One of the categories `GET /report-categories` offers.
    pub category: ReportCategoryId,
    /// What the reporter would add, at most `app::report::EXPLANATION_MAX_CHARS` characters;
    /// needed with the Other category.
    #[serde(default)]
    pub explanation: Option<String>,
}

/// A report of a profile, naming what on it is objectionable.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProfileReportRequest {
    pub category: ReportCategoryId,
    #[serde(default)]
    pub explanation: Option<String>,
    /// At least one.
    pub aspects: Vec<ProfileAspect>,
}

/// A report of a member's nickname in a community.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NicknameReportRequest {
    pub category: ReportCategoryId,
    #[serde(default)]
    pub explanation: Option<String>,
}

/// A report as it was received.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportReceipt {
    pub id: ReportId,
    pub created_at: DateTime<Utc>,
}

impl From<app::report::Filed> for ReportReceipt {
    fn from(filed: app::report::Filed) -> Self {
        Self {
            id: filed.id,
            created_at: filed.created_at,
        }
    }
}

/// The report categories offered: the built-in ones in the caller's language, then the
/// deployment's own, then Other.
#[utoipa::path(
    get,
    path = "/report-categories",
    tag = TAG_REPORTS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<ReportCategory>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_report_categories(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
) -> ApiResult<Json<Vec<ReportCategory>>> {
    Ok(Json(app::report::categories(&state, false).await?))
}

/// Reports a message the caller can read to the deployment's moderators. An echo is reported
/// as the reply it shows. Nobody reports their own messages or the system account's, and each
/// person reports a message once while its case is unresolved.
#[utoipa::path(
    post,
    path = "/messages/{message}/reports",
    tag = TAG_REPORTS,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = ReportReceipt),
        (status = BAD_REQUEST, description = "`validation`: an unknown or hidden category, an explanation too long or missing for Other, or the caller's own message or the system account's", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such message, or one the caller cannot read", body = Problem),
        (status = CONFLICT, description = "`alreadyReported`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn report_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
    Json(request): Json<MessageReportRequest>,
) -> ApiResult<Created<ReportReceipt>> {
    let filed = app::report::report_message(
        &state,
        user.id,
        message,
        &ReportRequest {
            category: request.category,
            explanation: request.explanation,
        },
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/messages/{}/reports/{}", message.0, filed.id.0),
        filed.into(),
    ))
}

/// Reports a person's profile to the deployment's moderators, naming the aspects found
/// objectionable; the profile is kept as it stands. Nobody reports their own or the system
/// account's, and each person reports a profile once while its case is unresolved.
#[utoipa::path(
    post,
    path = "/users/{user}/reports",
    tag = TAG_REPORTS,
    params(("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = ReportReceipt),
        (status = BAD_REQUEST, description = "`validation`: no aspects, an unknown or hidden category, an explanation too long or missing for Other, or the caller's own profile or the system account's", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`alreadyReported`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn report_profile(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(subject): Path<UserId>,
    Json(request): Json<ProfileReportRequest>,
) -> ApiResult<Created<ReportReceipt>> {
    let filed = app::report::report_profile(
        &state,
        user.id,
        subject,
        &ReportRequest {
            category: request.category,
            explanation: request.explanation,
        },
        request.aspects,
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/users/{}/reports/{}", subject.0, filed.id.0),
        filed.into(),
    ))
}

/// Reports the nickname a member chose in a community the caller belongs to, to the
/// deployment's moderators; the nickname is kept as it stands. Nobody reports their own or the
/// system account's, and each person reports a member's nickname in a community once while its
/// case is unresolved.
#[utoipa::path(
    post,
    path = "/communities/{community}/members/{user}/nickname/reports",
    tag = TAG_REPORTS,
    params(("community" = CommunityId, Path), ("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = ReportReceipt),
        (status = BAD_REQUEST, description = "`validation`: no nickname to report, an unknown or hidden category, an explanation too long or missing for Other, or the caller's own nickname or the system account's", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, or either person is not a member", body = Problem),
        (status = CONFLICT, description = "`alreadyReported`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn report_nickname(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, subject)): Path<(CommunityId, UserId)>,
    Json(request): Json<NicknameReportRequest>,
) -> ApiResult<Created<ReportReceipt>> {
    let filed = app::report::report_nickname(
        &state,
        user.id,
        community,
        subject,
        &ReportRequest {
            category: request.category,
            explanation: request.explanation,
        },
    )
    .await?;
    Ok(Created::new(
        format!(
            "{API_PREFIX}/communities/{}/members/{}/nickname/reports/{}",
            community.0, subject.0, filed.id.0
        ),
        filed.into(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Review

/// Cases with everything they name.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportCaseList {
    pub cases: Vec<ReportCase>,
    /// The messages the cases are about, deleted ones included.
    pub messages: Vec<ReviewedMessage>,
    /// The categories the reports used, hidden ones included.
    pub categories: Vec<ReportCategory>,
    /// Subjects, reporters, reviewers, and authors.
    pub users: Vec<User>,
    /// Where the messages were posted.
    pub channels: Vec<Channel>,
    /// The communities of those channels, and those the nickname cases are about.
    pub communities: Vec<Community>,
    pub attachments: Vec<Attachment>,
}

/// The users, attachments, channels, and communities `messages`, `people`, and `communities`
/// name.
async fn named_by(
    state: &GlobalServerContext,
    viewer: UserId,
    messages: &[ReviewedMessage],
    people: impl IntoIterator<Item = UserId>,
    communities: impl IntoIterator<Item = CommunityId>,
) -> ApiResult<(Vec<User>, Vec<Channel>, Vec<Community>, Vec<Attachment>)> {
    let mut users: HashSet<UserId> = people.into_iter().collect();
    users.extend(messages.iter().map(|m| m.message.author));
    let users: Vec<UserId> = users.into_iter().collect();
    let channel_ids: Vec<ChannelId> = messages
        .iter()
        .map(|m| m.message.channel_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let attachment_ids: Vec<AttachmentId> = messages
        .iter()
        .flat_map(|m| {
            m.message
                .attachments
                .iter()
                .chain(&m.removed_attachments)
                .copied()
        })
        .collect();
    let (users, mut channels, attachments) = tokio::try_join!(
        app::user::read_users(state, viewer, &users),
        app::channel::read_channels(state, &channel_ids),
        app::attachment::evidence::read_for_review(state, &attachment_ids),
    )?;
    // A thread's parent says where the thread is.
    let parents: Vec<ChannelId> = channels
        .iter()
        .filter_map(|c| c.parent_channel)
        .filter(|parent| !channel_ids.contains(parent))
        .collect();
    channels.extend(app::channel::read_channels(state, &parents).await?);
    let community_ids: Vec<CommunityId> = channels
        .iter()
        .filter_map(|c| c.community)
        .chain(communities)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let communities = app::community::read_communities(state, &community_ids).await?;
    let mut reviewed = Vec::with_capacity(attachments.len());
    for row in attachments {
        reviewed.push(crate::attachment::attachment_for_review(state, row).await?);
    }
    Ok((
        users.into_iter().map(User::from).collect(),
        channels,
        communities.into_iter().map(Community::from).collect(),
        reviewed,
    ))
}

impl ReportCaseList {
    async fn of(
        state: &GlobalServerContext,
        viewer: UserId,
        page: app::report::CasePage,
    ) -> ApiResult<Self> {
        let people = page.people();
        let case_communities = page.communities();
        let messages: Vec<ReviewedMessage> = page
            .messages
            .into_iter()
            .map(ReviewedMessage::from)
            .collect();
        let (users, channels, communities, attachments) =
            named_by(state, viewer, &messages, people, case_communities).await?;
        Ok(Self {
            cases: page.cases,
            messages,
            categories: page.categories,
            users,
            channels,
            communities,
            attachments,
        })
    }
}

/// A page of the cases list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct ReportCaseQuery {
    /// Which cases: `open` (the default), `resolved`, or `dismissed`.
    #[serde(rename = "filter[status]")]
    #[param(rename = "filter[status]", value_type = Option<ReportStatus>)]
    pub status: Option<ReportStatus>,
    /// How many to skip, at most 100,000.
    pub offset: Option<i64>,
    /// How many to return, at most 100; 15 when absent.
    pub limit: Option<i64>,
}

/// The cases in one state: open and dismissed ones most recently reported first, resolved ones
/// most recently resolved first. Takes Review reports.
#[utoipa::path(
    get,
    path = "/admin/reports",
    tag = TAG_REPORTS,
    params(ReportCaseQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportCaseList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Review reports", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_report_cases(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<ReportCaseQuery>,
) -> ApiResult<Json<ReportCaseList>> {
    let page = app::report::list_cases(
        &state,
        &access,
        query.status.unwrap_or(ReportStatus::Open),
        query.offset.unwrap_or(0),
        query.limit.unwrap_or(15),
    )
    .await?;
    Ok(Json(ReportCaseList::of(&state, access.user, page).await?))
}

/// How many cases are open, and how many dismissed. Takes Review reports.
#[utoipa::path(
    get,
    path = "/admin/report-counts",
    tag = TAG_REPORTS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportCounts),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Review reports", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_report_counts(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<ReportCounts>> {
    Ok(Json(app::report::counts(&state, &access).await?))
}

/// One case. Takes Review reports.
#[utoipa::path(
    get,
    path = "/admin/reports/{case}",
    tag = TAG_REPORTS,
    params(("case" = ReportCaseId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportCaseList),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Review reports", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_report_case(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(case): Path<ReportCaseId>,
) -> ApiResult<Json<ReportCaseList>> {
    let page = app::report::read_case(&state, &access, case).await?;
    Ok(Json(ReportCaseList::of(&state, access.user, page).await?))
}

/// Where a context window starts: around the reported message when neither is given.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct ReportContextQuery {
    /// The messages before this one.
    pub before: Option<MessageId>,
    /// The messages after this one.
    pub after: Option<MessageId>,
}

/// Messages around a reported one, oldest first.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportContext {
    /// The channel, thread, or DM they are in, among `channels`.
    pub channel: ChannelId,
    pub messages: Vec<ReviewedMessage>,
    /// Whether there are older messages than these, and newer.
    pub more_before: bool,
    pub more_after: bool,
    pub users: Vec<User>,
    pub channels: Vec<Channel>,
    pub communities: Vec<Community>,
    pub attachments: Vec<Attachment>,
}

/// The messages around a case's reported message in its channel, thread, or DM, deleted ones
/// included, `app::report::CONTEXT_MESSAGES` either side at a time. Reading a DM's is written
/// to the moderation log. Takes Review reports.
#[utoipa::path(
    get,
    path = "/admin/reports/{case}/context",
    tag = TAG_REPORTS,
    params(("case" = ReportCaseId, Path), ReportContextQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportContext),
        (status = BAD_REQUEST, description = "`validation`: a profile case, or both `before` and `after`", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Review reports", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_report_context(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(case): Path<ReportCaseId>,
    Query(query): Query<ReportContextQuery>,
) -> ApiResult<Json<ReportContext>> {
    let anchor = match (query.before, query.after) {
        (None, None) => ContextAnchor::Around,
        (Some(before), None) => ContextAnchor::Before(before),
        (None, Some(after)) => ContextAnchor::After(after),
        (Some(_), Some(_)) => {
            return Err(app::Error::Validation(crate::t!("messageWindowMultipleAnchors")).into());
        }
    };
    let window = app::report::case_context(&state, &access, case, anchor).await?;
    let messages: Vec<ReviewedMessage> = window
        .messages
        .into_iter()
        .map(ReviewedMessage::from)
        .collect();
    let (users, channels, communities, attachments) = named_by(
        &state,
        access.user,
        &messages,
        std::iter::empty(),
        std::iter::empty(),
    )
    .await?;
    Ok(Json(ReportContext {
        channel: window.channel,
        messages,
        more_before: window.more_before,
        more_after: window.more_after,
        users,
        channels,
        communities,
        attachments,
    }))
}

/// What a review does about a case; at least one action.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportResolutionRequest {
    /// A warning in the reviewer's own words, sent to the subject by the system account for the
    /// deployment's moderators, at most `app::report::WARNING_MAX_CHARS` characters. Review
    /// reports allows it.
    #[serde(default)]
    pub warn: Option<String>,
    /// A ban from the deployment, as `PUT /admin/users/{user}/ban` takes it. Takes Ban users.
    #[serde(default)]
    pub ban: Option<UserBanRequest>,
    /// For a message case, delete the message. Takes Remove content.
    #[serde(default)]
    pub delete_message: bool,
    /// For a profile case of this deployment's user, the aspects to reset: each is cleared,
    /// and a username replaced with a placeholder they are told to change. Takes Remove
    /// content.
    #[serde(default)]
    pub reset: Vec<ProfileAspect>,
    /// For a nickname case, clear the nickname, whatever it is now. Takes Remove content.
    #[serde(default)]
    pub clear_nickname: bool,
}

/// Resolves an open case with the actions asked for, each taking its own permission; the case
/// stays resolved for good. A case about the reviewer, or about someone whose highest deployment
/// role is not below theirs, is not theirs to act on. If an action fails, the case is open
/// again and those before it stand. Takes Review reports.
#[utoipa::path(
    post,
    path = "/admin/reports/{case}/resolution",
    tag = TAG_REPORTS,
    params(("case" = ReportCaseId, Path)),
    request_body = ReportResolutionRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportCaseList),
        (status = BAD_REQUEST, description = "`validation`: no action, a warning too long or empty, an action the case's kind does not take, or a ban's reason, duration, or deletion window", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden`: a permission an action needs, or a case not the caller's to act on", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`conflict`: the case is no longer open", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn resolve_report_case(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(case): Path<ReportCaseId>,
    Json(request): Json<ReportResolutionRequest>,
) -> ApiResult<Json<ReportCaseList>> {
    let page = app::report::resolve(
        &state,
        &access,
        case,
        app::report::Actions {
            warn: request.warn,
            ban: request.ban.map(Into::into),
            delete_message: request.delete_message,
            reset: request.reset,
            clear_nickname: request.clear_nickname,
        },
    )
    .await?;
    Ok(Json(ReportCaseList::of(&state, access.user, page).await?))
}

/// Dismisses an open case: it leaves review but is kept, and comes back if restored or reported
/// again. Takes Review reports, and a case the caller may act on.
#[utoipa::path(
    put,
    path = "/admin/reports/{case}/dismissal",
    tag = TAG_REPORTS,
    params(("case" = ReportCaseId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportCaseList),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden`: without Review reports, or a case not the caller's to act on", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`conflict`: the case was resolved", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn dismiss_report_case(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(case): Path<ReportCaseId>,
) -> ApiResult<Json<ReportCaseList>> {
    let page = app::report::dismiss(&state, &access, case).await?;
    Ok(Json(ReportCaseList::of(&state, access.user, page).await?))
}

/// Restores a dismissed case to review. Takes Review reports, and a case the caller may act on.
#[utoipa::path(
    delete,
    path = "/admin/reports/{case}/dismissal",
    tag = TAG_REPORTS,
    params(("case" = ReportCaseId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden`: without Review reports, or a case not the caller's to act on", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`conflict`: the case was resolved", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn restore_report_case(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(case): Path<ReportCaseId>,
) -> ApiResult<NoContent> {
    app::report::restore(&state, &access, case).await?;
    Ok(NoContent)
}

// ---------------------------------------------------------------------------------------------
// Categories

/// Every report category, hidden ones included, in the order they are offered. Takes Manage
/// report categories or Review reports.
#[utoipa::path(
    get,
    path = "/admin/report-categories",
    tag = TAG_REPORTS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<ReportCategory>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without either permission", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_all_report_categories(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<Vec<ReportCategory>>> {
    use app::deployment::DeploymentPermission;
    if !access.has(DeploymentPermission::ReviewReports) {
        access.require(DeploymentPermission::ManageReportCategories)?;
    }
    Ok(Json(app::report::categories(&state, true).await?))
}

/// A category of the deployment's own.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportCategoryCreateRequest {
    /// 1 to `app::report::CATEGORY_NAME_MAX_CHARS` characters, in any language.
    pub name: String,
    /// What it covers, shown beneath its name; at most
    /// `app::report::CATEGORY_DESCRIPTION_MAX_CHARS` characters.
    #[serde(default)]
    pub description: Option<String>,
}

/// Adds a category of the deployment's own, offered after the others it added and before
/// Other. Takes Manage report categories.
#[utoipa::path(
    post,
    path = "/admin/report-categories",
    tag = TAG_REPORTS,
    request_body = ReportCategoryCreateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = ReportCategory),
        (status = BAD_REQUEST, description = "`validation`: the name or description's length", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage report categories", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_report_category(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<ReportCategoryCreateRequest>,
) -> ApiResult<Created<ReportCategory>> {
    let category = app::report::create_category(
        &state,
        &access,
        &request.name,
        request.description.as_deref(),
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/admin/report-categories/{}", category.id.0),
        category,
    ))
}

/// A change to a category, as a merge patch.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportCategoryUpdateRequest {
    /// A category of the deployment's own only.
    #[serde(default)]
    pub name: Option<String>,
    /// A category of the deployment's own only; `null` clears it.
    #[serde(default, deserialize_with = "crate::extract::double_option")]
    #[schema(nullable)]
    pub description: Option<Option<String>>,
    /// Whether it is no longer offered; reports made with it keep it. Other is always offered.
    #[serde(default)]
    pub hidden: Option<bool>,
}

/// Renames, describes, hides, or shows a category. A built-in one may only be hidden or shown.
/// Takes Manage report categories.
#[utoipa::path(
    patch,
    path = "/admin/report-categories/{category}",
    tag = TAG_REPORTS,
    params(("category" = ReportCategoryId, Path)),
    request_body = ReportCategoryUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReportCategory),
        (status = BAD_REQUEST, description = "`validation`: the name or description's length, renaming a built-in category, or hiding Other", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage report categories", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_report_category(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(category): Path<ReportCategoryId>,
    Json(request): Json<ReportCategoryUpdateRequest>,
) -> ApiResult<Json<ReportCategory>> {
    Ok(Json(
        app::report::update_category(
            &state,
            &access,
            category,
            request.name.as_deref(),
            request.description.as_ref().map(|d| d.as_deref()),
            request.hidden,
        )
        .await?,
    ))
}

/// The order of the deployment's own categories.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportCategoryOrder {
    /// Each of the deployment's own categories, once, in the order they are offered.
    pub categories: Vec<ReportCategoryId>,
}

/// Puts the deployment's own categories in order. Takes Manage report categories.
#[utoipa::path(
    put,
    path = "/admin/report-category-order",
    tag = TAG_REPORTS,
    request_body = ReportCategoryOrder,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, description = "`validation`: not each of the deployment's own categories once", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage report categories", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn order_report_categories(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<ReportCategoryOrder>,
) -> ApiResult<NoContent> {
    app::report::order_categories(&state, &access, &request.categories).await?;
    Ok(NoContent)
}
