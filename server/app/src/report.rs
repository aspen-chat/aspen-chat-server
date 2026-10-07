//! Reports of objectionable messages, profiles, and nicknames, and their review.
//!
//! Anyone may report a message they can read, a person's profile, or the nickname a fellow
//! member chose in a community, choosing what is wrong from the report categories
//! (`report_category`): the built-in ones, and those the deployment added with Manage report
//! categories, which may also hide any of them but Other. A profile report names the aspects it
//! finds objectionable and keeps the profile as it stood, picture included; a nickname report
//! keeps the nickname. A person reports a thing once; nobody reports their own messages,
//! profile, or nickname, or the system account's.
//!
//! Reports of one message, of one person's profile, or of one member's nickname in one
//! community gather in a case (`report_case`), of which at most one is unresolved at a time: `open` until a reviewer acts on it, or
//! `dismissed`, hidden but kept, until it is restored or reported again, either of which opens
//! it again. Holders of Review reports read the cases, and the messages around a reported
//! message (logged once for a DM's). Acting on an open case resolves it for good, with any of:
//! a warning, sent for the deployment's moderators by the system account
//! (`MessageKind::Warning`, which Review reports allows, so every reviewer can act); deleting
//! the reported message, clearing the reported nickname, or resetting the reported aspects of a
//! profile (each of which takes Remove content); and a ban from the deployment (`app::user_ban`,
//! which takes Ban users). Nobody acts on a case about themselves, or about someone whose
//! highest deployment role is not below theirs. Reports and cases are never deleted
//! through the API. Every change to what awaits review is announced to each holder of Review
//! reports as `reportsChanged`.

use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission, DeploymentPermissions};
use crate::events::{ChannelHome, channel_home, dm_recipients};
use crate::message::{Message, MessageKind, MessageWithRelations, Posting, with_relations};
use crate::moderation_log::{ModerationAction, log_moderation};
use crate::t;
use crate::user::UserPg;
use crate::user_ban::{UserBanRequest, banned};
use crate::{
    AttachmentId, ChannelId, CommunityId, EventScope, MessageId, ReportCaseId, ReportCategoryId,
    ReportId, UserId, publish_event,
};
use aspen_schema::{
    community, community_user, deployment_role, message, report, report_case, report_category,
    user, user_deployment_role,
};
use aspen_wire::message_enum::request::UserUpdateRequest;
use aspen_wire::message_enum::server_event::ServerEvent;
pub use aspen_wire::report::{NicknameSnapshot, ProfileAspect, ProfileSnapshot, Warning};
use aspen_wire::user::CustomStatus;
use chrono::{DateTime, Utc};
use diesel::deserialize::FromSql;
use diesel::pg::{Pg, PgValue};
use diesel::prelude::*;
use diesel::serialize::{Output, ToSql};
use diesel::sql_types::{Array, Nullable, Text};
use diesel::{AsExpression, FromSqlRow};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

/// How long a report's explanation may be, in characters.
pub const EXPLANATION_MAX_CHARS: usize = 1000;
/// How long a warning may be, in characters.
pub const WARNING_MAX_CHARS: usize = 2000;
/// How long a category's name and description may be, in characters.
pub const CATEGORY_NAME_MAX_CHARS: usize = 64;
pub const CATEGORY_DESCRIPTION_MAX_CHARS: usize = 200;
/// How many messages either side of a reported one its context shows at a time.
pub const CONTEXT_MESSAGES: i64 = 25;
/// How far a case's context reaches from the reported message either way, however it is
/// paged: four pages of [`CONTEXT_MESSAGES`].
pub const CONTEXT_REACH: i64 = 4 * CONTEXT_MESSAGES;
/// The most cases one page lists.
pub const MAX_PAGE: i64 = 100;

// ---------------------------------------------------------------------------------------------
// Types

/// The aspects a report names, stored as a `TEXT[]` of their wire names; a name this version
/// does not know is left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromSqlRow, AsExpression)]
#[diesel(sql_type = Array<Nullable<Text>>)]
pub struct ProfileAspects(pub Vec<ProfileAspect>);

impl FromSql<Array<Nullable<Text>>, Pg> for ProfileAspects {
    fn from_sql(value: PgValue<'_>) -> diesel::deserialize::Result<Self> {
        let names = <Vec<Option<String>> as FromSql<Array<Nullable<Text>>, Pg>>::from_sql(value)?;
        Ok(Self(
            names
                .into_iter()
                .flatten()
                .filter_map(|name| name.parse().ok())
                .collect(),
        ))
    }
}

impl ToSql<Array<Nullable<Text>>, Pg> for ProfileAspects {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        let names: Vec<Option<String>> = self.0.iter().map(|a| Some(a.to_string())).collect();
        <Vec<Option<String>> as ToSql<Array<Nullable<Text>>, Pg>>::to_sql(
            &names,
            &mut out.reborrow(),
        )
    }
}

impl From<&UserPg> for ProfileSnapshot {
    fn from(user: &UserPg) -> Self {
        Self {
            name: user.name.clone(),
            display_name: user.display_name.clone(),
            icon: user.icon.as_ref().map(|icon| *icon.id()),
            status: user.status_text.clone().map(|text| CustomStatus {
                text,
                emoji: user.status_emoji.clone(),
            }),
            bio: user.bio.clone(),
            pronouns: user.pronouns.clone(),
        }
    }
}

/// What a case is about.
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
pub enum ReportKind {
    Message,
    Profile,
    /// A member's nickname in one community.
    Nickname,
}

crate::wire_name_traits!(ReportKind);
crate::text_sql_traits!(ReportKind);

/// Where a case stands.
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
pub enum ReportStatus {
    /// Awaiting review.
    Open,
    /// Acted on, for good.
    Resolved,
    /// Hidden without action, until restored or reported again.
    Dismissed,
}

crate::wire_name_traits!(ReportStatus);
crate::text_sql_traits!(ReportStatus);

/// The categories every deployment has, in the order they are offered; Other comes last,
/// after the deployment's own.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
    strum::VariantArray,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = Text)]
pub enum BuiltinCategory {
    Spam,
    Harassment,
    HateSpeech,
    Violence,
    SelfHarm,
    IllegalContent,
    Impersonation,
    /// Something no other category names; a report in it must explain itself.
    Other,
}

crate::wire_name_traits!(BuiltinCategory);
crate::text_sql_traits!(BuiltinCategory);

impl BuiltinCategory {
    fn name(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Spam => t!("reportCategorySpam"),
            Self::Harassment => t!("reportCategoryHarassment"),
            Self::HateSpeech => t!("reportCategoryHateSpeech"),
            Self::Violence => t!("reportCategoryViolence"),
            Self::SelfHarm => t!("reportCategorySelfHarm"),
            Self::IllegalContent => t!("reportCategoryIllegalContent"),
            Self::Impersonation => t!("reportCategoryImpersonation"),
            Self::Other => t!("reportCategoryOther"),
        }
    }

    fn description(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Spam => t!("reportCategorySpamDescription"),
            Self::Harassment => t!("reportCategoryHarassmentDescription"),
            Self::HateSpeech => t!("reportCategoryHateSpeechDescription"),
            Self::Violence => t!("reportCategoryViolenceDescription"),
            Self::SelfHarm => t!("reportCategorySelfHarmDescription"),
            Self::IllegalContent => t!("reportCategoryIllegalContentDescription"),
            Self::Impersonation => t!("reportCategoryImpersonationDescription"),
            Self::Other => t!("reportCategoryOtherDescription"),
        }
    }

    /// Where it is offered among the built-in categories.
    fn rank(self) -> usize {
        <Self as strum::VariantArray>::VARIANTS
            .iter()
            .position(|c| *c == self)
            .unwrap_or(usize::MAX)
    }
}

