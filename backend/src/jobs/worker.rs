//! One worker: claim, run under a timeout with a heartbeat, record the outcome.

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn, Instrument};

use super::{JobContext, JobError, JobHandler, JobsConfig, Registry};
use crate::domain::jobs::backoff_delay;
use crate::infrastructure::postgres::repositories::jobs::{self as repo, ClaimedJob};

pub(crate) struct Worker {
    pub id: String,
    pub pool: PgPool,
    pub registry: Arc<Registry>,
    pub config: Arc<JobsConfig>,
    pub wake: Arc<Notify>,
}

#[derive(Debug)]
pub(crate) enum Outcome {
    Complete,
    Retry { delay: Duration, error: String },
    Defer(Duration),
    Fail(String),
}

pub(crate) async fn run(worker: Worker, shutdown: CancellationToken) {
    let types = worker.registry.type_names();
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        // Registered before the claim: a NOTIFY that lands between an empty
        // claim and the wait below would otherwise be lost (notify_waiters
        // stores no permit) and cost a whole poll interval.
        let notified = worker.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        match repo::claim(&worker.pool, &worker.id, &types).await {
            Ok(Some(job)) => worker.run_one(job, &shutdown).await,
            Ok(None) => {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = notified => {}
                    _ = tokio::time::sleep(worker.config.poll_interval) => {}
                }
            }
            Err(err) => {
                warn!(worker = %worker.id, error = %err, "Claiming a job failed");
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(worker.config.poll_interval) => {}
                }
            }
        }
    }
}

enum Execution {
    Finished(Result<(), JobError>),
    Abandoned,
}

impl Worker {
    async fn run_one(&self, job: ClaimedJob, shutdown: &CancellationToken) {
        let span = tracing::info_span!(
            "job", job_id = %job.id, job_type = %job.job_type, attempt = job.attempts
        );
        async {
            let Some(handler) = self.registry.get(&job.job_type) else {
                // claim() only returns registered types; a hit here is a bug.
                self.record(
                    &job,
                    Err(JobError::Permanent(anyhow::anyhow!(
                        "no handler registered"
                    ))),
                )
                .await;
                return;
            };
            let started = std::time::Instant::now();
            match self.execute(handler.as_ref(), &job, shutdown).await {
                Execution::Finished(result) => {
                    self.record(&job, result).await;
                    info!(
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Job finished"
                    );
                }
                Execution::Abandoned => match repo::release(&self.pool, job.id, &self.id).await {
                    Ok(_) => warn!("Handler overran the shutdown grace; job released"),
                    Err(err) => warn!(
                        error = %err,
                        "Releasing the job failed; the reaper will reclaim it"
                    ),
                },
            }
        }
        .instrument(span)
        .await
    }

