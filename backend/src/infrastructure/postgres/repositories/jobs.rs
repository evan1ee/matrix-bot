//! The durable job queue (`jobs`, migration 0003).
//!
//! Every transition after `claim` is guarded by `locked_by = worker AND
//! status = 'running'`: a worker whose job was reaped and re-claimed can
//! never overwrite the new owner's state — the call returns `false`.

use std::time::Duration;

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::jobs::{truncate_error, JobId, JobPayload, NewJob};

/// The channel the 0003 trigger NOTIFYs on every inserted job.
pub const JOBS_CHANNEL: &str = "jobs";

#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub id: JobId,
    pub job_type: String,
    pub account_id: Option<AccountId>,
    pub payload: serde_json::Value,
    pub attempts: i32,
    pub max_attempts: i32,
    pub backoff_seconds: i32,
}

struct ClaimedRow {
    id: i64,
    job_type: String,
    account_id: Option<uuid::Uuid>,
    payload: serde_json::Value,
    attempts: i32,
    max_attempts: i32,
    backoff_seconds: i32,
}

impl From<ClaimedRow> for ClaimedJob {
    fn from(r: ClaimedRow) -> Self {
        Self {
            id: JobId(r.id),
            job_type: r.job_type,
            account_id: r.account_id.map(AccountId::from_uuid),
            payload: r.payload,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            backoff_seconds: r.backoff_seconds,
        }
    }
}

