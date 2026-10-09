//! Work done in the background, in bounded steps, by whichever server claims it: the `job`
//! table and the runner every API server keeps going over it ([`spawn_runner`]).
//!
//! A job is saved in the transaction of what decided it ([`enqueue`]), so it exists exactly
//! when that commits, and nothing of it is lost to a restart. Each step a job's kind takes is
//! bounded (a few hundred rows, one mail, one preview) and commits on its own, publishing its
//! events as every write does, before its commit; a step that reaches the database writes how
//! far the job has come ([`checkpoint`]) in its own transaction, so a server that stops part way
//! leaves the job where its last step left it, and the next to claim it carries on from there.
//! Every kind's steps can be run again, since a lease that ran out lets another server take a
//! job whose step had in fact finished.
//!
//! A runner claims jobs by pushing their `not_before` past their lease, under
//! `FOR UPDATE SKIP LOCKED`, and keeps renewing the lease while a step runs; a job whose lease
//! runs out, its server having stopped, is claimed again. Classes ([`JobClass`]) are claimed in
//! order, each keeping one place of its own beside the server's shared places
//! (`[jobs] concurrency`), so urgent work goes first and upkeep is never starved. A recurring
//! job ([`Recurring`]) is declared in code; each server makes sure of it when it starts, and
//! once done it is due a period after it was last due, missed periods skipped. A job that fails
//! is tried again later, waiting longer each time, until its kind's attempts run out; it is then
//! kept as failed, for the operator (`jobs retry`, `jobs cancel`), and swept after
//! [`FAILED_KEPT_DAYS`].

pub mod delete_messages;
pub mod upkeep;

use crate::context::GlobalServerContext;
pub use aspen_wire::job::{JobClass, JobKind};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Integer, Jsonb, Nullable, SmallInt, Text, Timestamptz};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

/// How long a claim holds a job before another server may take it, renewed while it runs.
const LEASE_SECONDS: i32 = 60;
/// How often a running job's lease is renewed.
const RENEW_EVERY: Duration = Duration::from_secs(20);
/// How often a runner looks for jobs when nothing wakes it sooner.
const POLL: Duration = Duration::from_secs(1);
/// How long a job given up is kept for the operator before it is swept.
pub const FAILED_KEPT_DAYS: i32 = 30;
/// The core NATS subject that wakes every runner at once, published when a job someone waits
/// for is saved (`wake`).
pub const WAKE_SUBJECT: &str = "aspen.jobs.wake";

/// What a step of a job did.
#[derive(Debug)]
pub enum Outcome {
    /// The job is done: a one-shot job goes, a recurring one is due again.
    Done,
    /// It came further and saved how far with [`checkpoint`] in its step's transaction; the next
    /// step follows at once.
    Continue,
    /// It came further and this is how far, saved by the runner; the next step follows at once.
    Progress(serde_json::Value),
    /// It cannot go on yet (the work it does is waiting on something else); it is tried again
    /// after this long, which counts as no attempt.
    Later(Duration),
}

/// A job a runner holds: what it is and how far it has come.
#[derive(Debug, Clone, QueryableByName)]
pub struct Claimed {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = Text)]
    pub kind: JobKind,
    #[diesel(sql_type = Nullable<Text>)]
    pub key: Option<String>,
    #[diesel(sql_type = SmallInt)]
    class: i16,
    #[diesel(sql_type = Timestamptz)]
    pub due: DateTime<Utc>,
    #[diesel(sql_type = Nullable<BigInt>)]
    every_seconds: Option<i64>,
    #[diesel(sql_type = Jsonb)]
    pub payload: serde_json::Value,
    #[diesel(sql_type = Nullable<Jsonb>)]
    pub progress: Option<serde_json::Value>,
    #[diesel(sql_type = Integer)]
    pub attempts: i32,
}

impl Claimed {
    pub fn class(&self) -> JobClass {
        JobClass::of_rank(self.class)
    }

    /// The job's input, as its kind wrote it.
    pub fn payload<T: DeserializeOwned>(&self) -> crate::Result<T> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }

    /// How far it has come, as its kind wrote it; `None` before its first step.
    pub fn progress<T: DeserializeOwned>(&self) -> crate::Result<Option<T>> {
        Ok(self
            .progress
            .clone()
            .map(serde_json::from_value)
            .transpose()?)
    }
}

/// A job to save: its kind, how soon it must be done, what it is given, and, optionally, a key
/// naming it among its kind's and a time before which it does not start.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub kind: JobKind,
    pub class: JobClass,
    pub key: Option<String>,
    pub payload: serde_json::Value,
    pub not_before: Option<DateTime<Utc>>,
}

