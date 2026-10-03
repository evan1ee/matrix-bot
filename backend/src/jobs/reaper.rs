//! Housekeeping every instance runs (both statements are safe to run
//! concurrently): requeue jobs whose worker stopped heart-beating, and
//! delete finished jobs after `purge_after`.

use std::sync::Arc;

use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::JobsConfig;
use crate::infrastructure::postgres::repositories::jobs as repo;

pub(crate) async fn run(pool: PgPool, config: Arc<JobsConfig>, shutdown: CancellationToken) {
    let mut reap = tokio::time::interval(config.reap_interval);
    let mut purge = tokio::time::interval(config.purge_interval);
    reap.set_missed_tick_behavior(MissedTickBehavior::Delay);
    purge.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        // `biased` so a cancelled token always wins over a due tick; each
        // query is raced against shutdown so a slow statement cannot delay
        // the runtime's exit.
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            _ = reap.tick() => {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return,
                    result = repo::reap_stale(&pool, config.stale_after) => match result {
                        Ok(0) => {}
                        Ok(n) => warn!(reaped = n, "Requeued jobs whose worker stopped heart-beating"),
                        Err(err) => warn!(error = %err, "Reaping stale jobs failed"),
                    },
                }
            }
            _ = purge.tick() => {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return,
                    result = repo::purge_finished(&pool, config.purge_after) => match result {
                        Ok(n) => debug!(purged = n, "Purged finished jobs"),
                        Err(err) => warn!(error = %err, "Purging finished jobs failed"),
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::jobs::{JobStatus, NewJob};
    use crate::infrastructure::postgres::repositories::jobs as repo;
    use crate::jobs::testing::{Harness, TEST_TYPE};
    use crate::jobs::JobsConfig;

    /// Given a running job whose heartbeat is 10 s old, When the reaper loop
    /// runs (stale_after 1 s), Then the job is pending again.
    #[tokio::test]
    async fn the_loop_requeues_a_job_with_a_stale_heartbeat() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("stale"))).await;
        repo::claim(&h.pool, "dead-worker", &[TEST_TYPE.as_str()])
            .await
            .unwrap()
            .unwrap();
        repo::backdate(&h.pool, id, 10.0).await.unwrap();
        let token = CancellationToken::new();
        let task = tokio::spawn(run(
            h.pool.clone(),
            Arc::new(JobsConfig::for_tests()),
            token.clone(),
        ));
        h.wait_for_status(id, JobStatus::Pending).await;
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }

    /// Given a job completed 8 days ago, When the loop starts, Then its first
    /// purge tick deletes it.
    #[tokio::test]
    async fn the_loop_purges_old_finished_jobs_on_start() {
        let h = Harness::new().await;
        let id = h.enqueue(NewJob::test_job(TEST_TYPE, Some("old"))).await;
        repo::claim(&h.pool, "w", &[TEST_TYPE.as_str()])
            .await
            .unwrap()
            .unwrap();
        assert!(repo::complete(&h.pool, id, "w").await.unwrap());
        repo::backdate(&h.pool, id, 8.0 * 86_400.0).await.unwrap();
        let token = CancellationToken::new();
        let task = tokio::spawn(run(
            h.pool.clone(),
            Arc::new(JobsConfig::for_tests()),
            token.clone(),
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while repo::get(&h.pool, id).await.unwrap().is_some() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "job {id} was never purged"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }

    /// Given a cancelled token, When run starts, Then it returns immediately.
    #[tokio::test]
    async fn the_loop_stops_on_cancel() {
        let h = Harness::new().await;
        let token = CancellationToken::new();
        token.cancel();
        tokio::time::timeout(
            Duration::from_secs(1),
            run(h.pool.clone(), Arc::new(JobsConfig::for_tests()), token),
        )
        .await
        .expect("run returns on a cancelled token");
    }
}