/// A report category as offered: a built-in one by its name in the reader's language, or the
/// deployment's own by the name it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportCategory {
    pub id: ReportCategoryId,
    /// Which built-in category this is; `null` for one the deployment added.
    pub builtin: Option<BuiltinCategory>,
    pub name: String,
    pub description: Option<String>,
    /// Whether it is no longer offered; reports made with it keep it.
    pub hidden: bool,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = report_category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct CategoryRow {
    id: ReportCategoryId,
    builtin: Option<BuiltinCategory>,
    name: Option<String>,
    description: Option<String>,
    position: i32,
    hidden: bool,
    created_at: DateTime<Utc>,
}

impl From<&CategoryRow> for ReportCategory {
    fn from(row: &CategoryRow) -> Self {
        Self {
            id: row.id,
            builtin: row.builtin,
            name: match row.builtin {
                Some(builtin) => builtin.name().into_owned(),
                None => row.name.clone().unwrap_or_default(),
            },
            description: match row.builtin {
                Some(builtin) => Some(builtin.description().into_owned()),
                None => row.description.clone(),
            },
            hidden: row.hidden,
        }
    }
}

/// What reviewing a case did.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = diesel::sql_types::Jsonb)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    /// The warning's words, and the message that carries them.
    pub warning: Option<String>,
    pub warning_message: Option<MessageId>,
    pub ban: Option<BanGiven>,
    /// Whether the reported message was deleted.
    #[serde(default)]
    pub deleted_message: bool,
    /// The aspects of the profile reset.
    #[serde(default)]
    pub reset: Vec<ProfileAspect>,
    /// Whether the reported nickname was cleared.
    #[serde(default)]
    pub cleared_nickname: bool,
}

crate::jsonb_sql_traits!(Resolution);

/// A ban given in reviewing a case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BanGiven {
    pub reason: Option<String>,
    pub duration_seconds: Option<u32>,
    pub delete_messages_seconds: Option<u32>,
    #[serde(default)]
    pub with_owner: bool,
}

/// One person's report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: ReportId,
    pub reporter: UserId,
    pub category: ReportCategoryId,
    pub explanation: Option<String>,
    /// For a profile report, what it finds objectionable, and the profile as it stood.
    pub aspects: Vec<ProfileAspect>,
    pub profile: Option<ProfileSnapshot>,
    /// For a nickname report, the nickname as it stood.
    pub nickname: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A case as a reviewer reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportCase {
    pub id: ReportCaseId,
    pub kind: ReportKind,
    pub status: ReportStatus,
    /// Whose message or profile it is.
    pub subject: UserId,
    /// For a message case, the message reported.
    pub message: Option<MessageId>,
    /// For a nickname case, the community the nickname was chosen in.
    pub community: Option<CommunityId>,
    /// Its reports, oldest first.
    pub reports: Vec<Report>,
    pub opened_at: DateTime<Utc>,
    pub last_reported_at: DateTime<Utc>,
    /// When it was resolved or dismissed, and by whom.
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_by: Option<UserId>,
    pub resolution: Option<Resolution>,
    /// Whether the reader may act on it: it is not about them, nor about someone whose highest
    /// deployment role is not below theirs.
    pub may_act: bool,
    /// Whether a ban from the deployment stands against its subject now.
    pub subject_banned: bool,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = report_case)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct CaseRow {
    id: ReportCaseId,
    kind: ReportKind,
    subject: UserId,
    message: Option<MessageId>,
    community: Option<CommunityId>,
    status: ReportStatus,
    opened_at: DateTime<Utc>,
    last_reported_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    closed_by: Option<UserId>,
    resolution: Option<Resolution>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = report)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct ReportRow {
    id: ReportId,
    case: ReportCaseId,
    reporter: UserId,
    category: ReportCategoryId,
    explanation: Option<String>,
    aspects: ProfileAspects,
    profile: Option<ProfileSnapshot>,
    nickname: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<ReportRow> for Report {
    fn from(row: ReportRow) -> Self {
        Self {
            id: row.id,
            reporter: row.reporter,
            category: row.category,
            explanation: row.explanation,
            aspects: row.aspects.0,
            profile: row.profile,
            nickname: row.nickname,
            created_at: row.created_at,
        }
    }
}

/// A message a case is about, or one around it, deleted or not.
pub struct ReviewedMessage {
    pub message: MessageWithRelations,
    pub deleted_at: Option<DateTime<Utc>>,
    /// The attachments taken off it, kept as evidence (`attachment::evidence`).
    pub removed_attachments: Vec<AttachmentId>,
}

/// Cases with what they name: their messages, and every report category their reports used.
pub struct CasePage {
    pub cases: Vec<ReportCase>,
    pub messages: Vec<ReviewedMessage>,
    pub categories: Vec<ReportCategory>,
}

impl CasePage {
    /// Everyone the page names: subjects, reporters, reviewers, and authors.
    pub fn people(&self) -> Vec<UserId> {
        let mut people: HashSet<UserId> = HashSet::new();
        for case in &self.cases {
            people.insert(case.subject);
            people.extend(case.closed_by);
            people.extend(case.reports.iter().map(|r| r.reporter));
        }
        people.extend(self.messages.iter().map(|m| *m.message.message.author.id()));
        people.into_iter().collect()
    }

    /// The communities the page's nickname cases are about.
    pub fn communities(&self) -> Vec<CommunityId> {
        let communities: HashSet<CommunityId> =
            self.cases.iter().filter_map(|c| c.community).collect();
        communities.into_iter().collect()
    }
}

/// How many cases await review, and how many are dismissed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportCounts {
    pub open: i64,
    pub dismissed: i64,
}

// ---------------------------------------------------------------------------------------------
// Categories

/// The report categories, built-in ones first in their order, then the deployment's own in
/// theirs, and Other last; hidden ones only with `with_hidden`.
pub async fn categories(
    state: &GlobalServerContext,
    with_hidden: bool,
) -> crate::Result<Vec<ReportCategory>> {
    let mut conn = state.connection_pool.get().await?;
    let mut query = report_category::table
        .select(CategoryRow::as_select())
        .into_boxed();
    if !with_hidden {
        query = query.filter(report_category::hidden.eq(false));
    }
    let mut rows: Vec<CategoryRow> = query.load(conn.as_mut()).await?;
    rows.sort_by_key(|row| match row.builtin {
        Some(BuiltinCategory::Other) => (2, 0, 0, row.created_at),
        Some(builtin) => (0, builtin.rank(), 0, row.created_at),
        None => (1, 0, row.position, row.created_at),
    });
    Ok(rows.iter().map(ReportCategory::from).collect())
}

fn checked_category_text(
    name: Option<&str>,
    description: Option<Option<&str>>,
) -> crate::Result<(Option<String>, Option<Option<String>>)> {
    let name = match name.map(str::trim) {
        Some(name) if name.is_empty() || name.chars().count() > CATEGORY_NAME_MAX_CHARS => {
            return Err(crate::Error::Validation(t!(
                "reportCategoryNameLength",
                max = CATEGORY_NAME_MAX_CHARS
            )));
        }
        name => name.map(str::to_string),
    };
    let description = match description {
        Some(Some(text)) => {
            let text = text.trim();
            if text.chars().count() > CATEGORY_DESCRIPTION_MAX_CHARS {
                return Err(crate::Error::Validation(t!(
                    "reportCategoryDescriptionLength",
                    max = CATEGORY_DESCRIPTION_MAX_CHARS
                )));
            }
            Some((!text.is_empty()).then(|| text.to_string()))
        }
        Some(None) => Some(None),
        None => None,
    };
    Ok((name, description))
}

