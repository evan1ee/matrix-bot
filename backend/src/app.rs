//! Composition root: builds every long-lived component once, starts both front
//! doors (the Axum API server and the Matrix sync loop), and stops them within
//! a fixed bound when shutdown is requested.
//!
//! Shutdown is one `CancellationToken` tree. The caller (`main.rs`) owns the
//! root and cancels it on SIGINT/SIGTERM; the API server and the Matrix loop
//! each get a child token, so cancelling the root reaches both and neither can
//! cancel the other.

use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::channels::matrix::handles::BotHandles;
use crate::interface::channels::matrix::sync_loop::run_bot_loop;
use crate::interface::http;

/// Signal to return, including closing the database pool (spec §5.3).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

/// A long-lived task `run` owns, named for the shutdown log.
pub struct OwnedTask {
    pub name: &'static str,
    pub handle: JoinHandle<()>,
}

/// Builds every long-lived component, starts the API server and the Matrix
/// loop, waits for `shutdown` to be cancelled, then stops everything within
/// SHUTDOWN_GRACE and closes the pool.
pub async fn run(config: Config, shutdown: CancellationToken) -> anyhow::Result<()> {
    log_registration_mode(&config);

    // Boot is interruptible too: the signal handler is already installed, so a
    // Ctrl-C during a slow connect or migration must not wait for it to finish.
    // sqlx runs each migration in its own transaction, so dropping one
    // mid-way rolls it back.
    let db = tokio::select! {
        _ = shutdown.cancelled() => {
            info!("Shutdown requested during boot; nothing started yet.");
            return Ok(());
        }
        db = Database::connect(&config.database_url, config.db_max_connections) => db?,
    };
    let broadcaster = EventBroadcaster::default();
    // Built once and cloned into every reconnect generation, so `SyncHealth`
    // and the pending merges survive a Matrix reconnect.
    let handles = BotHandles::default();

    let listener = http::bind(&config).await?;
    let api = http::start_api_server(
        &config,
        db.clone(),
        broadcaster.clone(),
        handles.clone(),
        listener,
        shutdown.child_token(),
    )?;
    let matrix = tokio::spawn(run_bot_loop(
        config,
        db.clone(),
        broadcaster,
        handles,
        shutdown.child_token(),
    ));

    shutdown.cancelled().await;
    info!("Shutting down Matrix Bot & Web API server...");
    let deadline = Instant::now() + SHUTDOWN_GRACE;

    let tasks = vec![
        OwnedTask {
            name: "REST API server",
            handle: api,
        },
        OwnedTask {
            name: "Matrix sync loop",
            handle: matrix,
        },
    ];
    let laggards = stop_by(deadline, tasks).await;
    if !laggards.is_empty() {
        warn!(
            "Did not stop within {:?}, aborted: {}",
            SHUTDOWN_GRACE,
            laggards.join(", ")
        );
    }

    // Whatever is left of the same deadline: `close` waits for every checked
    // out connection to come back, which an aborted laggard may never do.
    match tokio::time::timeout_at(deadline, db.pool.close()).await {
        Ok(()) => info!("Database pool closed."),
        Err(_) => warn!("Database pool did not close before the shutdown deadline."),
    }

    Ok(())
}

/// Waits for every task until `deadline`, then aborts whichever is still
/// running and returns their names.
///
/// Awaiting the handles one after another against a shared deadline is the
/// same bound as joining them concurrently: each wait ends by `deadline`. A
/// laggard is aborted rather than detached so its drop guards (e.g.
/// `BackgroundWorkers`) run; it is not awaited again, because a task stuck
/// outside an await point would hold the process past the bound.
pub async fn stop_by(deadline: Instant, tasks: Vec<OwnedTask>) -> Vec<&'static str> {
    let mut laggards = Vec::new();
    for OwnedTask { name, mut handle } in tasks {
        match tokio::time::timeout_at(deadline, &mut handle).await {
            Ok(Ok(())) => info!("{name} stopped."),
            Ok(Err(err)) => error!("{name} ended abnormally: {err:?}"),
            Err(_) => {
                handle.abort();
                laggards.push(name);
            }
        }
    }
    laggards
}

fn log_registration_mode(config: &Config) {
    if config.registration_secret.is_some() {
        info!("Registration mode: INVITE-ONLY (invite secret configured)");
    } else if config.registration_open {
        info!("Registration mode: OPEN (open registration enabled)");
    } else {
        info!("Registration mode: CLOSED (release default: open internet scanners cannot register accounts)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Sets its flag when dropped: proves a laggard was aborted, not detached.
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn returns_as_soon_as_every_task_finishes() {
        let started = Instant::now();
        let tasks = vec![
            OwnedTask {
                name: "api",
                handle: tokio::spawn(async {}),
            },
            OwnedTask {
                name: "matrix",
                handle: tokio::spawn(tokio::time::sleep(Duration::from_secs(2))),
            },
        ];

        let laggards = stop_by(started + Duration::from_secs(15), tasks).await;

        assert!(laggards.is_empty(), "no laggards expected: {laggards:?}");
        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn names_and_aborts_a_task_that_ignores_cancellation() {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        let started = Instant::now();
        let tasks = vec![
            OwnedTask {
                name: "api",
                handle: tokio::spawn(async {}),
            },
            OwnedTask {
                name: "matrix",
                handle: tokio::spawn(async move {
                    let _guard = guard;
                    std::future::pending::<()>().await;
                }),
            },
        ];

        let laggards = stop_by(started + Duration::from_secs(15), tasks).await;

        assert_eq!(laggards, vec!["matrix"]);
        assert_eq!(started.elapsed(), Duration::from_secs(15));
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            dropped.load(Ordering::SeqCst),
            "the laggard must be aborted"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicked_task_is_finished_not_a_laggard() {
        let started = Instant::now();
        let tasks = vec![OwnedTask {
            name: "api",
            handle: tokio::spawn(async { panic!("boom") }),
        }];

        let laggards = stop_by(started + Duration::from_secs(15), tasks).await;

        assert!(laggards.is_empty());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
