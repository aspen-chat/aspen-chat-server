//! The jobs preview of the Administration Dashboard (`GET /admin/jobs`): what the deployment's
//! background jobs are doing, a glance rather than a diagnostic tool (`app::jobs`).

use crate::TAG_ADMIN;
use crate::admin::AdminUser;
use crate::error::{ApiResult, Problem};
use crate::extract::Json;
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app};
pub use aspen_wire::job::JobClass;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// A background job as the preview lists it: never what it was given.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobEntry {
    pub id: uuid::Uuid,
    /// What it does, by its kind's name.
    pub kind: String,
    pub class: JobClass,
    /// When it was due, which its age counts from.
    pub due: DateTime<Utc>,
    /// When the server running it started it; `null` while it waits.
    pub running_since: Option<DateTime<Utc>>,
    /// How many times it has been tried since it last came further.
    pub attempts: u32,
    /// Whether it runs on a schedule.
    pub recurring: bool,
    /// When it was given up, and why; `null` unless it was.
    pub failed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl From<app::jobs::Listed> for JobEntry {
    fn from(job: app::jobs::Listed) -> Self {
        JobEntry {
            class: job.class(),
            id: job.id,
            kind: job.kind,
            due: job.due,
            running_since: job.running_since,
            attempts: u32::try_from(job.attempts).unwrap_or(0),
            recurring: job.recurring,
            failed_at: job.failed_at,
            error: job.error,
        }
    }
}

/// How many jobs wait in one class.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobClassCount {
    pub class: JobClass,
    pub count: u32,
}

/// The preview: at most 100 jobs, those running first, then those waiting by class and age,
/// then the latest given up; and how many of each there are, each count stopping at 1000.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobsOverview {
    pub running: Vec<JobEntry>,
    pub waiting: Vec<JobEntry>,
    pub failed: Vec<JobEntry>,
    pub running_count: u32,
    pub waiting_counts: Vec<JobClassCount>,
    pub failed_count: u32,
}

fn counted(count: i64) -> u32 {
    u32::try_from(count).unwrap_or(0)
}

/// What the deployment's background jobs are doing now. Takes View jobs.
#[utoipa::path(
    get,
    path = "/admin/jobs",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = JobsOverview),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_jobs(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<JobsOverview>> {
    let overview = app::jobs::overview(&state, &access).await?;
    Ok(Json(JobsOverview {
        running: overview.running.into_iter().map(JobEntry::from).collect(),
        waiting: overview.waiting.into_iter().map(JobEntry::from).collect(),
        failed: overview.failed.into_iter().map(JobEntry::from).collect(),
        running_count: counted(overview.running_count),
        waiting_counts: overview
            .waiting_counts
            .into_iter()
            .map(|(class, count)| JobClassCount {
                class,
                count: counted(count),
            })
            .collect(),
        failed_count: counted(overview.failed_count),
    }))
}