    async fn execute(
        &self,
        handler: &dyn JobHandler,
        job: &ClaimedJob,
        shutdown: &CancellationToken,
    ) -> Execution {
        let cancel = shutdown.child_token();
        let ctx = JobContext {
            job_id: job.id,
            account_id: job.account_id,
            attempt: job.attempts,
            max_attempts: job.max_attempts,
            cancel: cancel.clone(),
        };
        let limit = handler.timeout();
        // A panicking handler must cost one attempt, not the worker task (and
        // with it the whole runtime, which treats any task exit as fatal).
        let guarded = async {
            match std::panic::AssertUnwindSafe(handler.run(&ctx, job.payload.clone()))
                .catch_unwind()
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    error!("Job handler panicked");
                    Err(JobError::Retryable(anyhow::anyhow!("handler panicked")))
                }
            }
        };
        let run = tokio::time::timeout(limit, guarded);
        tokio::pin!(run);
        let period = self.config.heartbeat_interval;
        let mut beat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        // select! evaluates every branch expression even when its guard is
        // false, so the grace sleep needs a real deadline, never an unwrap.
        let far = tokio::time::Instant::now() + Duration::from_secs(365 * 86_400);
        let mut grace_until: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                result = &mut run => {
                    return Execution::Finished(result.unwrap_or_else(|_| {
                        Err(JobError::Retryable(anyhow::anyhow!("timed out after {limit:?}")))
                    }));
                }
                _ = beat.tick() => self.beat(job, &cancel).await,
                _ = shutdown.cancelled(), if grace_until.is_none() => {
                    grace_until = Some(tokio::time::Instant::now() + self.config.shutdown_grace);
                }
                _ = tokio::time::sleep_until(grace_until.unwrap_or(far)), if grace_until.is_some() => {
                    return Execution::Abandoned;
                }
            }
        }
    }

    async fn beat(&self, job: &ClaimedJob, cancel: &CancellationToken) {
        match repo::heartbeat(&self.pool, job.id, &self.id).await {
            Ok(true) => {}
            Ok(false) => {
                warn!("Job was taken from this worker (reaped); cancelling its handler");
                cancel.cancel();
            }
            Err(err) => warn!(error = %err, "Heartbeat failed"),
        }
    }

    async fn record(&self, job: &ClaimedJob, result: Result<(), JobError>) {
        let outcome = outcome_for(job.attempts, job.backoff_seconds, result);
        let recorded = match &outcome {
            Outcome::Complete => repo::complete(&self.pool, job.id, &self.id).await,
            Outcome::Retry { delay, error } => {
                repo::retry(&self.pool, job.id, &self.id, error, *delay).await
            }
            Outcome::Defer(delay) => repo::defer(&self.pool, job.id, &self.id, *delay).await,
            Outcome::Fail(error) => repo::fail(&self.pool, job.id, &self.id, error).await,
        };
        let terminal = matches!(outcome, Outcome::Fail(_))
            || (matches!(outcome, Outcome::Retry { .. }) && job.attempts >= job.max_attempts);
        match recorded {
            Ok(true) if terminal => error!(
                outcome = ?outcome, max_attempts = job.max_attempts, "Job failed permanently"
            ),
            Ok(true) => debug!(outcome = ?outcome, "Job outcome recorded"),
            Ok(false) => warn!("Job was taken from this worker before its outcome was recorded"),
            Err(err) => {
                warn!(error = %err, "Recording the job outcome failed; the reaper will reclaim it")
            }
        }
    }
}