/// Adds a category of the deployment's own, after the others it added. Takes Manage report
/// categories.
pub async fn create_category(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    name: &str,
    description: Option<&str>,
) -> crate::Result<ReportCategory> {
    access.require(DeploymentPermission::ManageReportCategories)?;
    let (name, description) = checked_category_text(Some(name), Some(description))?;
    let mut conn = state.connection_pool.get().await?;
    let last: Option<i32> = report_category::table
        .select(diesel::dsl::max(report_category::position))
        .filter(report_category::builtin.is_null())
        .first(conn.as_mut())
        .await?;
    let row: CategoryRow = diesel::insert_into(report_category::table)
        .values((
            report_category::id.eq(ReportCategoryId::new()),
            report_category::name.eq(name),
            report_category::description.eq(description.flatten()),
            report_category::position.eq(last.map_or(0, |last| last + 1)),
        ))
        .returning(CategoryRow::as_returning())
        .get_result(conn.as_mut())
        .await?;
    Ok(ReportCategory::from(&row))
}

/// Renames, describes, hides, or shows a category. A built-in category's name and description
/// are fixed, and Other is never hidden. Takes Manage report categories.
pub async fn update_category(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: ReportCategoryId,
    name: Option<&str>,
    description: Option<Option<&str>>,
    hidden: Option<bool>,
) -> crate::Result<ReportCategory> {
    access.require(DeploymentPermission::ManageReportCategories)?;
    let (name, description) = checked_category_text(name, description)?;
    let mut conn = state.connection_pool.get().await?;
    let builtin: Option<BuiltinCategory> = report_category::table
        .select(report_category::builtin)
        .find(id)
        .first(conn.as_mut())
        .await?;
    if builtin.is_some() && (name.is_some() || description.is_some()) {
        return Err(crate::Error::Validation(t!("reportCategoryBuiltinFixed")));
    }
    if builtin == Some(BuiltinCategory::Other) && hidden == Some(true) {
        return Err(crate::Error::Validation(t!("reportCategoryOtherShown")));
    }
    #[derive(AsChangeset)]
    #[diesel(table_name = report_category)]
    struct Change {
        name: Option<String>,
        description: Option<Option<String>>,
        hidden: Option<bool>,
    }
    let change = Change {
        name,
        description,
        hidden,
    };
    let row: CategoryRow =
        if change.name.is_none() && change.description.is_none() && change.hidden.is_none() {
            report_category::table
                .select(CategoryRow::as_select())
                .find(id)
                .first(conn.as_mut())
                .await?
        } else {
            diesel::update(report_category::table.find(id))
                .set(change)
                .returning(CategoryRow::as_returning())
                .get_result(conn.as_mut())
                .await?
        };
    Ok(ReportCategory::from(&row))
}