impl NewJob {
    pub fn new(kind: JobKind, class: JobClass, payload: &impl Serialize) -> crate::Result<Self> {
        Ok(Self {
            kind,
            class,
            key: None,
            payload: serde_json::to_value(payload)?,
            not_before: None,
        })
    }

    pub fn keyed(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn not_before(mut self, at: DateTime<Utc>) -> Self {
        self.not_before = Some(at);
        self
    }
}

/// Saves `job` on `conn`, inside the transaction of what decided it. A job of the same kind and
/// key already saved is left as it is, and its id answered.
pub async fn enqueue(conn: &mut AsyncPgConnection, job: NewJob) -> crate::Result<Uuid> {
    #[derive(QueryableByName)]
    struct Saved {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        id: Uuid,
    }
    let saved: Saved = diesel::sql_query(
        r#"
        WITH made AS (
            INSERT INTO job (id, kind, key, class, due, not_before, payload)
            VALUES ($1, $2, $3, $4, COALESCE($5, now()), COALESCE($5, now()), $6)
            ON CONFLICT (kind, key) DO NOTHING
            RETURNING id
        )
        SELECT id FROM made
        UNION ALL
        SELECT id FROM job WHERE kind = $2 AND key = $3 AND NOT EXISTS (SELECT 1 FROM made)
        "#,
    )
    .bind::<diesel::sql_types::Uuid, _>(Uuid::now_v7())
    .bind::<Text, _>(job.kind)
    .bind::<Nullable<Text>, _>(job.key)
    .bind::<SmallInt, _>(job.class.rank())
    .bind::<Nullable<Timestamptz>, _>(job.not_before)
    .bind::<Jsonb, _>(job.payload)
    .get_result(conn)
    .await?;
    Ok(saved.id)
}

/// Saves `jobs` on `conn` in one statement, as [`enqueue`] saves one; those of a kind and key
/// already saved are left as they are.
pub async fn enqueue_many(conn: &mut AsyncPgConnection, jobs: Vec<NewJob>) -> crate::Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = jobs.iter().map(|_| Uuid::now_v7()).collect();
    let kinds: Vec<JobKind> = jobs.iter().map(|job| job.kind).collect();
    let keys: Vec<Option<String>> = jobs.iter().map(|job| job.key.clone()).collect();
    let classes: Vec<i16> = jobs.iter().map(|job| job.class.rank()).collect();
    let starts: Vec<Option<DateTime<Utc>>> = jobs.iter().map(|job| job.not_before).collect();
    let payloads: Vec<serde_json::Value> = jobs.into_iter().map(|job| job.payload).collect();
    diesel::sql_query(
        r#"
        INSERT INTO job (id, kind, key, class, due, not_before, payload)
        SELECT id, kind, key, class, COALESCE(start, now()), COALESCE(start, now()), payload
        FROM unnest($1::uuid[], $2::text[], $3::text[], $4::smallint[], $5::timestamptz[],
                    $6::jsonb[]) AS j(id, kind, key, class, start, payload)
        ON CONFLICT (kind, key) DO NOTHING
        "#,
    )
    .bind::<Array<diesel::sql_types::Uuid>, _>(ids)
    .bind::<Array<Text>, _>(kinds)
    .bind::<Array<Nullable<Text>>, _>(keys)
    .bind::<Array<SmallInt>, _>(classes)
    .bind::<Array<Nullable<Timestamptz>>, _>(starts)
    .bind::<Array<Jsonb>, _>(payloads)
    .execute(conn)
    .await?;
    Ok(())
}

