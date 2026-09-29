//! The job runtime: workers that claim and run durable jobs, a reaper for
//! jobs whose worker died, a purge of old finished jobs, and the
//! leader-elected scheduler that enqueues recurring work.
//!
//! Spawned once by `app::run` under a child `CancellationToken` and joined
//! in its bounded shutdown. Holds no SQL (`tests/architecture.rs`): every
//! query is in `infrastructure/postgres/repositories/jobs.rs`.

pub(crate) mod reaper;
pub mod registry;
pub mod schedule;
pub(crate) mod scheduler;
#[cfg(test)]
pub(crate) mod testing;
pub(crate) mod wakeup;
pub(crate) mod worker;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::jobs::{JobId, JobType};
pub use registry::Registry;
pub use schedule::Tick;

pub const DEFAULT_JOB_TIMEOUT: Duration = Duration::from_secs(300);

/// What a handler knows about the job it runs.
///
/// `cancel` fires on shutdown (the worker then waits `shutdown_grace`) or when
/// the worker lost the job to the reaper. A handler that stops because of it
/// should return `JobError::Unavailable(Duration::ZERO)`, which hands the job
/// back without spending the attempt.
pub struct JobContext {
    pub job_id: JobId,
    /// The job row's account (follows an account merge); `None` for global jobs.
    pub account_id: Option<AccountId>,
    pub attempt: i32,
    pub max_attempts: i32,
    pub cancel: CancellationToken,
}

impl JobContext {
    pub fn is_last_attempt(&self) -> bool {
        self.attempt >= self.max_attempts
    }
}

#[derive(Debug)]
pub enum JobError {
    /// Counts an attempt; retried with backoff, failed after `max_attempts`.
    Retryable(anyhow::Error),
    /// Fails the job now.
    Permanent(anyhow::Error),
    /// A dependency is down (e.g. Matrix disconnected): retry after the delay
    /// without spending an attempt.
    Unavailable(Duration),
}

impl From<AppError> for JobError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Database(_) | AppError::External { .. } => JobError::Retryable(err.into()),
            AppError::Validation(_)
            | AppError::NotFound(_)
            | AppError::Forbidden(_)
            | AppError::Conflict(_) => JobError::Permanent(err.into()),
        }
    }
}

#[async_trait]
pub trait JobHandler: Send + Sync {
    fn job_type(&self) -> JobType;
    /// Upper bound on one run; exceeding it is a retryable failure.
    fn timeout(&self) -> Duration {
        DEFAULT_JOB_TIMEOUT
    }
    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError>;
}

#[derive(Debug, Clone)]
pub struct JobsConfig {
    pub workers: usize,
    pub worker_host: String,
    pub poll_interval: Duration,
    pub heartbeat_interval: Duration,
    pub stale_after: Duration,
    pub reap_interval: Duration,
    pub purge_interval: Duration,
    pub purge_after: Duration,
    pub shutdown_grace: Duration,
    pub leader_retry: Duration,
    pub scheduler_lock_key: i64,
}

impl JobsConfig {
    pub fn production(workers: usize, worker_host: String) -> Self {
        Self {
            workers,
            worker_host,
            poll_interval: Duration::from_secs(5),
            heartbeat_interval: Duration::from_secs(30),
            stale_after: Duration::from_secs(120),
            reap_interval: Duration::from_secs(60),
            purge_interval: Duration::from_secs(3_600),
            purge_after: Duration::from_secs(7 * 86_400),
            shutdown_grace: Duration::from_secs(10),
            leader_retry: Duration::from_secs(30),
            scheduler_lock_key: SCHEDULER_LOCK_KEY,
        }
    }

    /// Short intervals so runtime tests finish in seconds. The advisory-lock
    /// key is random per call: advisory locks are database-wide, so tests
    /// sharing a server must not contend for one key.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            workers: 1,
            worker_host: "test".to_string(),
            poll_interval: Duration::from_millis(100),
            heartbeat_interval: Duration::from_millis(200),
            stale_after: Duration::from_secs(1),
            reap_interval: Duration::from_millis(200),
            purge_interval: Duration::from_secs(3_600),
            purge_after: Duration::from_secs(7 * 86_400),
            shutdown_grace: Duration::from_millis(300),
            leader_retry: Duration::from_millis(100),
            scheduler_lock_key: uuid::Uuid::new_v4().as_u64_pair().0 as i64,
        }
    }
}

/// `pg_try_advisory_lock` key of the scheduler ("jobsschd" in ASCII).
pub const SCHEDULER_LOCK_KEY: i64 = 0x6a6f_6273_7363_6864;

