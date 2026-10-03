//! Turns `NOTIFY jobs` into a wake-up for idle workers.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::infrastructure::postgres::job_notifications::JobNotifications;

/// Forwards `NOTIFY jobs` to `wake.notify_waiters()`. On any listener error
/// it logs, waits `retry_after` and reconnects; workers keep polling
/// meanwhile, so a broken listener costs latency, never work.
pub(crate) async fn run(
    pool: PgPool,
    wake: Arc<Notify>,
    retry_after: Duration,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            result = listen(&pool, &wake) => {
                let Err(err) = result;
                warn!(error = %err, "Job LISTEN failed; workers fall back to polling");
            }
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(retry_after) => {}
        }
    }
}

/// Forwards notifications until the listener fails; never returns `Ok`.
async fn listen(pool: &PgPool, wake: &Notify) -> sqlx::Result<std::convert::Infallible> {
    let mut listener = JobNotifications::connect(pool).await?;
    // A NOTIFY sent while the listener was down is gone, so every (re)connect
    // wakes the workers once to re-check the queue. It also tells a test that
    // the subscription is live.
    wake.notify_waiters();
    loop {
        listener.next().await?;
        wake.notify_waiters();
    }
}