/// Writes how far `job` has come, inside the transaction of the step that came that far, and
/// renews its lease: what it did and what it says it did commit together.
pub async fn checkpoint(
    conn: &mut AsyncPgConnection,
    job: Uuid,
    progress: &impl Serialize,
) -> crate::Result<()> {
    let progress = serde_json::to_value(progress)?;
    diesel::sql_query(
        "UPDATE job SET progress = $2, attempts = 0, \
         not_before = now() + make_interval(secs => $3) WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(job)
    .bind::<Jsonb, _>(progress)
    .bind::<Integer, _>(LEASE_SECONDS)
    .execute(conn)
    .await?;
    Ok(())
}

/// Wakes every runner to look for jobs at once, rather than at its next look: for a job
/// someone is waiting on, published once what saved it has committed.
pub async fn wake(state: &GlobalServerContext) {
    if let Err(e) = state
        .nats_context
        .client()
        .publish(WAKE_SUBJECT, bytes::Bytes::new())
        .await
    {
        tracing::warn!("could not wake the job runners: {e}");
    }
}

/// A job every deployment runs on a schedule: its kind, how soon it must be done, and how often.
#[derive(Debug, Clone, Copy)]
pub struct Recurring {
    pub kind: JobKind,
    pub class: JobClass,
    pub every: Duration,
}

/// The recurring jobs, with the periods `state`'s configuration gives them.
fn recurring(state: &GlobalServerContext) -> Vec<Recurring> {
    const HOUR: Duration = Duration::from_secs(3600);
    vec![
        Recurring {
            kind: JobKind::PruneFailedJobs,
            class: JobClass::Maintenance,
            every: 24 * HOUR,
        },
        Recurring {
            kind: JobKind::SweepSignIns,
            class: JobClass::Maintenance,
            every: HOUR,
        },
        Recurring {
            kind: JobKind::ReapVoice,
            class: JobClass::Normal,
            every: crate::voice::REAPER_INTERVAL,
        },
        Recurring {
            kind: JobKind::SweepUploads,
            class: JobClass::Maintenance,
            every: crate::media_store::SWEEP_EVERY,
        },
        Recurring {
            kind: JobKind::MoveEvidence,
            class: JobClass::Normal,
            every: crate::attachment::evidence::MOVE_EVERY,
        },
        Recurring {
            kind: JobKind::MakeDigests,
            class: JobClass::Bulk,
            every: crate::email::digest::TICK,
        },
        Recurring {
            kind: JobKind::SweepExpired,
            class: JobClass::Maintenance,
            every: HOUR,
        },
        Recurring {
            kind: JobKind::RecountMembers,
            class: JobClass::Maintenance,
            every: HOUR,
        },
        Recurring {
            kind: JobKind::RecordStats,
            class: JobClass::Maintenance,
            every: HOUR,
        },
        Recurring {
            kind: JobKind::PurgeEvidence,
            class: JobClass::Maintenance,
            every: 24 * HOUR,
        },
        Recurring {
            kind: JobKind::ConfirmStanding,
            class: JobClass::Normal,
            every: crate::federation::standing::pass_every(&state.config.federation),
        },
    ]
}

/// Makes sure of every recurring job: saved once, its class and period as the code now says.
async fn ensure_recurring(state: &GlobalServerContext) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    for job in recurring(state) {
        diesel::sql_query(
            r#"
            INSERT INTO job (id, kind, key, class, due, not_before, every, payload)
            VALUES ($1, $2, '', $3, now(), now(), make_interval(secs => $4), '{}')
            ON CONFLICT (kind, key) DO UPDATE
            SET class = excluded.class, every = excluded.every
            "#,
        )
        .bind::<diesel::sql_types::Uuid, _>(Uuid::now_v7())
        .bind::<Text, _>(job.kind)
        .bind::<SmallInt, _>(job.class.rank())
        .bind::<diesel::sql_types::Double, _>(job.every.as_secs_f64())
        .execute(conn.as_mut())
        .await?;
    }
    Ok(())
}

/// What this server has of what some kinds need beyond its configuration, found out when its
/// runner starts.
struct Means {
    /// Whether it can run `ffmpeg` and `ffprobe`, which videos' posters are taken with.
    posters: bool,
}

impl Means {
    async fn find(state: &GlobalServerContext) -> Self {
        let previews = &state.config.media.previews;
        let posters = previews.make && aspen_previews::video::available(previews).await;
        if previews.make && !posters {
            tracing::warn!(
                ffmpeg = previews.ffmpeg,
                ffprobe = previews.ffprobe,
                "ffmpeg or ffprobe cannot be run, so this server makes previews of pictures only"
            );
        }
        Self { posters }
    }
}

/// Whether this server runs jobs of `kind`: every kind, unless what it needs is not here, as a
/// server that sends no mail makes no digests.
fn handles(state: &GlobalServerContext, means: &Means, kind: JobKind) -> bool {
    match kind {
        JobKind::MakeDigests | JobKind::SendEmail => {
            state.mailer.as_ref().is_some_and(|mailer| mailer.sends())
        }
        JobKind::MakePicturePreview => state.config.media.previews.make,
        JobKind::MakeVideoPoster => means.posters,
        _ => true,
    }
}

/// The kinds this server runs that share a bound of their own on how many run at once, beyond
/// the runner's places, with that bound: previews, which take memory and cores in proportion to
/// what they decode, at most `[media.previews] concurrency` at once.
fn bounded(state: &GlobalServerContext) -> Vec<(Vec<JobKind>, usize)> {
    vec![(
        crate::attachment::preview::KINDS.to_vec(),
        state.config.media.previews.concurrency.max(1),
    )]
}