pub fn worker_id(host: &str) -> String {
    format!(
        "{host}:{}:{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    )
}

pub struct JobRuntime {
    pool: PgPool,
    services: AppServices,
    registry: Arc<Registry>,
    ticks: Vec<Tick>,
    config: Arc<JobsConfig>,
}

impl JobRuntime {
    pub fn new(
        pool: PgPool,
        services: AppServices,
        registry: Registry,
        ticks: Vec<Tick>,
        config: JobsConfig,
    ) -> Self {
        Self {
            pool,
            services,
            registry: Arc::new(registry),
            ticks,
            config: Arc::new(config),
        }
    }

    /// Runs until `shutdown` fires, then waits for every task (workers get
    /// `shutdown_grace` to finish or release their job). Any task ending
    /// before shutdown is a bug: it is logged and the runtime stops, which
    /// `app::run`'s supervision turns into a process restart.
    pub async fn run(self, shutdown: CancellationToken) {
        let wake = Arc::new(Notify::new());
        let mut tasks = JoinSet::new();
        tasks.spawn(wakeup::run(
            self.pool.clone(),
            wake.clone(),
            Duration::from_secs(30),
            shutdown.clone(),
        ));
        tasks.spawn(reaper::run(
            self.pool.clone(),
            self.config.clone(),
            shutdown.clone(),
        ));
        tasks.spawn(scheduler::run(
            self.pool.clone(),
            self.services.clone(),
            self.ticks.clone(),
            self.config.clone(),
            shutdown.clone(),
        ));
        for _ in 0..self.config.workers {
            let w = worker::Worker {
                id: worker_id(&self.config.worker_host),
                pool: self.pool.clone(),
                registry: self.registry.clone(),
                config: self.config.clone(),
                wake: wake.clone(),
            };
            tasks.spawn(worker::run(w, shutdown.clone()));
        }
        info!(
            workers = self.config.workers,
            types = ?self.registry.type_names(),
            "Job runtime started"
        );
        while let Some(result) = tasks.join_next().await {
            if !shutdown.is_cancelled() {
                error!(
                    ?result,
                    "A job runtime task ended before shutdown; stopping the runtime"
                );
                shutdown.cancel();
            }
        }
        info!("Job runtime stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::jobs::{JobStatus, NewJob};

    /// Given each AppError variant, When converted, Then infrastructure
    /// failures retry and input/ownership failures are permanent.
    #[test]
    fn app_errors_map_to_retry_or_permanent() {
        assert!(matches!(
            JobError::from(AppError::Database(sqlx::Error::PoolTimedOut)),
            JobError::Retryable(_)
        ));
        assert!(matches!(
            JobError::from(AppError::External {
                service: "x",
                source: anyhow::anyhow!("down")
            }),
            JobError::Retryable(_)
        ));
        for e in [
            AppError::Validation("v".into()),
            AppError::NotFound("n".into()),
            AppError::Forbidden("f".into()),
            AppError::Conflict("c".into()),
        ] {
            assert!(matches!(JobError::from(e), JobError::Permanent(_)));
        }
    }

    #[test]
    fn worker_ids_are_unique_and_carry_host_and_pid() {
        let a = worker_id("host");
        let b = worker_id("host");
        assert_ne!(a, b);
        assert!(a.starts_with(&format!("host:{}:", std::process::id())));
    }

    #[test]
    fn the_last_attempt_is_detected() {
        let ctx = |attempt| JobContext {
            job_id: JobId(1),
            account_id: None,
            attempt,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        assert!(!ctx(2).is_last_attempt());
        assert!(ctx(3).is_last_attempt());
    }

    /// Given a runtime with one scripted handler and no ticks, When a job is
    /// enqueued, Then it completes; When shutdown fires, Then run returns
    /// within 2 s.
    #[tokio::test]
    async fn the_runtime_runs_jobs_and_stops_on_cancel() {
        let h = testing::Harness::new().await;
        let registry = Registry::new(vec![Arc::new(testing::Scripted::new(
            testing::Behaviour::Succeed,
        ))])
        .unwrap();
        let token = CancellationToken::new();
        let runtime = JobRuntime::new(
            h.pool.clone(),
            AppServices::new(&h.pool),
            registry,
            Vec::new(),
            JobsConfig::for_tests(),
        );
        let task = tokio::spawn(runtime.run(token.clone()));
        let id = h
            .enqueue(NewJob::test_job(testing::TEST_TYPE, Some("e2e")))
            .await;
        h.wait_for_status(id, JobStatus::Completed).await;
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
}