pub(crate) fn outcome_for(
    attempt: i32,
    backoff_seconds: i32,
    result: Result<(), JobError>,
) -> Outcome {
    match result {
        Ok(()) => Outcome::Complete,
        Err(JobError::Retryable(e)) => Outcome::Retry {
            delay: backoff_delay(backoff_seconds, attempt),
            error: format!("{e:#}"),
        },
        Err(JobError::Permanent(e)) => Outcome::Fail(format!("{e:#}")),
        Err(JobError::Unavailable(d)) => Outcome::Defer(d),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::jobs::{JobStatus, NewJob};
    use crate::jobs::testing::{Behaviour, Harness, Scripted, TEST_TYPE};
    use crate::jobs::wakeup;

    #[test]
    fn outcomes_follow_the_error_kind() {
        assert!(matches!(outcome_for(1, 30, Ok(())), Outcome::Complete));
        match outcome_for(
            2,
            30,
            Err(JobError::Retryable(anyhow::anyhow!("send failed"))),
        ) {
            Outcome::Retry { delay, error } => {
                assert_eq!(delay, Duration::from_secs(60));
                assert!(error.contains("send failed"));
            }
            other => panic!("expected retry, got {other:?}"),
        }
        assert!(matches!(
            outcome_for(1, 30, Err(JobError::Permanent(anyhow::anyhow!("bad")))),
            Outcome::Fail(_)
        ));
        assert!(matches!(
            outcome_for(1, 30, Err(JobError::Unavailable(Duration::from_secs(30)))),
            Outcome::Defer(d) if d == Duration::from_secs(30)
        ));
    }

    /// Given an enqueued job and a succeeding handler, When one worker runs,
    /// Then the job completes after one attempt.
    #[tokio::test]
    async fn a_successful_job_completes() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("ok"))).await;
        let handler = Scripted::new(Behaviour::Succeed);
        let stop = h.spawn_worker(handler.clone(), JobsConfig::for_tests());
        let rec = h.wait_for_status(id, JobStatus::Completed).await;
        assert_eq!(rec.attempts, 1);
        assert_eq!(handler.runs(), 1);
        stop.shutdown().await;
    }

    /// Given a job allowed two attempts and a handler that always fails
    /// retryably, When a worker runs, Then it runs twice and the job fails
    /// with the error recorded.
    #[tokio::test]
    async fn a_retryable_failure_is_retried_then_fails() {
        let h = Harness::new().await;
        let id = h
            .enqueue(NewJob::test_job(TEST_TYPE, Some("r")).with_policy(2, 0))
            .await;
        let handler = Scripted::new(Behaviour::Retry);
        let stop = h.spawn_worker(handler.clone(), JobsConfig::for_tests());
        let rec = h.wait_for_status(id, JobStatus::Failed).await;
        assert_eq!(rec.attempts, 2);
        assert_eq!(handler.runs(), 2);
        assert!(rec.error.unwrap().contains("scripted retry"));
        stop.shutdown().await;
    }

    /// Given a handler that panics, When a worker runs the job, Then the
    /// panic counts as a retryable failure (the job is retried, then fails
    /// with the reason recorded) and the worker itself survives.
    #[tokio::test]
    async fn a_panicking_handler_is_retried_and_the_worker_survives() {
        let h = Harness::new().await;
        let id = h
            .enqueue(NewJob::test_job(TEST_TYPE, Some("boom")).with_policy(2, 0))
            .await;
        let handler = Scripted::new(Behaviour::Panic);
        let stop = h.spawn_worker(handler.clone(), JobsConfig::for_tests());
        let rec = h.wait_for_status(id, JobStatus::Failed).await;
        assert_eq!(rec.attempts, 2, "the panic was retried once");
        assert_eq!(handler.runs(), 2);
        assert!(rec.error.unwrap().contains("handler panicked"));
        assert!(!stop.handle.is_finished(), "the worker outlived the panic");
        stop.shutdown().await;
    }

    /// Given a job allowed three attempts and a permanent failure, When a
    /// worker runs, Then it fails at once without a retry.
    #[tokio::test]
    async fn a_permanent_failure_fails_at_once() {
        let h = Harness::new().await;
        let id = h
            .enqueue(NewJob::test_job(TEST_TYPE, Some("p")).with_policy(3, 0))
            .await;
        let handler = Scripted::new(Behaviour::Permanent);
        let stop = h.spawn_worker(handler.clone(), JobsConfig::for_tests());
        let rec = h.wait_for_status(id, JobStatus::Failed).await;
        assert_eq!(rec.attempts, 1);
        assert_eq!(handler.runs(), 1);
        assert!(rec.error.unwrap().contains("scripted permanent"));
        stop.shutdown().await;
    }

    /// Given a handler reporting its dependency unavailable, When a worker
    /// runs, Then the job is pending again, later, with no attempt spent.
    #[tokio::test]
    async fn unavailable_defers_without_spending_an_attempt() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("u"))).await;
        let handler = Scripted::new(Behaviour::Unavailable(Duration::from_secs(60)));
        let stop = h.spawn_worker(handler.clone(), JobsConfig::for_tests());
        // A fresh job is already Pending, so wait for the deferral itself: the
        // handler ran and the row was pushed out by ~60 s.
        let threshold = chrono::Utc::now() + chrono::Duration::seconds(50);
        let rec = h
            .wait_until(id, |r| {
                r.status == JobStatus::Pending && r.available_at > threshold
            })
            .await;
        assert_eq!(handler.runs(), 1);
        assert_eq!(rec.attempts, 0);
        assert!(rec.locked_by.is_none());
        stop.shutdown().await;
    }

    /// Given a handler that outlives its timeout, When a worker runs, Then the
    /// run is cut off and counted as a retryable failure.
    #[tokio::test]
    async fn a_handler_that_overruns_its_timeout_is_retried() {
        let h = Harness::new().await;
        let id = h
            .enqueue(NewJob::test_job(TEST_TYPE, Some("t")).with_policy(1, 0))
            .await;
        let handler = Scripted::new(Behaviour::Sleep(Duration::from_secs(5)))
            .with_timeout(Duration::from_millis(200));
        let stop = h.spawn_worker(handler, JobsConfig::for_tests());
        let rec = h.wait_for_status(id, JobStatus::Failed).await;
        assert!(rec.error.unwrap().contains("timed out"));
        stop.shutdown().await;
    }

    /// Given a job running longer than `stale_after` and a reaper sweeping
    /// concurrently, When the worker heartbeats, Then the job is never reaped.
    #[tokio::test]
    async fn heartbeats_keep_a_long_job_from_being_reaped() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("hb"))).await;
        let handler = Scripted::new(Behaviour::Sleep(Duration::from_secs(5)));
        let mut config = JobsConfig::for_tests();
        config.stale_after = Duration::from_secs(3);
        config.heartbeat_interval = Duration::from_millis(200);
        let stop = h.spawn_worker(handler.clone(), config);

        let pool = h.pool.clone();
        let reaper = tokio::spawn(async move {
            loop {
                let _ = repo::reap_stale(&pool, Duration::from_secs(3)).await;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        let rec = h.wait_for_status(id, JobStatus::Completed).await;
        reaper.abort();
        assert_eq!(rec.attempts, 1);
        assert_eq!(handler.runs(), 1);
        stop.shutdown().await;
    }

    /// Given a handler that ignores cancellation, When shutdown fires, Then
    /// the worker stops after the grace period and the job goes back to
    /// pending with its attempt refunded.
    #[tokio::test]
    async fn shutdown_releases_a_job_whose_handler_ignores_cancellation() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("sd"))).await;
        let handler = Scripted::new(Behaviour::IgnoreCancel);
        let stop = h.spawn_worker(handler.clone(), JobsConfig::for_tests());
        wait_for_runs(&handler, 1).await;
        stop.token.cancel();
        tokio::time::timeout(Duration::from_secs(3), stop.handle)
            .await
            .expect("worker ends within the grace period")
            .expect("worker did not panic");
        let rec = h.wait_for_status(id, JobStatus::Pending).await;
        assert_eq!(rec.attempts, 0);
        assert!(rec.locked_by.is_none());
    }

    /// Given a handler that returns when cancelled, When shutdown fires, Then
    /// it finishes inside the grace period and the job completes.
    #[tokio::test]
    async fn a_cooperative_handler_finishes_within_the_grace_period() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("co"))).await;
        let handler = Scripted::new(Behaviour::UntilCancelled);
        let mut config = JobsConfig::for_tests();
        config.shutdown_grace = Duration::from_secs(5);
        let stop = h.spawn_worker(handler.clone(), config);
        wait_for_runs(&handler, 1).await;
        stop.token.cancel();
        tokio::time::timeout(Duration::from_secs(3), stop.handle)
            .await
            .expect("worker ends")
            .expect("worker did not panic");
        let rec = h.wait_for_status(id, JobStatus::Completed).await;
        assert_eq!(rec.attempts, 1);
    }

    /// Given an idle worker with a 30 s poll interval and a running wake-up
    /// task, When a job is enqueued, Then the NOTIFY wakes it long before the
    /// poll would.
    #[tokio::test]
    async fn an_enqueue_wakes_an_idle_worker_before_the_poll_interval() {
        let h = Harness::new().await;
        let handler = Scripted::new(Behaviour::Succeed);
        let mut config = JobsConfig::for_tests();
        config.poll_interval = Duration::from_secs(30);
        let stop = h.spawn_worker(handler.clone(), config);
        // The listener wakes waiters once it is subscribed; observe that
        // instead of sleeping. The worker registers for wake-ups before it
        // claims, so it cannot miss the NOTIFY whatever it is doing.
        let mut subscribed = Box::pin(stop.wake.notified());
        subscribed.as_mut().enable();
        let listener = tokio::spawn(wakeup::run(
            h.pool.clone(),
            stop.wake.clone(),
            Duration::from_millis(100),
            stop.token.clone(),
        ));
        tokio::time::timeout(Duration::from_secs(10), subscribed)
            .await
            .expect("listener subscribes");
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("w"))).await;
        tokio::time::timeout(
            Duration::from_secs(5),
            h.wait_for_status(id, JobStatus::Completed),
        )
        .await
        .expect("completed within 5 s, not after the 30 s poll");
        stop.shutdown().await;
        listener.await.expect("listener stops");
    }

    async fn wait_for_runs(handler: &Scripted, n: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while handler.runs() < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "handler never started"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