/// How many times a job of `kind` is claimed without coming further before it is given up. A
/// piece of mail gives itself up first, once it has been tried as often as mail is
/// (`outbox::MAX_ATTEMPTS`), so its content is never kept as a job given up; so does a held
/// message, dropped with why on its last attempt.
fn max_attempts(kind: JobKind) -> i32 {
    match kind {
        JobKind::SendEmail => crate::email::outbox::MAX_ATTEMPTS + 1,
        JobKind::MakePicturePreview | JobKind::MakeVideoPoster => {
            crate::attachment::preview::MAX_ATTEMPTS
        }
        JobKind::ReleaseHeldMessage => crate::message::held::MAX_ATTEMPTS,
        JobKind::FirePluginTimer => crate::plugin::timer::MAX_ATTEMPTS,
        _ => 5,
    }
}

/// Whether this claim of `job` is its last before it is given up, should its step fail: for a
/// kind that does something of its own on giving up.
pub fn last_attempt(job: &Claimed) -> bool {
    job.attempts >= max_attempts(job.kind)
}

/// How long a job of `kind` waits after its `attempts`th failure before it is tried again: ten
/// seconds, doubling, at most an hour; mail and previews a minute, doubling, as mail servers
/// expect and as a store or `ffmpeg` that failed may take to come back; a held message half a
/// minute each time, since its author is waiting.
fn backoff(kind: JobKind, attempts: i32) -> Duration {
    let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 9)).unwrap_or(0);
    match kind {
        JobKind::SendEmail => crate::email::outbox::retry_wait(attempts)
            .to_std()
            .unwrap_or(Duration::from_secs(60)),
        JobKind::MakePicturePreview | JobKind::MakeVideoPoster => {
            Duration::from_secs(60 * 2u64.pow(doublings)).min(Duration::from_secs(3600))
        }
        JobKind::ReleaseHeldMessage => Duration::from_secs(30),
        _ => Duration::from_secs(10 * 2u64.pow(doublings)).min(Duration::from_secs(3600)),
    }
}

/// One step of `job`.
async fn step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    match job.kind {
        JobKind::DeleteMessagesBy => delete_messages::step(state, job).await,
        JobKind::PruneFailedJobs => upkeep::prune_failed_jobs(state, job).await,
        JobKind::SweepSignIns => upkeep::sweep_sign_ins(state, job).await,
        JobKind::ClosePoll => crate::poll::close_at_deadline(state, job).await,
        JobKind::ReapVoice => crate::voice::reap(state, job).await,
        JobKind::PurgeRole => crate::role::purge_step(state, job).await,
        JobKind::PurgeCustomEmoji => crate::custom_emoji::purge_step(state, job).await,
        JobKind::ForgetPluginScope => crate::plugin::storage::forget_step(state, job).await,
        JobKind::RetirePlugin => crate::plugin::install::retire_step(state, job).await,
        JobKind::PurgePlugin => crate::plugin::install::purge_step(state, job).await,
        JobKind::ShutOut => crate::federation::standing::shut_out_step(state, job).await,
        JobKind::RecheckAllCalls => crate::voice::recheck_all_step(state, job).await,
        JobKind::ConfirmStanding => crate::federation::standing::pass_step(state, job).await,
        JobKind::MakeDigests => crate::email::digest::make_step(state, job).await,
        JobKind::SweepUploads => crate::media_store::sweep_step(state, job).await,
        JobKind::MoveEvidence => crate::attachment::evidence::move_step(state, job).await,
        JobKind::SendEmail => crate::email::outbox::send_step(state, job).await,
        JobKind::QueueNewsletter => crate::email::newsletter::queue_step(state, job).await,
        JobKind::MakePicturePreview | JobKind::MakeVideoPoster => {
            crate::attachment::preview::make_step(state, job).await
        }
        JobKind::ReleaseHeldMessage => crate::message::held::release_step(state, job).await,
        JobKind::FirePluginTimer => crate::plugin::timer::fire_step(state, job).await,
        JobKind::SweepExpired => upkeep::sweep_expired(state, job).await,
        JobKind::RecountMembers => upkeep::recount_members(state, job).await,
        JobKind::RecordStats => upkeep::record_stats(state, job).await,
        JobKind::ForgetIcon => crate::icon::forget_step(state, job).await,
        JobKind::PurgeEvidence => crate::attachment::evidence::purge_step(state, job).await,
    }
}

/// What a runner keeps of its places: one per class, and those every class shares.
struct Places {
    own: Vec<Arc<Semaphore>>,
    shared: Arc<Semaphore>,
}

impl Places {
    fn new(shared: usize) -> Self {
        Self {
            own: JobClass::ALL
                .iter()
                .map(|_| Arc::new(Semaphore::new(1)))
                .collect(),
            shared: Arc::new(Semaphore::new(shared)),
        }
    }