/// Puts the deployment's own categories in the order given, which must name each of them once.
/// Takes Manage report categories.
pub async fn order_categories(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    order: &[ReportCategoryId],
) -> crate::Result<()> {
    access.require(DeploymentPermission::ManageReportCategories)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let own: HashSet<ReportCategoryId> = report_category::table
                .select(report_category::id)
                .filter(report_category::builtin.is_null())
                .for_update()
                .load::<ReportCategoryId>(conn.as_mut())
                .await?
                .into_iter()
                .collect();
            let given: HashSet<ReportCategoryId> = order.iter().copied().collect();
            if given.len() != order.len() || given != own {
                return Err(crate::Error::Validation(t!("reportCategoryOrder")));
            }
            for (position, id) in order.iter().enumerate() {
                diesel::update(report_category::table.find(*id))
                    .set(report_category::position.eq(position as i32))
                    .execute(conn.as_mut())
                    .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Reporting

/// A report as its reporter writes it.
#[derive(Debug, Clone)]
pub struct ReportRequest {
    pub category: ReportCategoryId,
    pub explanation: Option<String>,
}

/// What a report was filed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filed {
    pub id: ReportId,
    pub created_at: DateTime<Utc>,
}

/// Checks a report's category, which must be offered, and its explanation, which Other needs.
async fn checked_request(
    conn: &mut AsyncPgConnection,
    request: &ReportRequest,
) -> crate::Result<Option<String>> {
    let builtin: Option<Option<BuiltinCategory>> = report_category::table
        .select(report_category::builtin)
        .filter(report_category::id.eq(request.category))
        .filter(report_category::hidden.eq(false))
        .first(conn)
        .await
        .optional()?;
    let Some(builtin) = builtin else {
        return Err(crate::Error::Validation(t!("reportCategoryUnknown")));
    };
    let explanation = request
        .explanation
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if explanation.is_some_and(|text| text.chars().count() > EXPLANATION_MAX_CHARS) {
        return Err(crate::Error::Validation(t!(
            "reportExplanationLength",
            max = EXPLANATION_MAX_CHARS
        )));
    }
    if explanation.is_none() && builtin == Some(BuiltinCategory::Other) {
        return Err(crate::Error::Validation(t!("reportExplanationRequired")));
    }
    Ok(explanation.map(str::to_string))
}

/// Refuses a report of `subject` by `reporter`: of themselves, or of the system account.
async fn check_subject(
    conn: &mut AsyncPgConnection,
    reporter: UserId,
    subject: UserId,
) -> crate::Result<()> {
    if reporter == subject {
        return Err(crate::Error::Validation(t!("reportOwn")));
    }
    let system: bool = user::table
        .select(user::system)
        .filter(user::id.eq(subject).and(user::deleted_at.is_null()))
        .first(conn)
        .await?;
    if system {
        return Err(crate::Error::Validation(t!("reportSystemAccount")));
    }
    Ok(())
}

/// What a report is about, and what it keeps of it.
enum Reported {
    Message(MessageId),
    Profile {
        aspects: Vec<ProfileAspect>,
        profile: ProfileSnapshot,
    },
    Nickname {
        community: CommunityId,
        nickname: String,
    },
}

/// Adds a report to the unresolved case about what is `reported` of `subject`, opening one or
/// reopening a dismissed one, inside the caller's transaction.
async fn file(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    subject: UserId,
    reported: Reported,
    reporter: UserId,
    request: &ReportRequest,
    explanation: Option<String>,
) -> crate::Result<Filed> {
    let now = Utc::now();
    let (kind, message, community) = match &reported {
        Reported::Message(message) => (ReportKind::Message, Some(*message), None),
        Reported::Profile { .. } => (ReportKind::Profile, None, None),
        Reported::Nickname { community, .. } => (ReportKind::Nickname, None, Some(*community)),
    };
    // A first report opens the case; concurrent first reports make one, the second finding
    // the first's.
    diesel::insert_into(report_case::table)
        .values(&CaseRow {
            id: ReportCaseId::new(),
            kind,
            subject,
            message,
            community,
            status: ReportStatus::Open,
            opened_at: now,
            last_reported_at: now,
            closed_at: None,
            closed_by: None,
            resolution: None,
        })
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    let unresolved = report_case::table
        .select(CaseRow::as_select())
        .filter(report_case::kind.eq(kind))
        .filter(report_case::status.ne(ReportStatus::Resolved));
    let case: CaseRow = match (message, community) {
        (Some(message), _) => {
            unresolved
                .filter(report_case::message.eq(message))
                .for_update()
                .first(conn)
                .await?
        }
        (None, Some(community)) => {
            unresolved
                .filter(report_case::subject.eq(subject))
                .filter(report_case::community.eq(community))
                .for_update()
                .first(conn)
                .await?
        }
        (None, None) => {
            unresolved
                .filter(report_case::subject.eq(subject))
                .for_update()
                .first(conn)
                .await?
        }
    };
    let already: bool = diesel::select(diesel::dsl::exists(
        report::table.filter(report::case.eq(case.id).and(report::reporter.eq(reporter))),
    ))
    .get_result(conn)
    .await?;
    if already {
        return Err(crate::Error::AlreadyReported);
    }
    let reopened = case.status == ReportStatus::Dismissed;
    diesel::update(report_case::table.find(case.id))
        .set((
            report_case::status.eq(ReportStatus::Open),
            report_case::last_reported_at.eq(now),
            report_case::closed_at.eq(None::<DateTime<Utc>>),
            report_case::closed_by.eq(None::<UserId>),
        ))
        .execute(conn)
        .await?;
    if reopened {
        tracing::info!(case = %case.id.0, "a dismissed report case was reported again");
    }
    let (aspects, profile, nickname) = match reported {
        Reported::Message(_) => (Vec::new(), None, None),
        Reported::Profile { aspects, profile } => (aspects, Some(profile), None),
        Reported::Nickname { nickname, .. } => (Vec::new(), None, Some(nickname)),
    };
    let row = ReportRow {
        id: ReportId::new(),
        case: case.id,
        reporter,
        category: request.category,
        explanation,
        aspects: ProfileAspects(aspects),
        profile,
        nickname,
        created_at: now,
    };
    diesel::insert_into(report::table)
        .values(&row)
        .execute(conn)
        .await?;
    announce(state, conn, case.id).await?;
    Ok(Filed {
        id: row.id,
        created_at: now,
    })
}

/// Reports a message the reporter can read. An echo is reported as the reply it shows.
pub async fn report_message(
    state: &GlobalServerContext,
    reporter: UserId,
    message_id: MessageId,
    request: &ReportRequest,
) -> crate::Result<Filed> {
    let mut conn = state.connection_pool.get().await?;
    let (channel, author, kind, echo_of): (ChannelId, UserId, MessageKind, Option<MessageId>) =
        message::table
            .select((
                message::channel,
                message::author,
                message::kind,
                message::echo_of,
            ))
            .filter(
                message::id
                    .eq(message_id)
                    .and(message::deleted_at.is_null()),
            )
            .first(conn.as_mut())
            .await?;
    crate::permissions::channel_access(state, conn.as_mut(), reporter, channel).await?;
    if kind == MessageKind::ThreadEcho
        && let Some(reply) = echo_of
    {
        drop(conn);
        return Box::pin(report_message(state, reporter, reply, request)).await;
    }
    check_subject(conn.as_mut(), reporter, author).await?;
    let explanation = checked_request(conn.as_mut(), request).await?;
    conn.transaction(|conn| {
        async move {
            file(
                state,
                conn.as_mut(),
                author,
                Reported::Message(message_id),
                reporter,
                request,
                explanation,
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// Reports a person's profile, naming the aspects found objectionable; the profile is kept as
/// it stands.
pub async fn report_profile(
    state: &GlobalServerContext,
    reporter: UserId,
    subject: UserId,
    request: &ReportRequest,
    aspects: Vec<ProfileAspect>,
) -> crate::Result<Filed> {
    let mut aspects_named: Vec<ProfileAspect> = Vec::new();
    for aspect in aspects {
        if !aspects_named.contains(&aspect) {
            aspects_named.push(aspect);
        }
    }
    if aspects_named.is_empty() {
        return Err(crate::Error::Validation(t!("reportAspectsRequired")));
    }
    let mut conn = state.connection_pool.get().await?;
    check_subject(conn.as_mut(), reporter, subject).await?;
    let explanation = checked_request(conn.as_mut(), request).await?;
    let profile: UserPg = user::table
        .select(UserPg::as_select())
        .find(subject)
        .first(conn.as_mut())
        .await?;
    conn.transaction(|conn| {
        async move {
            file(
                state,
                conn.as_mut(),
                subject,
                Reported::Profile {
                    aspects: aspects_named,
                    profile: ProfileSnapshot::from(&profile),
                },
                reporter,
                request,
                explanation,
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// Reports the nickname `subject` chose in `community`, which the reporter must belong to; the
/// nickname is kept as it stands.
pub async fn report_nickname(
    state: &GlobalServerContext,
    reporter: UserId,
    community: CommunityId,
    subject: UserId,
    request: &ReportRequest,
) -> crate::Result<Filed> {
    let mut conn = state.connection_pool.get().await?;
    crate::permissions::require_actual_member(conn.as_mut(), reporter, community).await?;
    let nickname: Option<String> = community_user::table
        .select(community_user::nickname)
        .filter(
            community_user::community
                .eq(community)
                .and(community_user::user.eq(subject)),
        )
        .first(conn.as_mut())
        .await?;
    let Some(nickname) = nickname else {
        return Err(crate::Error::Validation(t!("reportNoNickname")));
    };
    check_subject(conn.as_mut(), reporter, subject).await?;
    let explanation = checked_request(conn.as_mut(), request).await?;
    conn.transaction(|conn| {
        async move {
            file(
                state,
                conn.as_mut(),
                subject,
                Reported::Nickname {
                    community,
                    nickname,
                },
                reporter,
                request,
                explanation,
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Review

/// Tells each holder of Review reports who may see the case (`may_act`) that what awaits review
/// changed, with how many of the cases they may see are open now, inside the caller's
/// transaction.
async fn announce(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    case: ReportCaseId,
) -> crate::Result<()> {
    let subject: UserId = report_case::table
        .select(report_case::subject)
        .find(case)
        .first(conn)
        .await?;
    let reviewers: Vec<UserId> = user_deployment_role::table
        .inner_join(deployment_role::table)
        .select(user_deployment_role::user)
        .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(
            "deployment_role.permissions & {} <> 0",
            DeploymentPermissions::REVIEW_REPORTS.bits()
        )))
        .distinct()
        .load(conn)
        .await?;
    let ranks = ranks(conn, &reviewers).await?;
    let standing = standing_of(conn, subject).await?;
    for reviewer in reviewers {
        let rank = ranks.get(&reviewer).copied().unwrap_or(0);
        if !reviewable(reviewer, rank, subject, standing) {
            continue;
        }
        let open: i64 = report_case::table
            .filter(report_case::status.eq(ReportStatus::Open))
            .filter(reviewable_cases(reviewer, rank))
            .count()
            .get_result(conn)
            .await?;
        publish_event(
            state,
            conn,
            EventScope::User(reviewer),
            &ServerEvent::ReportsChanged { case, open },
        )
        .await?;
    }
    Ok(())
}

/// How many of the cases the reviewer may see are open and dismissed. Takes Review reports.
pub async fn counts(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
) -> crate::Result<ReportCounts> {
    access.require(DeploymentPermission::ReviewReports)?;
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<(ReportStatus, i64)> = report_case::table
        .group_by(report_case::status)
        .select((report_case::status, diesel::dsl::count_star()))
        .filter(report_case::status.ne(ReportStatus::Resolved))
        .filter(reviewable_cases(access.user, access.rank()))
        .load(conn.as_mut())
        .await?;
    let count = |status| {
        rows.iter()
            .find(|(s, _)| *s == status)
            .map_or(0, |(_, n)| *n)
    };
    Ok(ReportCounts {
        open: count(ReportStatus::Open),
        dismissed: count(ReportStatus::Dismissed),
    })
}

/// The highest deployment role position each of `users` holds, 0 with none.
async fn ranks(
    conn: &mut AsyncPgConnection,
    users: &[UserId],
) -> crate::Result<HashMap<UserId, i32>> {
    let rows: Vec<(UserId, i32)> = user_deployment_role::table
        .inner_join(deployment_role::table)
        .select((user_deployment_role::user, deployment_role::position))
        .filter(user_deployment_role::user.eq_any(users))
        .load(conn)
        .await?;
    let mut ranks = HashMap::new();
    for (user, position) in rows {
        let rank = ranks.entry(user).or_insert(0);
        *rank = (*rank).max(position);
    }
    Ok(ranks)
}

/// Who answers for a case's subject, for review: a bot's owner, whose bot acts for them, and
/// the rank it is judged at, the higher of the subject's and its owner's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Standing {
    owner: Option<UserId>,
    rank: i32,
}

/// The `Standing` of each of `subjects`; one a query does not find stands alone at rank 0.
async fn standings(
    conn: &mut AsyncPgConnection,
    subjects: &[UserId],
) -> crate::Result<HashMap<UserId, Standing>> {
    let owners: HashMap<UserId, UserId> = user::table
        .select((user::id, user::bot_owner.assume_not_null()))
        .filter(user::id.eq_any(subjects))
        .filter(user::bot_owner.is_not_null())
        .load::<(UserId, UserId)>(conn)
        .await?
        .into_iter()
        .collect();
    let people: Vec<UserId> = subjects
        .iter()
        .copied()
        .chain(owners.values().copied())
        .collect();
    let ranks = ranks(conn, &people).await?;
    let rank_of = |user: &UserId| ranks.get(user).copied().unwrap_or(0);
    Ok(subjects
        .iter()
        .map(|subject| {
            let owner = owners.get(subject).copied();
            let rank = rank_of(subject).max(owner.as_ref().map_or(0, rank_of));
            (*subject, Standing { owner, rank })
        })
        .collect())
}

/// The `Standing` of one subject.
async fn standing_of(conn: &mut AsyncPgConnection, subject: UserId) -> crate::Result<Standing> {
    Ok(standings(conn, &[subject])
        .await?
        .remove(&subject)
        .unwrap_or_default())
}

/// Whether `access` may act on a case about `subject`, standing as `standing`, and so see it at
/// all: nobody reviews a case about themselves or their own bot, or about someone (or a bot of
/// someone's) whose highest deployment role is not below theirs.
fn may_act(access: &DeploymentAccess, subject: UserId, standing: Standing) -> bool {
    reviewable(access.user, access.rank(), subject, standing)
}

/// Whether `reviewer`, of rank `rank`, may see and act on a case about `subject`.
fn reviewable(reviewer: UserId, rank: i32, subject: UserId, standing: Standing) -> bool {
    subject != reviewer && standing.owner != Some(reviewer) && standing.rank < rank
}

/// The cases `reviewer`, of rank `rank`, may see: `reviewable` as a filter on `report_case`,
/// judging a bot's case by its owner's `Standing` as `standings` does.
fn reviewable_cases(
    reviewer: UserId,
    rank: i32,
) -> Box<dyn BoxableExpression<report_case::table, diesel::pg::Pg, SqlType = diesel::sql_types::Bool>>
{
    use diesel::sql_types::{Bool, Integer};
    Box::new(
        report_case::subject.ne(reviewer).and(
            diesel::dsl::sql::<Bool>(
                "NOT EXISTS (SELECT 1 FROM user_deployment_role subject_role \
                 JOIN deployment_role ON deployment_role.id = subject_role.role \
                 WHERE (subject_role.\"user\" = report_case.subject \
                 OR subject_role.\"user\" = (SELECT subject_bot.bot_owner \
                 FROM \"user\" subject_bot WHERE subject_bot.id = report_case.subject)) \
                 AND deployment_role.position >= ",
            )
            .bind::<Integer, _>(rank)
            .sql(
                ") AND NOT EXISTS (SELECT 1 FROM \"user\" subject_bot \
                 WHERE subject_bot.id = report_case.subject AND subject_bot.bot_owner = ",
            )
            .bind::<diesel::sql_types::Uuid, _>(reviewer.0)
            .sql(") AND 0 < ")
            .bind::<Integer, _>(rank),
        ),
    )
}

/// The cases among `rows` with their reports, messages, and categories.
async fn page_of(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    access: &DeploymentAccess,
    rows: Vec<CaseRow>,
) -> crate::Result<CasePage> {
    let ids: Vec<ReportCaseId> = rows.iter().map(|c| c.id).collect();
    let subjects: Vec<UserId> = rows.iter().map(|c| c.subject).collect();
    let mut reports: HashMap<ReportCaseId, Vec<Report>> = HashMap::new();
    for row in report::table
        .select(ReportRow::as_select())
        .filter(report::case.eq_any(&ids))
        .order(report::created_at.asc())
        .load::<ReportRow>(conn)
        .await?
    {
        reports.entry(row.case).or_default().push(Report::from(row));
    }
    let standings = standings(conn, &subjects).await?;
    let banned_now: HashSet<UserId> = user::table
        .select(user::id)
        .filter(user::id.eq_any(&subjects))
        .filter(banned())
        .load::<UserId>(conn)
        .await?
        .into_iter()
        .collect();
    let message_ids: Vec<MessageId> = rows.iter().filter_map(|c| c.message).collect();
    let messages = reviewed(state, conn, &message_ids).await?;
    let used: HashSet<ReportCategoryId> = reports
        .values()
        .flatten()
        .map(|report| report.category)
        .collect();
    let categories = categories(state, true)
        .await?
        .into_iter()
        .filter(|category| used.contains(&category.id))
        .collect();
    let cases = rows
        .into_iter()
        .map(|row| ReportCase {
            may_act: may_act(
                access,
                row.subject,
                standings.get(&row.subject).copied().unwrap_or_default(),
            ),
            subject_banned: banned_now.contains(&row.subject),
            reports: reports.remove(&row.id).unwrap_or_default(),
            id: row.id,
            kind: row.kind,
            status: row.status,
            subject: row.subject,
            message: row.message,
            community: row.community,
            opened_at: row.opened_at,
            last_reported_at: row.last_reported_at,
            closed_at: row.closed_at,
            closed_by: row.closed_by,
            resolution: row.resolution,
        })
        .collect();
    Ok(CasePage {
        cases,
        messages,
        categories,
    })
}

/// The messages among `ids`, deleted or not, with their relations, oldest first.
async fn reviewed(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    ids: &[MessageId],
) -> crate::Result<Vec<ReviewedMessage>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<Message> = message::table
        .select(Message::as_select())
        .filter(message::id.eq_any(ids))
        .order(message::id.asc())
        .load(conn)
        .await?;
    let deleted: HashMap<MessageId, Option<DateTime<Utc>>> =
        rows.iter().map(|row| (row.id, row.deleted_at)).collect();
    let mut removed: HashMap<MessageId, Vec<AttachmentId>> = HashMap::new();
    for (message, attachment) in crate::attachment::evidence::removed_from(conn, ids).await? {
        removed.entry(message).or_default().push(attachment);
    }
    Ok(with_relations(state, conn, rows)
        .await?
        .into_iter()
        .map(|message| ReviewedMessage {
            deleted_at: deleted.get(&message.message.id).copied().flatten(),
            removed_attachments: removed.remove(&message.message.id).unwrap_or_default(),
            message,
        })
        .collect())
}

/// A page of the cases in `status`, most recently reported first. Takes Review reports.
pub async fn list_cases(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    status: ReportStatus,
    offset: i64,
    limit: i64,
) -> crate::Result<CasePage> {
    access.require(DeploymentPermission::ReviewReports)?;
    let mut conn = state.connection_pool.get().await?;
    // Resolved cases read in the order they were closed; the others in the order reports came.
    let mut query = report_case::table
        .select(CaseRow::as_select())
        .filter(report_case::status.eq(status))
        .filter(reviewable_cases(access.user, access.rank()))
        .into_boxed();
    query = if status == ReportStatus::Resolved {
        query.order((report_case::closed_at.desc(), report_case::id.desc()))
    } else {
        query.order((report_case::last_reported_at.desc(), report_case::id.desc()))
    };
    let rows: Vec<CaseRow> = query
        .offset(offset.clamp(0, crate::admin::MAX_OFFSET))
        .limit(limit.clamp(1, MAX_PAGE))
        .load(conn.as_mut())
        .await?;
    page_of(state, conn.as_mut(), access, rows).await
}

/// One case. Takes Review reports; a case the reviewer may not see (`may_act`) is not found.
pub async fn read_case(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: ReportCaseId,
) -> crate::Result<CasePage> {
    access.require(DeploymentPermission::ReviewReports)?;
    let mut conn = state.connection_pool.get().await?;
    let row: CaseRow = report_case::table
        .select(CaseRow::as_select())
        .filter(report_case::id.eq(id))
        .filter(reviewable_cases(access.user, access.rank()))
        .first(conn.as_mut())
        .await?;
    page_of(state, conn.as_mut(), access, vec![row]).await
}

/// Where a context window starts.
#[derive(Debug, Clone, Copy)]
pub enum ContextAnchor {
    /// Around the reported message.
    Around,
    /// The messages before this one.
    Before(MessageId),
    /// The messages after this one.
    After(MessageId),
}

/// Messages around a reported one, deleted ones included, and whether there are more before
/// and after them.
pub struct ContextWindow {
    pub channel: ChannelId,
    pub messages: Vec<ReviewedMessage>,
    pub more_before: bool,
    pub more_after: bool,
}

/// The messages of a reported message's channel or thread around it, read for its review, no
/// further than [`CONTEXT_REACH`] from it either way. Each page read of a DM's is written to the
/// moderation log. Takes Review reports; a case the reviewer may not see is not found.
pub async fn case_context(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: ReportCaseId,
    anchor: ContextAnchor,
) -> crate::Result<ContextWindow> {
    access.require(DeploymentPermission::ReviewReports)?;
    let mut conn = state.connection_pool.get().await?;
    let reported: Option<MessageId> = report_case::table
        .select(report_case::message)
        .filter(report_case::id.eq(id))
        .filter(reviewable_cases(access.user, access.rank()))
        .first(conn.as_mut())
        .await?;
    let Some(reported) = reported else {
        return Err(crate::Error::Validation(t!("reportNoContext")));
    };
    let channel: ChannelId = message::table
        .select(message::channel)
        .find(reported)
        .first(conn.as_mut())
        .await?;
    if let ChannelHome::Direct(dm) = channel_home(state, conn.as_mut(), channel).await?
        && !dm_recipients(conn.as_mut(), dm)
            .await?
            .contains(&access.user)
    {
        log_moderation(
            conn.as_mut(),
            access.user,
            ModerationAction::ReadReportContext,
            None,
            Some(channel),
            Some(reported.0.to_string()),
        )
        .await?;
    }
    // Echoes show replies the thread's own context holds; they are left out.
    let in_channel = || {
        message::channel
            .eq(channel)
            .and(message::kind.ne(MessageKind::ThreadEcho))
    };
    // The furthest message either way the context reaches, when there are more than that.
    let reach = async |conn: &mut AsyncPgConnection, earlier: bool| {
        let mut query = message::table
            .select(message::id)
            .filter(in_channel())
            .into_boxed();
        query = if earlier {
            query
                .filter(message::id.lt(reported))
                .order(message::id.desc())
        } else {
            query
                .filter(message::id.gt(reported))
                .order(message::id.asc())
        };
        query
            .offset(CONTEXT_REACH - 1)
            .limit(1)
            .first::<MessageId>(conn)
            .await
            .optional()
    };
    let earliest = reach(conn.as_mut(), true).await?;
    let latest = reach(conn.as_mut(), false).await?;
    let side = async |conn: &mut AsyncPgConnection,
                      before: Option<MessageId>,
                      after: Option<MessageId>| {
        let mut query = message::table
            .select(message::id)
            .filter(in_channel())
            .into_boxed();
        if let Some(earliest) = earliest {
            query = query.filter(message::id.ge(earliest));
        }
        if let Some(latest) = latest {
            query = query.filter(message::id.le(latest));
        }
        if let Some(before) = before {
            query = query
                .filter(message::id.lt(before))
                .order(message::id.desc());
        }
        if let Some(after) = after {
            query = query.filter(message::id.gt(after)).order(message::id.asc());
        }
        let mut ids: Vec<MessageId> = query.limit(CONTEXT_MESSAGES + 1).load(conn).await?;
        let more = ids.len() as i64 > CONTEXT_MESSAGES;
        ids.truncate(CONTEXT_MESSAGES as usize);
        Ok::<_, crate::Error>((ids, more))
    };
    let (ids, more_before, more_after) = match anchor {
        ContextAnchor::Around => {
            let (mut before, more_before) = side(conn.as_mut(), Some(reported), None).await?;
            let (after, more_after) = side(conn.as_mut(), None, Some(reported)).await?;
            before.push(reported);
            before.extend(after);
            (before, more_before, more_after)
        }
        ContextAnchor::Before(id) => {
            let (ids, more) = side(conn.as_mut(), Some(id), None).await?;
            (ids, more, true)
        }
        ContextAnchor::After(id) => {
            let (ids, more) = side(conn.as_mut(), None, Some(id)).await?;
            (ids, true, more)
        }
    };
    let messages = reviewed(state, conn.as_mut(), &ids).await?;
    Ok(ContextWindow {
        channel,
        messages,
        more_before,
        more_after,
    })
}

/// The actions a review takes on a case; at least one.
#[derive(Debug, Clone, Default)]
pub struct Actions {
    /// The warning's words, sent by the system account.
    pub warn: Option<String>,
    pub ban: Option<UserBanRequest>,
    pub delete_message: bool,
    /// For a profile case, the aspects to reset.
    pub reset: Vec<ProfileAspect>,
    /// For a nickname case, clear the nickname.
    pub clear_nickname: bool,
}

/// Loads a case to act on, refusing a reviewer who may not act on it.
async fn actionable(
    conn: &mut AsyncPgConnection,
    access: &DeploymentAccess,
    id: ReportCaseId,
) -> crate::Result<CaseRow> {
    access.require(DeploymentPermission::ReviewReports)?;
    let row: CaseRow = report_case::table
        .select(CaseRow::as_select())
        .find(id)
        .for_update()
        .first(conn)
        .await?;
    let standing = standing_of(conn, row.subject).await?;
    if !may_act(access, row.subject, standing) {
        return Err(crate::Error::Forbidden(t!("reportConflictOfInterest")));
    }
    Ok(row)
}

/// Resolves an open case with `actions`, each taking its own permission. The case is resolved
/// first, so two reviewers cannot both act on it; if an action then fails, it is opened again
/// and the error returned, and the actions taken before it stand.
pub async fn resolve(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: ReportCaseId,
    actions: Actions,
) -> crate::Result<CasePage> {
    let warn = match actions.warn.as_deref().map(str::trim) {
        Some(text) if text.is_empty() || text.chars().count() > WARNING_MAX_CHARS => {
            return Err(crate::Error::Validation(t!(
                "warningLength",
                max = WARNING_MAX_CHARS
            )));
        }
        text => text.map(str::to_string),
    };
    if warn.is_none()
        && actions.ban.is_none()
        && !actions.delete_message
        && actions.reset.is_empty()
        && !actions.clear_nickname
    {
        return Err(crate::Error::Validation(t!("reportNoAction")));
    }
    if actions.delete_message || actions.clear_nickname || !actions.reset.is_empty() {
        access.require(DeploymentPermission::RemoveContent)?;
    }
    if let Some(ban) = &actions.ban {
        access.require(DeploymentPermission::BanUsers)?;
        crate::ban::validate(&ban.ban)?;
        if ban.ban.delete_messages_seconds.is_some() {
            access.require(DeploymentPermission::RemoveContent)?;
        }
    }
    let (deleting, resetting, clearing) = (
        actions.delete_message,
        !actions.reset.is_empty(),
        actions.clear_nickname,
    );
    let mut conn = state.connection_pool.get().await?;
    let case = conn
        .transaction(|conn| {
            async move {
                let case = actionable(conn.as_mut(), access, id).await?;
                if case.status != ReportStatus::Open {
                    return Err(crate::Error::Conflict(t!("reportCaseClosed")));
                }
                if deleting && case.kind != ReportKind::Message {
                    return Err(crate::Error::Validation(t!("reportDeleteNotMessage")));
                }
                if clearing && case.kind != ReportKind::Nickname {
                    return Err(crate::Error::Validation(t!("reportClearNotNickname")));
                }
                if resetting {
                    if case.kind != ReportKind::Profile {
                        return Err(crate::Error::Validation(t!("reportResetNotProfile")));
                    }
                    let foreign: bool = user::table
                        .select(user::home_domain.is_not_null())
                        .find(case.subject)
                        .first(conn.as_mut())
                        .await?;
                    if foreign {
                        return Err(crate::Error::Validation(t!("reportResetForeign")));
                    }
                }
                close(
                    state,
                    conn.as_mut(),
                    access,
                    id,
                    ReportStatus::Resolved,
                    None,
                )
                .await?;
                Ok(case)
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    let mut resolution = Resolution::default();
    let outcome = act(state, access, &case, warn, &actions, &mut resolution).await;
    let mut conn = state.connection_pool.get().await?;
    let reopened = outcome.is_err();
    conn.transaction(|conn| {
        let resolution = resolution.clone();
        async move {
            if reopened {
                diesel::update(report_case::table.find(id))
                    .set((
                        report_case::status.eq(ReportStatus::Open),
                        report_case::closed_at.eq(None::<DateTime<Utc>>),
                        report_case::closed_by.eq(None::<UserId>),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                announce(state, conn.as_mut(), id).await
            } else {
                diesel::update(report_case::table.find(id))
                    .set(report_case::resolution.eq(Some(resolution)))
                    .execute(conn.as_mut())
                    .await
                    .map(drop)
                    .map_err(Into::into)
            }
        }
        .scope_boxed()
    })
    .await?;
    outcome?;
    drop(conn);
    read_case(state, access, id).await
}

/// Takes a resolved case's actions, recording each in `resolution` as it is done.
async fn act(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    case: &CaseRow,
    warn: Option<String>,
    actions: &Actions,
    resolution: &mut Resolution,
) -> crate::Result<()> {
    if let Some(text) = warn {
        let warning = warning_of(state, case).await?;
        let message = send_warning(state, access, case.subject, text.clone(), warning).await?;
        resolution.warning = Some(text);
        resolution.warning_message = Some(message);
    }
    if actions.delete_message
        && let Some(message) = case.message
    {
        delete_reported_message(state, access, message).await?;
        resolution.deleted_message = true;
    }
    if !actions.reset.is_empty() {
        reset_profile(state, access, case.subject, &actions.reset).await?;
        resolution.reset = actions.reset.clone();
    }
    if actions.clear_nickname
        && let Some(community) = case.community
    {
        clear_reported_nickname(state, access, community, case.subject).await?;
        resolution.cleared_nickname = true;
    }
    if let Some(ban) = &actions.ban {
        crate::user_ban::ban_user(state, access, case.subject, ban).await?;
        resolution.ban = Some(BanGiven {
            reason: ban.ban.reason.clone(),
            duration_seconds: ban.ban.duration_seconds,
            delete_messages_seconds: ban.ban.delete_messages_seconds,
            with_owner: ban.with_owner,
        });
    }
    Ok(())
}

/// What a warning about `case` names: the message, the profile as the latest report found it
/// with every aspect the reports named, or the nickname as the latest report found it.
async fn warning_of(state: &GlobalServerContext, case: &CaseRow) -> crate::Result<Warning> {
    let mut conn = state.connection_pool.get().await?;
    let reports: Vec<ReportRow> = report::table
        .select(ReportRow::as_select())
        .filter(report::case.eq(case.id))
        .order(report::created_at.asc())
        .load(conn.as_mut())
        .await?;
    let mut aspects: Vec<ProfileAspect> = Vec::new();
    for aspect in reports.iter().flat_map(|r| r.aspects.0.iter()) {
        if !aspects.contains(aspect) {
            aspects.push(*aspect);
        }
    }
    let nickname = match (
        case.community,
        reports.iter().rev().find_map(|r| r.nickname.clone()),
    ) {
        (Some(community_id), Some(nickname)) => {
            let community_name: String = community::table
                .select(community::name)
                .find(community_id)
                .first(conn.as_mut())
                .await?;
            Some(NicknameSnapshot {
                community: community_id,
                community_name,
                nickname,
            })
        }
        _ => None,
    };
    Ok(Warning {
        subject: case.subject,
        message: case.message,
        profile: reports.iter().rev().find_map(|r| r.profile.clone()),
        aspects,
        nickname,
    })
}

/// Sends `subject` a warning from the system account, for the deployment's moderators, which
/// reaches them whatever communities they share with anyone and whoever they blocked, and logs
/// the reviewer who gave it. Returns the warning's message.
async fn send_warning(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    subject: UserId,
    text: String,
    warning: Warning,
) -> crate::Result<MessageId> {
    let message =
        crate::system_account::post(state, subject, text, Posting::Warning(Box::new(warning)))
            .await?;
    let mut conn = state.connection_pool.get().await?;
    log_moderation(
        conn.as_mut(),
        access.user,
        ModerationAction::WarnUser,
        None,
        Some(*message.channel.id()),
        Some(subject.0.to_string()),
    )
    .await?;
    Ok(message.id)
}

/// A username for an account whose own was reset: `user-` and eight random letters and digits.
fn placeholder_username() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let suffix: String = crate::CHACHA_RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        (0..8)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect()
    });
    format!("user-{suffix}")
}

/// Deletes the reported message wherever it is, which Remove content allows without the
/// reviewer reaching its channel, and logs it. Its author may have deleted it meanwhile.
async fn delete_reported_message(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: MessageId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let channel: ChannelId = message::table
                .select(message::channel)
                .find(id)
                .first(conn.as_mut())
                .await?;
            match crate::message::soft_delete(state, conn.as_mut(), id).await {
                Ok(()) => {}
                Err(crate::Error::Diesel(diesel::result::Error::NotFound)) => return Ok(()),
                Err(e) => return Err(e),
            }
            let community = match channel_home(state, conn.as_mut(), channel).await? {
                ChannelHome::Community { community, .. } => Some(community),
                ChannelHome::Direct(_) => None,
            };
            log_moderation(
                conn.as_mut(),
                access.user,
                ModerationAction::DeleteMessage,
                community,
                Some(channel),
                Some(id.0.to_string()),
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// Clears the named aspects of `subject`'s profile; a username becomes a placeholder, and the
/// system account tells them to choose another. Written to the moderation log.
async fn reset_profile(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    subject: UserId,
    aspects: &[ProfileAspect],
) -> crate::Result<()> {
    let mut request = UserUpdateRequest::default();
    for aspect in aspects {
        match aspect {
            ProfileAspect::DisplayName => request.display_name = Some(None),
            ProfileAspect::Picture => request.icon = Some(None),
            ProfileAspect::Status => request.status = Some(None),
            ProfileAspect::Bio => request.bio = Some(None),
            ProfileAspect::Pronouns => request.pronouns = Some(None),
            ProfileAspect::Username => {}
        }
    }
    let renamed = aspects.contains(&ProfileAspect::Username);
    // A placeholder that happens to be taken is drawn again.
    let mut attempts = 0;
    let updated = loop {
        attempts += 1;
        if renamed {
            request.name = Some(placeholder_username());
        }
        match crate::user::apply_profile_update(state.clone(), subject, request.clone()).await {
            Err(crate::Error::Diesel(diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            ))) if renamed && attempts < 3 => continue,
            outcome => break outcome?,
        }
    };
    let mut conn = state.connection_pool.get().await?;
    log_moderation(
        conn.as_mut(),
        access.user,
        ModerationAction::ResetProfile,
        None,
        None,
        Some(subject.0.to_string()),
    )
    .await?;
    drop(conn);
    if renamed {
        crate::system_account::notify(
            state,
            subject,
            t!("usernameResetNotice", name = updated.user_pg.name).into_owned(),
        )
        .await?;
    }
    Ok(())
}

/// Clears `subject`'s nickname in `community`, whatever it is now, if they are still there with
/// one; their rank in the community does not matter, since the reviewer's rank among the
/// deployment's moderators was checked. Written to the moderation log.
async fn clear_reported_nickname(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    community: CommunityId,
    subject: UserId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            crate::community::erase_nickname(state, conn.as_mut(), community, subject).await?;
            log_moderation(
                conn.as_mut(),
                access.user,
                ModerationAction::ClearNickname,
                Some(community),
                None,
                Some(subject.0.to_string()),
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// Marks a case resolved or dismissed by the reviewer, with what was done, inside the caller's
/// transaction, and announces it.
async fn close(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    access: &DeploymentAccess,
    id: ReportCaseId,
    status: ReportStatus,
    resolution: Option<Resolution>,
) -> crate::Result<()> {
    diesel::update(report_case::table.find(id))
        .set((
            report_case::status.eq(status),
            report_case::closed_at.eq(Some(Utc::now())),
            report_case::closed_by.eq(Some(access.user)),
            report_case::resolution.eq(resolution),
        ))
        .execute(conn)
        .await?;
    announce(state, conn, id).await
}

/// Dismisses an open case: it is hidden from review, kept, and opened again if restored or
/// reported again. Takes Review reports, and a case the reviewer may act on.
pub async fn dismiss(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: ReportCaseId,
) -> crate::Result<CasePage> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let case = actionable(conn.as_mut(), access, id).await?;
            match case.status {
                ReportStatus::Open => {
                    close(
                        state,
                        conn.as_mut(),
                        access,
                        id,
                        ReportStatus::Dismissed,
                        None,
                    )
                    .await
                }
                ReportStatus::Dismissed => Ok(()),
                ReportStatus::Resolved => Err(crate::Error::Conflict(t!("reportCaseClosed"))),
            }
        }
        .scope_boxed()
    })
    .await?;
    drop(conn);
    read_case(state, access, id).await
}

/// Restores a dismissed case to review. Takes Review reports, and a case the reviewer may act
/// on.
pub async fn restore(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    id: ReportCaseId,
) -> crate::Result<CasePage> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let case = actionable(conn.as_mut(), access, id).await?;
            match case.status {
                ReportStatus::Dismissed => {
                    diesel::update(report_case::table.find(id))
                        .set((
                            report_case::status.eq(ReportStatus::Open),
                            report_case::closed_at.eq(None::<DateTime<Utc>>),
                            report_case::closed_by.eq(None::<UserId>),
                        ))
                        .execute(conn.as_mut())
                        .await?;
                    announce(state, conn.as_mut(), id).await
                }
                ReportStatus::Open => Ok(()),
                ReportStatus::Resolved => Err(crate::Error::Conflict(t!("reportCaseClosed"))),
            }
        }
        .scope_boxed()
    })
    .await?;
    drop(conn);
    read_case(state, access, id).await
}

// ---------------------------------------------------------------------------------------------
// Warnings

/// The messages the warnings among `messages` are about, deleted ones included, for the people
/// of the warnings' DMs, who are the only ones to read a warning.
pub async fn warned_messages(
    state: &GlobalServerContext,
    messages: &[aspen_wire::message_enum::Message],
) -> crate::Result<Vec<ReviewedMessage>> {
    let ids: Vec<MessageId> = messages
        .iter()
        .filter(|m| m.kind == MessageKind::Warning)
        .filter_map(|m| m.warning.as_ref().and_then(|w| w.message))
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let mut messages = reviewed(state, conn.as_mut(), &ids).await?;
    // The warned person reads no evidence.
    for message in &mut messages {
        message.removed_attachments.clear();
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_categories_are_offered_in_order_with_other_last() {
        let all = <BuiltinCategory as strum::VariantArray>::VARIANTS;
        assert_eq!(all.last(), Some(&BuiltinCategory::Other));
        assert_eq!(BuiltinCategory::Spam.rank(), 0);
        // The names the migration seeds are the wire names.
        assert_eq!(BuiltinCategory::HateSpeech.to_string(), "hateSpeech");
        assert_eq!(
            "illegalContent".parse::<BuiltinCategory>().unwrap(),
            BuiltinCategory::IllegalContent
        );
    }

    #[test]
    fn a_placeholder_username_is_user_and_eight_letters_or_digits() {
        let name = placeholder_username();
        let suffix = name.strip_prefix("user-").unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn nobody_acts_on_their_own_case_or_one_about_their_equals() {
        let access = DeploymentAccess {
            user: UserId::new(),
            positions: vec![2],
            permissions: DeploymentPermissions::REVIEW_REPORTS,
        };
        let at = |rank| Standing { owner: None, rank };
        assert!(may_act(&access, UserId::new(), at(1)));
        assert!(!may_act(&access, UserId::new(), at(2)));
        assert!(!may_act(&access, access.user, at(0)));
    }

    #[test]
    fn nobody_acts_on_a_case_about_their_own_bot() {
        let access = DeploymentAccess {
            user: UserId::new(),
            positions: vec![2],
            permissions: DeploymentPermissions::REVIEW_REPORTS,
        };
        let owned = Standing {
            owner: Some(access.user),
            rank: 0,
        };
        assert!(!may_act(&access, UserId::new(), owned));
        let others = Standing {
            owner: Some(UserId::new()),
            rank: 1,
        };
        assert!(may_act(&access, UserId::new(), others));
    }
}
