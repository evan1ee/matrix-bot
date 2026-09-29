//! Test scaffolding for the job runtime: a scriptable handler and a harness
//! that runs one worker against a private test schema.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::worker::{self, Worker};
use super::{
    worker_id, JobContext, JobError, JobHandler, JobsConfig, Registry, DEFAULT_JOB_TIMEOUT,
};
use crate::domain::jobs::{JobId, JobStatus, JobType, NewJob};
use crate::infrastructure::postgres::repositories::jobs::{self as repo, JobRecord};

pub(crate) const TEST_TYPE: JobType = JobType::PortfolioCheckAlerts;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Behaviour {
    Succeed,
    Retry,
    Permanent,
    Unavailable(Duration),
    Sleep(Duration),
    IgnoreCancel,
    UntilCancelled,
}

#[derive(Clone)]
pub(crate) struct Scripted {
    behaviour: Behaviour,
    timeout: Duration,
    runs: Arc<AtomicUsize>,
}

impl Scripted {
    pub fn new(b: Behaviour) -> Self {
        Self {
            behaviour: b,
            timeout: DEFAULT_JOB_TIMEOUT,
            runs: Arc::default(),
        }
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    pub fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl JobHandler for Scripted {
    fn job_type(&self) -> JobType {
        TEST_TYPE
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn run(&self, ctx: &JobContext, _payload: serde_json::Value) -> Result<(), JobError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        match self.behaviour {
            Behaviour::Succeed => Ok(()),
            Behaviour::Retry => Err(JobError::Retryable(anyhow::anyhow!("scripted retry"))),
            Behaviour::Permanent => Err(JobError::Permanent(anyhow::anyhow!("scripted permanent"))),
            Behaviour::Unavailable(d) => Err(JobError::Unavailable(d)),
            Behaviour::Sleep(d) => {
                tokio::time::sleep(d).await;
                Ok(())
            }
            Behaviour::IgnoreCancel => {
                std::future::pending::<()>().await;
                Ok(())
            }
            Behaviour::UntilCancelled => {
                ctx.cancel.cancelled().await;
                Ok(())
            }
        }
    }
}

pub(crate) struct Harness {
    pub pool: PgPool,
}

impl Harness {
    pub async fn new() -> Self {
        let pool = crate::db::pool::test_pool_sized(6).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        Self { pool }
    }

    pub async fn enqueue(&self, job: NewJob) -> JobId {
        repo::enqueue(&self.pool, &job)
            .await
            .expect("enqueue")
            .expect("not a duplicate")
    }

    pub fn spawn_worker(&self, handler: Scripted, config: JobsConfig) -> Running {
        let registry = Registry::new(vec![Arc::new(handler)]).expect("registry");
        let wake = Arc::new(Notify::new());
        let token = CancellationToken::new();
        let w = Worker {
            id: worker_id("test"),
            pool: self.pool.clone(),
            registry: Arc::new(registry),
            config: Arc::new(config),
            wake: wake.clone(),
        };
        let handle = tokio::spawn(worker::run(w, token.clone()));
        Running {
            token,
            handle,
            wake,
        }
    }

    /// Polls every 50 ms until `done` accepts the row; panics after 10 s with
    /// the last seen record.
    pub async fn wait_until(&self, id: JobId, done: impl Fn(&JobRecord) -> bool) -> JobRecord {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let rec = repo::get(&self.pool, id)
                .await
                .expect("get")
                .expect("job exists");
            if done(&rec) {
                return rec;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting on job {id}: status {:?}, attempts {}, error {:?}",
                rec.status,
                rec.attempts,
                rec.error
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn wait_for_status(&self, id: JobId, status: JobStatus) -> JobRecord {
        self.wait_until(id, |r| r.status == status).await
    }
}

pub(crate) struct Running {
    pub token: CancellationToken,
    pub handle: JoinHandle<()>,
    pub wake: Arc<Notify>,
}

impl Running {
    pub async fn shutdown(self) {
        self.token.cancel();
        tokio::time::timeout(Duration::from_secs(5), self.handle)
            .await
            .expect("worker stops")
            .expect("worker did not panic");
    }
}