    fn free(&self, class: JobClass) -> usize {
        self.own[class.rank() as usize].available_permits() + self.shared.available_permits()
    }

    /// A place for a job of `class`, given back when the permit drops: its class's own if free,
    /// else a shared one.
    fn take(&self, class: JobClass) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.own[class.rank() as usize]
            .clone()
            .try_acquire_owned()
            .or_else(|_| self.shared.clone().try_acquire_owned())
            .ok()
    }
}

/// Starts this server's runner, which claims and runs jobs for as long as the server runs.
pub fn spawn_runner(state: GlobalServerContext) {
    if !state.config.jobs.run {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = ensure_recurring(&state).await {
            tracing::error!("could not make sure of the recurring jobs: {e}");
        }
        let woken = Arc::new(Notify::new());
        listen_for_wakes(state.clone(), woken.clone());
        let places = Places::new(state.config.jobs.concurrency.max(1));
        let means = Means::find(&state).await;
        let groups = groups(&state, &means);
        loop {
            for class in JobClass::ALL.iter().copied() {
                for group in &groups {
                    let free = places.free(class).min(
                        group
                            .bound
                            .as_ref()
                            .map_or(usize::MAX, |b| b.available_permits()),
                    );
                    if free == 0 {
                        continue;
                    }
                    let claimed = match claim(&state, &group.kinds, class, free).await {
                        Ok(claimed) => claimed,
                        Err(e) => {
                            tracing::warn!("could not claim jobs: {e}");
                            continue;
                        }
                    };
                    for job in claimed {
                        // Claimed beyond the places free, which no one else took meanwhile: it
                        // waits for its lease, then any runner takes it.
                        let Some(place) = places.take(class) else {
                            continue;
                        };
                        let bound = match &group.bound {
                            Some(bound) => match bound.clone().try_acquire_owned() {
                                Ok(permit) => Some(permit),
                                Err(_) => continue,
                            },
                            None => None,
                        };
                        let state = state.clone();
                        let woken = woken.clone();
                        tokio::spawn(async move {
                            run(&state, job).await;
                            drop((place, bound));
                            woken.notify_one();
                        });
                    }
                }
            }
            let _ = tokio::time::timeout(POLL, woken.notified()).await;
        }
    });
}

/// Kinds a runner claims together, and the bound they share, if any.
struct Group {
    kinds: Vec<JobKind>,
    bound: Option<Arc<Semaphore>>,
}

/// The kinds this server runs, in groups: those [`bounded`], each group with its bound, and the
/// rest together.
fn groups(state: &GlobalServerContext, means: &Means) -> Vec<Group> {
    let runs = |kind: &JobKind| handles(state, means, *kind);
    let bounded = bounded(state);
    let mut groups: Vec<Group> = bounded
        .iter()
        .map(|(kinds, bound)| Group {
            kinds: kinds.iter().copied().filter(runs).collect(),
            bound: Some(Arc::new(Semaphore::new(*bound))),
        })
        .collect();
    groups.push(Group {
        kinds: JobKind::ALL
            .iter()
            .copied()
            .filter(runs)
            .filter(|kind| !bounded.iter().any(|(kinds, _)| kinds.contains(kind)))
            .collect(),
        bound: None,
    });
    groups.retain(|group| !group.kinds.is_empty());
    groups
}

