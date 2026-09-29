//! Leader-elected scheduler: at most one instance at a time (holder of a
//! session advisory lock) fires the recurring ticks that enqueue jobs.

use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::schedule::Tick;
use super::JobsConfig;
use crate::application::AppServices;
use crate::infrastructure::postgres::advisory::LeaderLock;

pub(crate) async fn run(
    pool: PgPool,
    services: AppServices,
    ticks: Vec<Tick>,
    config: Arc<JobsConfig>,
    shutdown: CancellationToken,
) {
    if ticks.is_empty() {
        shutdown.cancelled().await;
        return;
    }
    loop {
        match LeaderLock::try_acquire(&pool, config.scheduler_lock_key).await {
            Ok(Some(mut lock)) => {
                info!("This instance is now the job scheduler leader");
                lead(&mut lock, &services, &ticks, &shutdown).await;
                lock.release().await;
                if shutdown.is_cancelled() {
                    return;
                }
                warn!("Lost scheduler leadership; retrying");
            }
            Ok(None) => debug!("Another instance leads the scheduler"),
            Err(err) => warn!(error = %err, "Could not try the scheduler lock"),
        }
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(config.leader_retry) => {}
        }
    }
}

/// Fires each tick at once, then every `tick.every`, while the lock's
/// connection is alive. Returns on shutdown or lost leadership.
async fn lead(
    lock: &mut LeaderLock,
    services: &AppServices,
    ticks: &[Tick],
    shutdown: &CancellationToken,
) {
    let start = tokio::time::Instant::now();
    let mut next: Vec<tokio::time::Instant> = vec![start; ticks.len()];
    loop {
        let due = next.iter().copied().min().unwrap_or(start);
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep_until(due) => {}
        }
        if !lock.is_alive().await {
            return;
        }
        let now = tokio::time::Instant::now();
        for (tick, at) in ticks.iter().zip(next.iter_mut()) {
            if *at > now {
                continue;
            }
            *at = now + tick.every;
            match (tick.fire)(services, Utc::now()).await {
                Ok(0) => {}
                Ok(n) => debug!(tick = tick.name, enqueued = n, "Scheduler tick"),
                Err(err) => warn!(tick = tick.name, error = %err, "Scheduler tick failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use chrono::DateTime;

    use super::*;
    use crate::application::AppError;
    use crate::jobs::schedule::TickFuture;

    static A: AtomicUsize = AtomicUsize::new(0);
    static B: AtomicUsize = AtomicUsize::new(0);
    static C: AtomicUsize = AtomicUsize::new(0);
    static E: AtomicUsize = AtomicUsize::new(0);

    fn count_a(_: &AppServices, _: DateTime<Utc>) -> TickFuture<'_> {
        A.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(0) })
    }
    fn count_b(_: &AppServices, _: DateTime<Utc>) -> TickFuture<'_> {
        B.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(0) })
    }
    fn count_c(_: &AppServices, _: DateTime<Utc>) -> TickFuture<'_> {
        C.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(0) })
    }
    fn failing(_: &AppServices, _: DateTime<Utc>) -> TickFuture<'_> {
        E.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AppError::Validation("boom".into())) })
    }

    fn tick(
        every_ms: u64,
        fire: for<'a> fn(&'a AppServices, DateTime<Utc>) -> TickFuture<'a>,
    ) -> Tick {
        Tick {
            name: "t",
            every: Duration::from_millis(every_ms),
            fire,
        }
    }

    async fn until(what: &str, cond: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(std::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Given two schedulers with the same lock key, When both run, Then
    /// exactly one of them fires its ticks.
    #[tokio::test]
    async fn only_the_leader_fires_ticks() {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let config = Arc::new(JobsConfig::for_tests());
        let token = CancellationToken::new();
        let a = tokio::spawn(run(
            pool.clone(),
            services.clone(),
            vec![tick(100, count_a)],
            config.clone(),
            token.clone(),
        ));
        let b = tokio::spawn(run(
            pool.clone(),
            services.clone(),
            vec![tick(100, count_b)],
            config.clone(),
            token.clone(),
        ));
        until("a leader fires", || {
            A.load(Ordering::SeqCst) + B.load(Ordering::SeqCst) > 0
        })
        .await;
        // The follower retries every 100 ms; give it several chances.
        tokio::time::sleep(Duration::from_millis(600)).await;
        token.cancel();
        a.await.unwrap();
        b.await.unwrap();
        let (a, b) = (A.load(Ordering::SeqCst), B.load(Ordering::SeqCst));
        assert!(
            (a > 0) ^ (b > 0),
            "exactly one leader expected, got a={a} b={b}"
        );
    }

    /// Given a leader, When it gains leadership, Then every tick fires at
    /// once and then per `every`.
    #[tokio::test]
    async fn ticks_fire_immediately_then_on_their_cadence() {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let token = CancellationToken::new();
        let task = tokio::spawn(run(
            pool.clone(),
            services,
            vec![tick(300, count_c)],
            Arc::new(JobsConfig::for_tests()),
            token.clone(),
        ));
        until("immediate first fire", || C.load(Ordering::SeqCst) >= 1).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        token.cancel();
        task.await.unwrap();
        let n = C.load(Ordering::SeqCst);
        assert!((2..=5).contains(&n), "expected 2..=5 fires, got {n}");
    }

    /// Given a tick that errors, When the scheduler runs, Then it keeps ticking.
    #[tokio::test]
    async fn a_failing_tick_does_not_stop_the_scheduler() {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let token = CancellationToken::new();
        let task = tokio::spawn(run(
            pool.clone(),
            services,
            vec![tick(100, failing)],
            Arc::new(JobsConfig::for_tests()),
            token.clone(),
        ));
        until("three failing fires", || E.load(Ordering::SeqCst) >= 3).await;
        token.cancel();
        task.await.unwrap();
    }
}