fn payload_json(payload: &JobPayload) -> sqlx::Result<serde_json::Value> {
    let value = match payload {
        JobPayload::ReminderFire(p) => serde_json::to_value(p),
        JobPayload::BriefingSend(p) => serde_json::to_value(p),
        JobPayload::Empty => Ok(serde_json::json!({})),
    };
    value.map_err(|e| sqlx::Error::Encode(Box::new(e)))
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

/// Inserts `job` unless one with the same `(job_type, dedupe_key)` exists
/// (in any state). `None` means it was a duplicate.
pub async fn enqueue(exec: impl PgExecutor<'_>, job: &NewJob) -> sqlx::Result<Option<JobId>> {
    let payload = payload_json(job.payload())?;
    let id = sqlx::query_scalar!(
        r#"
        INSERT INTO jobs (job_type, account_id, payload, priority, max_attempts,
                          backoff_seconds, available_at, dedupe_key)
        VALUES ($1, $2, $3, $4, $5, $6, COALESCE($7::timestamptz, NOW()), $8)
        ON CONFLICT (job_type, dedupe_key) DO NOTHING
        RETURNING id
        "#,
        job.job_type().as_str(),
        job.account_id().map(|a| *a.as_uuid()),
        payload,
        job.priority(),
        job.max_attempts(),
        job.backoff_seconds(),
        job.available_at(),
        job.dedupe_key(),
    )
    .fetch_optional(exec)
    .await?;
    Ok(id.map(JobId))
}

/// The next due job of one of `types`, `FOR UPDATE SKIP LOCKED` so
/// concurrent claimers never block on or double-take a row. Claiming spends
/// an attempt.
pub async fn claim(
    exec: impl PgExecutor<'_>,
    worker: &str,
    types: &[&str],
) -> sqlx::Result<Option<ClaimedJob>> {
    let row = sqlx::query_as!(
        ClaimedRow,
        r#"
        WITH next_job AS (
            SELECT id FROM jobs
            WHERE status = 'pending' AND available_at <= NOW() AND job_type = ANY($2::text[])
            ORDER BY priority DESC, available_at ASC, id ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE jobs
        SET status = 'running', locked_at = NOW(), locked_by = $1, started_at = NOW(),
            heartbeat_at = NOW(), attempts = jobs.attempts + 1, updated_at = NOW()
        FROM next_job
        WHERE jobs.id = next_job.id
        RETURNING jobs.id, jobs.job_type, jobs.account_id, jobs.payload,
                  jobs.attempts, jobs.max_attempts, jobs.backoff_seconds
        "#,
        worker,
        types as &[&str],
    )
    .fetch_optional(exec)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn heartbeat(exec: impl PgExecutor<'_>, id: JobId, worker: &str) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE jobs SET heartbeat_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        id.0,
        worker
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn complete(exec: impl PgExecutor<'_>, id: JobId, worker: &str) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE jobs SET status = 'completed', completed_at = NOW(), heartbeat_at = NULL, \
                locked_at = NULL, updated_at = NOW() \
         WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        id.0,
        worker
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// A failed attempt: pending again after `delay`, or `failed` once
/// `attempts >= max_attempts`.
pub async fn retry(
    exec: impl PgExecutor<'_>,
    id: JobId,
    worker: &str,
    error: &str,
    delay: Duration,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        r#"
        UPDATE jobs SET
            status       = CASE WHEN attempts >= max_attempts THEN 'failed' ELSE 'pending' END,
            available_at = CASE WHEN attempts >= max_attempts THEN available_at
                                ELSE NOW() + make_interval(secs => $4::float8) END,
            completed_at = CASE WHEN attempts >= max_attempts THEN NOW() END,
            error = $3, locked_at = NULL, locked_by = NULL, heartbeat_at = NULL, updated_at = NOW()
        WHERE id = $1 AND locked_by = $2 AND status = 'running'
        "#,
        id.0,
        worker,
        truncate_error(error),
        secs(delay)
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// A dependency is down: try again after `delay` without spending the attempt.
pub async fn defer(
    exec: impl PgExecutor<'_>,
    id: JobId,
    worker: &str,
    delay: Duration,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE jobs SET status = 'pending', attempts = attempts - 1, \
                available_at = NOW() + make_interval(secs => $3::float8), \
                locked_at = NULL, locked_by = NULL, heartbeat_at = NULL, updated_at = NOW() \
         WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        id.0,
        worker,
        secs(delay)
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn fail(
    exec: impl PgExecutor<'_>,
    id: JobId,
    worker: &str,
    error: &str,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE jobs SET status = 'failed', completed_at = NOW(), error = $3, \
                locked_at = NULL, locked_by = NULL, heartbeat_at = NULL, updated_at = NOW() \
         WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        id.0,
        worker,
        truncate_error(error)
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Shutdown: hand the job back untouched (the attempt is not spent).
pub async fn release(exec: impl PgExecutor<'_>, id: JobId, worker: &str) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE jobs SET status = 'pending', attempts = attempts - 1, available_at = NOW(), \
                locked_at = NULL, locked_by = NULL, heartbeat_at = NULL, updated_at = NOW() \
         WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        id.0,
        worker
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Reaper: a running job whose heartbeat is older than `older_than` lost its
/// worker. The attempt counts.
pub async fn reap_stale(exec: impl PgExecutor<'_>, older_than: Duration) -> sqlx::Result<u64> {
    let r = sqlx::query!(
        r#"
        UPDATE jobs SET
            status       = CASE WHEN attempts >= max_attempts THEN 'failed' ELSE 'pending' END,
            completed_at = CASE WHEN attempts >= max_attempts THEN NOW() END,
            error        = 'worker lost: heartbeat stale',
            available_at = NOW(), locked_at = NULL, locked_by = NULL, heartbeat_at = NULL,
            updated_at   = NOW()
        WHERE status = 'running' AND heartbeat_at < NOW() - make_interval(secs => $1::float8)
        "#,
        secs(older_than)
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected())
}

pub async fn purge_finished(exec: impl PgExecutor<'_>, older_than: Duration) -> sqlx::Result<u64> {
    let r = sqlx::query!(
        "DELETE FROM jobs WHERE status IN ('completed', 'failed', 'cancelled') \
           AND completed_at < NOW() - make_interval(secs => $1::float8)",
        secs(older_than)
    )
    .execute(exec)
    .await?;
    Ok(r.rows_affected())
}

/// Status of the job holding a dedupe key, if any. Lets a use case tell
/// "already queued" from "queued and already failed".
pub async fn status_by_key(
    exec: impl PgExecutor<'_>,
    job_type: &str,
    dedupe_key: &str,
) -> sqlx::Result<Option<crate::domain::jobs::JobStatus>> {
    let status = sqlx::query_scalar!(
        "SELECT status FROM jobs WHERE job_type = $1 AND dedupe_key = $2",
        job_type,
        dedupe_key
    )
    .fetch_optional(exec)
    .await?;
    Ok(status.map(|s| crate::domain::jobs::JobStatus::parse(&s).expect("status CHECK")))
}

/// Whether a job of `job_type` is still queued or running. Lets a recurring
/// tick skip enqueuing work that an unfinished job will already cover.
pub async fn has_open(exec: impl PgExecutor<'_>, job_type: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS(
               SELECT 1 FROM jobs WHERE job_type = $1 AND status IN ('pending', 'running')
           ) AS "open!""#,
        job_type
    )
    .fetch_one(exec)
    .await
}

/// Forces a keyed job to `failed`, as a timeout or the reaper would.
#[cfg(test)]
pub async fn force_failed(
    exec: impl PgExecutor<'_>,
    job_type: &str,
    dedupe_key: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE jobs SET status = 'failed', completed_at = NOW() \
         WHERE job_type = $1 AND dedupe_key = $2",
        job_type,
        dedupe_key
    )
    .execute(exec)
    .await?;
    Ok(())
}
/// A full `jobs` row as tests assert on it.
#[cfg(test)]
pub struct JobRecord {
    pub id: JobId,
    pub job_type: String,
    pub status: crate::domain::jobs::JobStatus,
    pub attempts: i32,
    pub available_at: chrono::DateTime<chrono::Utc>,
    pub locked_by: Option<String>,
    pub error: Option<String>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub account_id: Option<AccountId>,
}

#[cfg(test)]
pub async fn get(exec: impl PgExecutor<'_>, id: JobId) -> sqlx::Result<Option<JobRecord>> {
    let row = sqlx::query!(
        "SELECT id, job_type, status, attempts, available_at, locked_by, error, completed_at, account_id \
         FROM jobs WHERE id = $1",
        id.0
    )
    .fetch_optional(exec)
    .await?;
    Ok(row.map(|r| JobRecord {
        id: JobId(r.id),
        job_type: r.job_type,
        status: crate::domain::jobs::JobStatus::parse(&r.status).expect("status CHECK"),
        attempts: r.attempts,
        available_at: r.available_at,
        locked_by: r.locked_by,
        error: r.error,
        completed_at: r.completed_at,
        account_id: r.account_id.map(AccountId::from_uuid),
    }))
}

#[cfg(test)]
pub async fn count_with_type(exec: impl PgExecutor<'_>, job_type: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!" FROM jobs WHERE job_type = $1"#,
        job_type
    )
    .fetch_one(exec)
    .await
}

/// Pushes `heartbeat_at` and `completed_at` `secs` into the past.
#[cfg(test)]
pub async fn backdate(exec: impl PgExecutor<'_>, id: JobId, secs: f64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE jobs SET heartbeat_at = heartbeat_at - make_interval(secs => $2::float8), \
                completed_at = completed_at - make_interval(secs => $2::float8) \
         WHERE id = $1",
        id.0,
        secs
    )
    .execute(exec)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::PgPool;

    use super::*;
    use crate::domain::jobs::{JobId, JobStatus, JobType, NewJob};

    const W1: &str = "w1";
    const W2: &str = "w2";
    const T: JobType = JobType::PortfolioCheckAlerts;
    const TYPES: &[&str] = &["portfolio.check_alerts"];

    async fn pool() -> PgPool {
        let p = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&p).await;
        p
    }

    /// Makes a retried job due again without sleeping.
    async fn backdate_available(pool: &PgPool, id: JobId) {
        sqlx::query!("UPDATE jobs SET available_at = NOW() WHERE id = $1", id.0)
            .execute(pool)
            .await
            .unwrap();
    }

    /// Given a keyed job, When enqueued twice, Then the second returns None
    /// and one row exists; un-keyed jobs always insert.
    #[tokio::test]
    async fn enqueue_dedupes_on_type_and_key() {
        let pool = pool().await;
        let job = NewJob::test_job(T, Some("k"));
        assert!(enqueue(&pool, &job).await.unwrap().is_some());
        assert!(enqueue(&pool, &job).await.unwrap().is_none());
        enqueue(&pool, &NewJob::test_job(T, None))
            .await
            .unwrap()
            .unwrap();
        enqueue(&pool, &NewJob::test_job(T, None))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count_with_type(&pool, T.as_str()).await.unwrap(), 3);
    }

    /// Given jobs of priority 0 and 5, When claimed, Then priority wins, then
    /// the earlier available_at; claiming sets running/attempts=1/locked_by.
    #[tokio::test]
    async fn claim_orders_by_priority_then_available_at() {
        let pool = pool().await;
        let now = chrono::Utc::now();
        let old = enqueue(
            &pool,
            &NewJob::test_job(T, Some("old")).with_available_at(now - chrono::Duration::minutes(5)),
        )
        .await
        .unwrap()
        .unwrap();
        let new = enqueue(
            &pool,
            &NewJob::test_job(T, Some("new")).with_available_at(now - chrono::Duration::minutes(1)),
        )
        .await
        .unwrap()
        .unwrap();
        let urgent = enqueue(&pool, &NewJob::test_job(T, Some("urgent")).with_priority(5))
            .await
            .unwrap()
            .unwrap();

        let order: Vec<JobId> = vec![
            claim(&pool, W1, TYPES).await.unwrap().unwrap().id,
            claim(&pool, W1, TYPES).await.unwrap().unwrap().id,
            claim(&pool, W1, TYPES).await.unwrap().unwrap().id,
        ];
        assert_eq!(order, vec![urgent, old, new]);
        let rec = get(&pool, urgent).await.unwrap().unwrap();
        assert_eq!(
            (rec.status, rec.attempts, rec.locked_by.as_deref()),
            (JobStatus::Running, 1, Some(W1))
        );
        assert!(claim(&pool, W1, TYPES).await.unwrap().is_none());
    }

    /// Given a job available in the future or of an unregistered type, When
    /// claimed, Then nothing is returned.
    #[tokio::test]
    async fn claim_skips_future_and_foreign_types() {
        let pool = pool().await;
        enqueue(
            &pool,
            &NewJob::test_job(T, Some("later"))
                .with_available_at(chrono::Utc::now() + chrono::Duration::hours(1)),
        )
        .await
        .unwrap();
        enqueue(
            &pool,
            &NewJob::test_job(JobType::BriefingSend, Some("other")),
        )
        .await
        .unwrap();
        assert!(claim(&pool, W1, TYPES).await.unwrap().is_none());
    }

    /// Given 200 jobs, When four claimers race on separate connections and
    /// threads, Then every job is claimed exactly once (SKIP LOCKED).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_claimers_never_get_the_same_job() {
        const JOBS: usize = 200;
        const CLAIMERS: usize = 4;
        let pool = crate::db::pool::test_pool_sized(CLAIMERS as u32).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        for i in 0..JOBS {
            enqueue(&pool, &NewJob::test_job(T, Some(&format!("k{i}"))))
                .await
                .unwrap();
        }
        // The barrier releases every claimer at once so they genuinely race.
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(CLAIMERS));
        let handles: Vec<_> = (0..CLAIMERS)
            .map(|n| {
                let (pool, start) = (pool.clone(), start.clone());
                tokio::spawn(async move {
                    let worker = format!("w{n}");
                    start.wait().await;
                    let mut got = Vec::new();
                    while let Some(j) = claim(&pool, &worker, TYPES).await.unwrap() {
                        got.push(j.id);
                    }
                    got
                })
            })
            .collect();
        let mut all = Vec::new();
        for h in handles {
            all.extend(h.await.unwrap());
        }
        let claimed = all.len();
        all.sort_by_key(|j| j.0);
        all.dedup();
        assert_eq!(all.len(), JOBS, "every job is claimed");
        assert_eq!(claimed, JOBS, "no job may be claimed twice");
    }

    /// Given a claimed job with max 2 and backoff 0, When retried twice, Then
    /// the first retry makes it pending again and the second fails it with
    /// the (truncated) error and completed_at.
    #[tokio::test]
    async fn retry_backs_off_then_fails_after_max_attempts() {
        let pool = pool().await;
        let id = enqueue(&pool, &NewJob::test_job(T, Some("r")).with_policy(2, 0))
            .await
            .unwrap()
            .unwrap();
        claim(&pool, W1, TYPES).await.unwrap().unwrap();
        assert!(retry(&pool, id, W1, "boom", Duration::from_secs(60))
            .await
            .unwrap());
        let rec = get(&pool, id).await.unwrap().unwrap();
        assert_eq!(rec.status, JobStatus::Pending);
        assert!(rec.available_at > chrono::Utc::now() + chrono::Duration::seconds(50));
        assert!(rec.locked_by.is_none());
        backdate_available(&pool, id).await;
        claim(&pool, W1, TYPES).await.unwrap().unwrap();
        let long = "x".repeat(5_000);
        assert!(retry(&pool, id, W1, &long, Duration::ZERO).await.unwrap());
        let rec = get(&pool, id).await.unwrap().unwrap();
        assert_eq!((rec.status, rec.attempts), (JobStatus::Failed, 2));
        assert_eq!(rec.error.unwrap().chars().count(), 2_000);
    }

    /// Given a claimed job, When deferred or released, Then it is pending with
    /// attempts restored to 0 and no error recorded.
    #[tokio::test]
    async fn defer_and_release_do_not_spend_attempts() {
        let pool = pool().await;
        let id = enqueue(&pool, &NewJob::test_job(T, Some("d")))
            .await
            .unwrap()
            .unwrap();
        claim(&pool, W1, TYPES).await.unwrap().unwrap();
        assert!(defer(&pool, id, W1, Duration::from_secs(30)).await.unwrap());
        let rec = get(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            (rec.status, rec.attempts, rec.error),
            (JobStatus::Pending, 0, None)
        );
        assert!(rec.available_at > chrono::Utc::now() + chrono::Duration::seconds(20));

        let id2 = enqueue(&pool, &NewJob::test_job(T, Some("rel")))
            .await
            .unwrap()
            .unwrap();
        // The deferred job is not due for 30 s, so this claims id2.
        claim(&pool, W1, TYPES).await.unwrap().unwrap();
        assert!(release(&pool, id2, W1).await.unwrap());
        let rec = get(&pool, id2).await.unwrap().unwrap();
        assert_eq!((rec.status, rec.attempts), (JobStatus::Pending, 0));
    }

    /// Given a job claimed by W1, When W2 tries every guarded transition, Then
    /// each returns false and the row is untouched.
    #[tokio::test]
    async fn guarded_transitions_refuse_a_foreign_worker() {
        let pool = pool().await;
        let id = enqueue(&pool, &NewJob::test_job(T, Some("g")))
            .await
            .unwrap()
            .unwrap();
        claim(&pool, W1, TYPES).await.unwrap().unwrap();
        assert!(!heartbeat(&pool, id, W2).await.unwrap());
        assert!(!complete(&pool, id, W2).await.unwrap());
        assert!(!retry(&pool, id, W2, "e", Duration::ZERO).await.unwrap());
        assert!(!defer(&pool, id, W2, Duration::ZERO).await.unwrap());
        assert!(!fail(&pool, id, W2, "e").await.unwrap());
        assert!(!release(&pool, id, W2).await.unwrap());
        let rec = get(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            (rec.status, rec.locked_by.as_deref()),
            (JobStatus::Running, Some(W1))
        );
        assert!(heartbeat(&pool, id, W1).await.unwrap());
        assert!(complete(&pool, id, W1).await.unwrap());
        assert_eq!(
            get(&pool, id).await.unwrap().unwrap().status,
            JobStatus::Completed
        );
        assert!(
            !complete(&pool, id, W1).await.unwrap(),
            "a finished job is not running any more"
        );
    }

    /// Given two running jobs with stale heartbeats (one on its last attempt)
    /// and one fresh, When reaped, Then the first is pending again, the
    /// second failed, the fresh one untouched.
    #[tokio::test]
    async fn reap_stale_requeues_or_fails_by_attempts() {
        let pool = pool().await;
        let again = enqueue(&pool, &NewJob::test_job(T, Some("again")).with_policy(3, 0))
            .await
            .unwrap()
            .unwrap();
        let last = enqueue(&pool, &NewJob::test_job(T, Some("last")).with_policy(1, 0))
            .await
            .unwrap()
            .unwrap();
        let fresh = enqueue(&pool, &NewJob::test_job(T, Some("fresh")))
            .await
            .unwrap()
            .unwrap();
        for _ in 0..3 {
            claim(&pool, W1, TYPES).await.unwrap().unwrap();
        }
        backdate(&pool, again, 600.0).await.unwrap();
        backdate(&pool, last, 600.0).await.unwrap();
        assert_eq!(
            reap_stale(&pool, Duration::from_secs(120)).await.unwrap(),
            2
        );
        let requeued = get(&pool, again).await.unwrap().unwrap();
        assert_eq!(
            (requeued.status, requeued.attempts),
            (JobStatus::Pending, 1)
        );
        assert!(requeued.completed_at.is_none());
        let failed = get(&pool, last).await.unwrap().unwrap();
        assert_eq!(failed.status, JobStatus::Failed);
        assert!(failed.completed_at.is_some());
        assert!(failed.error.unwrap().contains("heartbeat"));
        assert_eq!(
            get(&pool, fresh).await.unwrap().unwrap().status,
            JobStatus::Running
        );
        assert!(
            !complete(&pool, again, W1).await.unwrap(),
            "the reaped owner lost the job"
        );
    }

    /// Given a job completed 8 days ago and one completed now, When purging
    /// older than 7 days, Then only the old one is deleted; pending jobs stay.
    #[tokio::test]
    async fn purge_finished_deletes_only_old_terminal_jobs() {
        let pool = pool().await;
        let old = enqueue(&pool, &NewJob::test_job(T, Some("old")))
            .await
            .unwrap()
            .unwrap();
        let recent = enqueue(&pool, &NewJob::test_job(T, Some("recent")))
            .await
            .unwrap()
            .unwrap();
        for id in [old, recent] {
            claim(&pool, W1, TYPES).await.unwrap().unwrap();
            assert!(complete(&pool, id, W1).await.unwrap());
        }
        let pending = enqueue(&pool, &NewJob::test_job(T, Some("pending")))
            .await
            .unwrap()
            .unwrap();
        backdate(&pool, old, 8.0 * 86_400.0).await.unwrap();
        assert_eq!(
            purge_finished(&pool, Duration::from_secs(7 * 86_400))
                .await
                .unwrap(),
            1
        );
        assert!(get(&pool, old).await.unwrap().is_none());
        assert!(get(&pool, recent).await.unwrap().is_some());
        assert!(get(&pool, pending).await.unwrap().is_some());
    }

    /// Given a job with an account and a payload, When claimed, Then both come
    /// back as enqueued.
    #[tokio::test]
    async fn claim_returns_account_and_payload() {
        let pool = pool().await;
        let owner = AccountId::from("acct_alice");
        crate::db::pool::seed_account(&pool, &owner).await;
        let t = chrono::Utc::now();
        enqueue(&pool, &NewJob::reminder_fire(owner, 9, t))
            .await
            .unwrap()
            .unwrap();
        let job = claim(&pool, W1, &["reminder.fire"]).await.unwrap().unwrap();
        assert_eq!(job.account_id, Some(owner));
        let p: crate::domain::jobs::ReminderFirePayload =
            serde_json::from_value(job.payload).unwrap();
        assert_eq!(p.reminder_id, 9);
        assert_eq!((job.max_attempts, job.backoff_seconds), (5, 30));
    }
}