/// Wakes the runner whenever any server publishes on [`WAKE_SUBJECT`].
fn listen_for_wakes(state: GlobalServerContext, woken: Arc<Notify>) {
    tokio::spawn(async move {
        loop {
            match state.nats_context.client().subscribe(WAKE_SUBJECT).await {
                Ok(mut wakes) => {
                    while wakes.next().await.is_some() {
                        woken.notify_one();
                    }
                }
                Err(e) => tracing::warn!("could not listen for job wakes: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

/// Claims up to `limit` of the jobs of `kinds` in `class` that are due, the longest due first:
/// one bounded walk of `job_next` per kind.
async fn claim(
    state: &GlobalServerContext,
    kinds: &[JobKind],
    class: JobClass,
    limit: usize,
) -> crate::Result<Vec<Claimed>> {
    if kinds.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let claimed: Vec<Claimed> = diesel::sql_query(
        r#"
        WITH picked AS (
            SELECT j.id FROM unnest($1::text[]) AS k(kind)
            CROSS JOIN LATERAL (
                SELECT id, not_before FROM job
                WHERE job.kind = k.kind AND job.class = $2
                  AND job.not_before <= now() AND job.failed_at IS NULL
                ORDER BY not_before, id
                LIMIT $3
                FOR UPDATE SKIP LOCKED
            ) j
            ORDER BY j.not_before, j.id
            LIMIT $3
        )
        UPDATE job SET not_before = now() + make_interval(secs => $4),
                       attempts = attempts + 1,
                       running_since = now()
        FROM picked WHERE job.id = picked.id
        RETURNING job.id, job.kind, job.key, job.class, job.due,
                  extract(epoch FROM job.every)::bigint AS every_seconds,
                  job.payload, job.progress, job.attempts
        "#,
    )
    .bind::<Array<Text>, _>(kinds)
    .bind::<SmallInt, _>(class.rank())
    .bind::<BigInt, _>(i64::try_from(limit).unwrap_or(i64::MAX))
    .bind::<Integer, _>(LEASE_SECONDS)
    .load(conn.as_mut())
    .await?;
    Ok(claimed)
}

/// Runs `job` step by step until it is done, waits, or fails, renewing its lease meanwhile.
async fn run(state: &GlobalServerContext, mut job: Claimed) {
    let started = std::time::Instant::now();
    let kind = job.kind.to_string();
    metrics::histogram!(aspen_metrics::api::JOB_WAIT_DURATION, "class" => job.class().to_string())
        .record((Utc::now() - job.due).num_milliseconds().max(0) as f64 / 1000.0);
    metrics::gauge!(aspen_metrics::api::JOBS_RUNNING, "kind" => kind.clone()).increment(1.0);
    let finished = loop {
        let outcome = {
            // A step is settled as a request is: what it published is answered if it rolls
            // back, and the rechecks of calls it noted run once it commits.
            let stepping = crate::events::settle_after(state, step(state, &job), Result::is_err);
            tokio::pin!(stepping);
            let mut renew = tokio::time::interval(RENEW_EVERY);
            renew.tick().await;
            loop {
                tokio::select! {
                    outcome = &mut stepping => break outcome,
                    _ = renew.tick() => {
                        if let Err(e) = renew_lease(state, job.id).await {
                            tracing::warn!(job = %job.id, "could not renew a job's lease: {e}");
                        }
                    }
                }
            }
        };
        match outcome {
            Ok(Outcome::Continue) => {
                job.attempts = 0;
                match reread_progress(state, job.id).await {
                    Ok(progress) => job.progress = progress,
                    Err(e) => break Err(e),
                }
            }
            Ok(Outcome::Progress(progress)) => {
                job.attempts = 0;
                let saved = async {
                    let mut conn = state.connection_pool.get().await?;
                    checkpoint(conn.as_mut(), job.id, &progress).await
                }
                .await;
                if let Err(e) = saved {
                    break Err(e);
                }
                job.progress = Some(progress);
            }
            Ok(Outcome::Done) => break finish(state, &job).await.map(|()| "done"),
            Ok(Outcome::Later(after)) => break wait(state, &job, after).await.map(|()| "later"),
            Err(e) => break Err(e),
        }
    };
    let outcome = match finished {
        Ok(outcome) => outcome,
        Err(e) => match fail(state, &job, &e).await {
            Ok(true) => "failed",
            Ok(false) => "retried",
            Err(saving) => {
                tracing::error!(job = %job.id, "could not record a job's failure ({e}): {saving}");
                "retried"
            }
        },
    };
    metrics::gauge!(aspen_metrics::api::JOBS_RUNNING, "kind" => kind.clone()).decrement(1.0);
    metrics::counter!(aspen_metrics::api::JOBS_FINISHED, "kind" => kind.clone(), "outcome" => outcome)
        .increment(1);
    metrics::histogram!(aspen_metrics::api::JOB_DURATION, "kind" => kind)
        .record(started.elapsed().as_secs_f64());
}

async fn renew_lease(state: &GlobalServerContext, job: Uuid) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    diesel::sql_query(
        "UPDATE job SET not_before = now() + make_interval(secs => $2) WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(job)
    .bind::<Integer, _>(LEASE_SECONDS)
    .execute(conn.as_mut())
    .await?;
    Ok(())
}

async fn reread_progress(
    state: &GlobalServerContext,
    job: Uuid,
) -> crate::Result<Option<serde_json::Value>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Jsonb>)]
        progress: Option<serde_json::Value>,
    }
    let mut conn = state.connection_pool.get().await?;
    let row: Row = diesel::sql_query("SELECT progress FROM job WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(job)
        .get_result(conn.as_mut())
        .await?;
    Ok(row.progress)
}

/// A one-shot job goes; a recurring one is due again a period after it was last due, as many
/// periods on as it takes to be in the future.
async fn finish(state: &GlobalServerContext, job: &Claimed) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    match job.every_seconds {
        None => {
            diesel::sql_query("DELETE FROM job WHERE id = $1")
                .bind::<diesel::sql_types::Uuid, _>(job.id)
                .execute(conn.as_mut())
                .await?;
        }
        Some(every) => {
            let every = every.max(1);
            let behind = (Utc::now() - job.due).num_seconds().max(0);
            let next = job.due + chrono::Duration::seconds((behind / every + 1) * every);
            diesel::sql_query(
                "UPDATE job SET due = $2, not_before = $2, attempts = 0, progress = NULL, \
                 running_since = NULL WHERE id = $1",
            )
            .bind::<diesel::sql_types::Uuid, _>(job.id)
            .bind::<Timestamptz, _>(next)
            .execute(conn.as_mut())
            .await?;
        }
    }
    Ok(())
}

/// Puts `job` back to wait `after`, counting no attempt.
async fn wait(state: &GlobalServerContext, job: &Claimed, after: Duration) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    diesel::sql_query(
        "UPDATE job SET not_before = now() + make_interval(secs => $2), \
         attempts = GREATEST(attempts - 1, 0), running_since = NULL WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(job.id)
    .bind::<diesel::sql_types::Double, _>(after.as_secs_f64())
    .execute(conn.as_mut())
    .await?;
    Ok(())
}

/// Records that a step of `job` failed: it is tried again after its backoff, or, its attempts
/// spent, given up. Answers whether it was given up.
async fn fail(
    state: &GlobalServerContext,
    job: &Claimed,
    error: &crate::Error,
) -> crate::Result<bool> {
    let given_up = job.attempts >= max_attempts(job.kind) && job.every_seconds.is_none();
    if given_up {
        tracing::error!(job = %job.id, kind = %job.kind, "a job was given up: {error}");
    } else {
        tracing::warn!(job = %job.id, kind = %job.kind, attempts = job.attempts, "a job's step failed: {error}");
    }
    let mut conn = state.connection_pool.get().await?;
    if given_up {
        diesel::sql_query(
            "UPDATE job SET failed_at = now(), error = $2, running_since = NULL WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(job.id)
        .bind::<Text, _>(error.to_string())
        .execute(conn.as_mut())
        .await?;
    } else {
        diesel::sql_query(
            "UPDATE job SET not_before = now() + make_interval(secs => $2), error = $3, \
             running_since = NULL WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(job.id)
        .bind::<diesel::sql_types::Double, _>(backoff(job.kind, job.attempts).as_secs_f64())
        .bind::<Text, _>(error.to_string())
        .execute(conn.as_mut())
        .await?;
    }
    Ok(given_up)
}

/// The most jobs the dashboard's preview lists, and of them the most given up.
pub const LISTED: i64 = 100;
pub const LISTED_FAILED: i64 = 10;
/// How far a count goes: a count this high reads as "this many or more".
pub const MAX_COUNTED: i64 = 1000;

/// A job as the dashboard lists it: what kind, how soon, since when, and how it is going, never
/// what it was given.
#[derive(Debug, Clone, QueryableByName)]
pub struct Listed {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    /// Its kind's name, as stored, so a kind this version does not know is still listed.
    #[diesel(sql_type = Text)]
    pub kind: String,
    #[diesel(sql_type = SmallInt)]
    class: i16,
    #[diesel(sql_type = Timestamptz)]
    pub due: DateTime<Utc>,
    #[diesel(sql_type = Nullable<Timestamptz>)]
    pub running_since: Option<DateTime<Utc>>,
    #[diesel(sql_type = Integer)]
    pub attempts: i32,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub recurring: bool,
    #[diesel(sql_type = Nullable<Timestamptz>)]
    pub failed_at: Option<DateTime<Utc>>,
    #[diesel(sql_type = Nullable<Text>)]
    pub error: Option<String>,
}

impl Listed {
    pub fn class(&self) -> JobClass {
        JobClass::of_rank(self.class)
    }
}

/// A preview of the deployment's jobs: those running, the next waiting by class and then by
/// how long they have waited, and the latest given up, at most [`LISTED`] in all; and how many
/// of each there are, each count stopping at [`MAX_COUNTED`]. Recurring jobs are listed only
/// while they run. Every part is a bounded walk of an index, so it costs the same however many
/// jobs wait.
pub struct Overview {
    pub running: Vec<Listed>,
    pub waiting: Vec<Listed>,
    pub failed: Vec<Listed>,
    pub running_count: i64,
    pub waiting_counts: Vec<(JobClass, i64)>,
    pub failed_count: i64,
}

const LISTED_COLUMNS: &str = "id, kind, class, due, running_since, attempts, \
     every IS NOT NULL AS recurring, failed_at, error";

/// The jobs preview; takes View jobs.
pub async fn overview(
    state: &GlobalServerContext,
    access: &crate::deployment::DeploymentAccess,
) -> crate::Result<Overview> {
    access.require(crate::deployment::DeploymentPermission::ViewJobs)?;
    let mut conn = state.connection_pool.get().await?;
    read_overview(conn.as_mut()).await
}

/// The jobs preview, for whoever has already been allowed it: the dashboard's reader, or the
/// operator at the terminal (`jobs list`).
pub async fn read_overview(conn: &mut AsyncPgConnection) -> crate::Result<Overview> {
    #[derive(QueryableByName)]
    struct Counted {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    let failed: Vec<Listed> = diesel::sql_query(format!(
        "SELECT {LISTED_COLUMNS} FROM job WHERE failed_at IS NOT NULL \
         ORDER BY failed_at DESC LIMIT $1"
    ))
    .bind::<BigInt, _>(LISTED_FAILED)
    .load(&mut *conn)
    .await?;
    let room = LISTED - failed.len() as i64;
    let running: Vec<Listed> = diesel::sql_query(format!(
        "SELECT {LISTED_COLUMNS} FROM job \
         WHERE running_since IS NOT NULL AND not_before > now() \
         ORDER BY running_since LIMIT $1"
    ))
    .bind::<BigInt, _>(room)
    .load(&mut *conn)
    .await?;
    let room = room - running.len() as i64;
    let waiting: Vec<Listed> = diesel::sql_query(format!(
        "SELECT {LISTED_COLUMNS} FROM job \
         WHERE failed_at IS NULL AND running_since IS NULL AND every IS NULL \
         ORDER BY class, due, id LIMIT $1"
    ))
    .bind::<BigInt, _>(room.max(0))
    .load(&mut *conn)
    .await?;
    let count = |sql: String| {
        diesel::sql_query(format!(
            "SELECT count(*) AS count FROM ({sql} LIMIT {MAX_COUNTED}) counted"
        ))
    };
    let running_count =
        count("SELECT 1 FROM job WHERE running_since IS NOT NULL AND not_before > now()".into())
            .get_result::<Counted>(&mut *conn)
            .await?
            .count;
    let failed_count = count("SELECT 1 FROM job WHERE failed_at IS NOT NULL".into())
        .get_result::<Counted>(&mut *conn)
        .await?
        .count;
    let mut waiting_counts = Vec::with_capacity(JobClass::ALL.len());
    for class in JobClass::ALL.iter().copied() {
        let counted = count(format!(
            "SELECT 1 FROM job WHERE failed_at IS NULL AND running_since IS NULL \
             AND every IS NULL AND class = {}",
            class.rank()
        ))
        .get_result::<Counted>(&mut *conn)
        .await?;
        waiting_counts.push((class, counted.count));
    }
    Ok(Overview {
        running,
        waiting,
        failed,
        running_count,
        waiting_counts,
        failed_count,
    })
}

/// Tries a job given up again, from where it stood: for the operator (`jobs retry`). Answers
/// whether there was such a job.
pub async fn retry(conn: &mut AsyncPgConnection, id: Uuid) -> crate::Result<bool> {
    let retried = diesel::sql_query(
        "UPDATE job SET failed_at = NULL, error = NULL, attempts = 0, not_before = now() \
         WHERE id = $1 AND failed_at IS NOT NULL",
    )
    .bind::<diesel::sql_types::Uuid, _>(id)
    .execute(conn)
    .await?;
    Ok(retried > 0)
}

/// Deletes a job, waiting or given up, so it is never run: for the operator (`jobs cancel`). A
/// job running now finishes its step. Answers whether there was such a job.
pub async fn cancel(conn: &mut AsyncPgConnection, id: Uuid) -> crate::Result<bool> {
    let cancelled = diesel::sql_query("DELETE FROM job WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(id)
        .execute(conn)
        .await?;
    Ok(cancelled > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_job_waits_longer_each_time_up_to_an_hour() {
        let kind = JobKind::DeleteMessagesBy;
        assert_eq!(backoff(kind, 1), Duration::from_secs(10));
        assert_eq!(backoff(kind, 2), Duration::from_secs(20));
        assert_eq!(backoff(kind, 4), Duration::from_secs(80));
        assert_eq!(backoff(kind, 20), Duration::from_secs(3600));
        assert_eq!(backoff(JobKind::SendEmail, 2), Duration::from_secs(120));
    }

    #[test]
    fn classes_are_stored_by_rank() {
        for class in JobClass::ALL {
            assert_eq!(JobClass::of_rank(class.rank()), *class);
        }
        assert!(JobClass::Urgent.rank() < JobClass::Maintenance.rank());
    }
}
