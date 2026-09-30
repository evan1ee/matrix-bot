//! Composition root: builds every long-lived component once, starts the front
//! doors (the Axum API server and the Matrix sync loop) and the job runtime, and stops them within
//! a fixed bound when shutdown is requested.
//!
//! Shutdown is one `CancellationToken` tree. The caller (`main.rs`) owns the
//! root and cancels it on SIGINT/SIGTERM; the API server, the Matrix loop and the job runtime
//! each get a child token, so cancelling the root reaches all three and none
//! can cancel another.

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::application::AppServices;
use crate::config::Config;
use crate::db::Database;
use crate::interface::channels::matrix::delivery::MatrixDelivery;
use crate::interface::channels::matrix::handles::BotHandles;
use crate::interface::channels::matrix::sync_loop::run_bot_loop;
use crate::interface::http;
use crate::interface::jobs::Delivery;
use crate::jobs;

/// Signal to return, including closing the database pool (spec §5.3).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

/// A long-lived task `run` owns, named for the shutdown log.
///
/// `pub(crate)` rather than `pub`: only `app::run` and its own tests
/// construct these, and nothing outside the crate needs to observe or build
/// one. Note for callers using `stop_by`: aborting the REST API server's own
/// task does not necessarily end its per-connection tasks — axum's graceful
/// shutdown hands those off to the runtime, and they outlive the server
/// task's abort until the runtime itself calls `shutdown_background()`
/// (`main.rs`), which is why closing the database pool afterward still waits
/// on whatever connections those stragglers hold.
pub(crate) struct OwnedTask {
    pub name: &'static str,
    pub handle: JoinHandle<()>,
}

/// Waits for `shutdown` to be cancelled, or for the API server, the Matrix loop
/// or the job runtime to end on its own — which is always an error, since each holds a child of
/// `shutdown` and exit only when it fires. A front door that stops on its
/// own (task panic, or a bug that lets its loop return) would otherwise leave
/// the process running with no API or no Matrix connection; instead this
/// cancels `shutdown` so the rest of `run` tears down and the process exits,
/// relying on the surrounding orchestrator (systemd, Docker, k8s) to restart
///
/// Returns the name of the task that ended on its own, if any: its `JoinHandle`
/// has been consumed, so it must not be awaited again by `stop_by`.
async fn supervise_front_doors(
    shutdown: &CancellationToken,
    api: &mut JoinHandle<()>,
    matrix: &mut JoinHandle<()>,
    jobs: &mut JoinHandle<()>,
    events: &mut JoinHandle<()>,
) -> Option<&'static str> {
    tokio::select! {
        _ = shutdown.cancelled() => None,
        result = &mut *api => {
            log_early_exit("REST API server", result);
            shutdown.cancel();
            Some("REST API server")
        }
        result = &mut *matrix => {
            log_early_exit("Matrix sync loop", result);
            shutdown.cancel();
            Some("Matrix sync loop")
        }
        result = &mut *jobs => {
            log_early_exit("Job runtime", result);
            shutdown.cancel();
            Some("Job runtime")
        }
        result = &mut *events => {
            log_early_exit("Events runtime", result);
            shutdown.cancel();
            Some("Events runtime")
        }
    }
}

/// Stops every task except `ended` (already joined by the supervisor) by
/// `deadline`. Awaiting a completed `JoinHandle` again panics.
pub(crate) async fn stop_remaining(
    deadline: Instant,
    tasks: Vec<OwnedTask>,
    ended: Option<&str>,
) -> Vec<&'static str> {
    let tasks = tasks
        .into_iter()
        .filter(|t| Some(t.name) != ended)
        .collect();
    stop_by(deadline, tasks).await
}

fn log_early_exit(name: &str, result: Result<(), tokio::task::JoinError>) {
    match result {
        Ok(()) => error!("{name} task ended before shutdown was requested; stopping the process."),
        Err(err) => error!("{name} task panicked: {err:?}; stopping the process."),
    }
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
    let services = AppServices::new(&db.pool);
    // Built once and cloned into every reconnect generation, so `SyncHealth`
    // and the pending merges survive a Matrix reconnect.
    let handles = BotHandles::default();
    let delivery: Arc<dyn Delivery> = Arc::new(MatrixDelivery::new(handles.matrix.clone()));
    let registry = jobs::Registry::new(crate::interface::jobs::handlers(&services, delivery))?;
    let runtime = jobs::JobRuntime::new(
        db.pool.clone(),
        services.clone(),
        registry,
        jobs::schedule::default_ticks(),
        jobs::JobsConfig::production(config.job_workers, config.job_worker_host.clone()),
    );

    let hub = crate::events::Hub::new(
        services.outbox.clone(),
        crate::events::EventsConfig::production().subscriber_capacity,
    );
    let sink: Arc<dyn crate::events::DeliverySink> = Arc::new(
        crate::interface::events::MatrixRoomSink::new(handles.matrix.clone(), services.clone()),
    );
    let events_runtime = crate::events::EventsRuntime::new(
        db.pool.clone(),
        services.clone(),
        hub.clone(),
        sink,
        crate::events::EventsConfig::production(),
    );

    let listener = http::bind(&config).await?;
    let api = http::start_api_server(
        &config,
        db.clone(),
        services.clone(),
        hub.clone(),
        handles.clone(),
        listener,
        shutdown.child_token(),
    )?;
    let mut api = api;
    // Spawned only once nothing fallible remains before supervision: an earlier
    // `?` would leave it detached, its token never cancelled.
    let mut jobs_task = tokio::spawn(runtime.run(shutdown.child_token()));
    // After the API server is up (R35): SSE subscribers exist before events flow.
    let mut events_task = tokio::spawn(events_runtime.run(shutdown.child_token()));
    let mut matrix = tokio::spawn(run_bot_loop(
        config,
        db.clone(),
        services,
        handles,
        shutdown.child_token(),
    ));

    let ended = supervise_front_doors(
        &shutdown,
        &mut api,
        &mut matrix,
        &mut jobs_task,
        &mut events_task,
    )
    .await;
    info!("Shutting down Matrix Bot & Web API server...");
    let deadline = Instant::now() + SHUTDOWN_GRACE;

    // The job runtime goes first: its shutdown releases running jobs, which
    // needs the pool open. The events runtime follows it, then the API server
    // (whose SSE streams end on their own token). All share the one deadline.
    let tasks = vec![
        OwnedTask {
            name: "Job runtime",
            handle: jobs_task,
        },
        OwnedTask {
            name: "Events runtime",
            handle: events_task,
        },
        OwnedTask {
            name: "REST API server",
            handle: api,
        },
        OwnedTask {
            name: "Matrix sync loop",
            handle: matrix,
        },
    ];
    let laggards = stop_remaining(deadline, tasks, ended).await;
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
/// `PublishedClient`) run; it is not awaited again, because a task stuck
/// outside an await point would hold the process past the bound.
pub(crate) async fn stop_by(deadline: Instant, tasks: Vec<OwnedTask>) -> Vec<&'static str> {
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

    #[tokio::test(start_paused = true)]
    async fn a_front_door_ending_early_cancels_shutdown() {
        let shutdown = CancellationToken::new();
        let mut events = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut api = tokio::spawn(async {}); // ends immediately, unrequested
        let mut matrix = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        let mut jobs = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        supervise_front_doors(&shutdown, &mut api, &mut matrix, &mut jobs, &mut events).await;

        assert!(
            shutdown.is_cancelled(),
            "an unrequested exit must cancel shutdown so the process stops \
             instead of running with a missing front door"
        );
        // The other task must now observe the cancellation and finish too.
        tokio::time::timeout(Duration::from_secs(1), &mut matrix)
            .await
            .expect("matrix task must finish after shutdown was cancelled")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_front_door_also_cancels_shutdown() {
        let shutdown = CancellationToken::new();
        let mut events = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut api = tokio::spawn(async { panic!("boom") });
        let mut matrix = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        let mut jobs = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        supervise_front_doors(&shutdown, &mut api, &mut matrix, &mut jobs, &mut events).await;

        assert!(shutdown.is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn a_requested_shutdown_is_not_treated_as_an_early_exit() {
        let shutdown = CancellationToken::new();
        let mut events = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut api = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut matrix = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        shutdown.cancel();
        let mut jobs = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        // Must return promptly via the shutdown branch, not hang waiting on
        // the tasks (which also complete once cancelled, but that's not the
        // branch under test).
        tokio::time::timeout(
            Duration::from_secs(1),
            supervise_front_doors(&shutdown, &mut api, &mut matrix, &mut jobs, &mut events),
        )
        .await
        .expect("must return promptly once shutdown was already requested");
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_runtime_ending_early_cancels_shutdown() {
        let shutdown = CancellationToken::new();
        let mut events = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let waiter = |s: CancellationToken| tokio::spawn(async move { s.cancelled().await });
        let mut api = waiter(shutdown.clone());
        let mut matrix = waiter(shutdown.clone());
        let mut jobs = tokio::spawn(async {}); // ends immediately, unrequested

        supervise_front_doors(&shutdown, &mut api, &mut matrix, &mut jobs, &mut events).await;

        assert!(
            shutdown.is_cancelled(),
            "a runtime that stops on its own must stop the process, not leave \
             it running with no background jobs"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_events_runtime_ending_early_cancels_shutdown() {
        let shutdown = CancellationToken::new();
        let waiter = |s: CancellationToken| tokio::spawn(async move { s.cancelled().await });
        let mut api = waiter(shutdown.clone());
        let mut matrix = waiter(shutdown.clone());
        let mut jobs = waiter(shutdown.clone());
        let mut events = tokio::spawn(async {}); // ends immediately, unrequested

        let ended =
            supervise_front_doors(&shutdown, &mut api, &mut matrix, &mut jobs, &mut events).await;

        assert_eq!(ended, Some("Events runtime"));
        assert!(shutdown.is_cancelled());
    }

    fn tasks_of(
        api: JoinHandle<()>,
        matrix: JoinHandle<()>,
        jobs: JoinHandle<()>,
        events: JoinHandle<()>,
    ) -> Vec<OwnedTask> {
        vec![
            OwnedTask {
                name: "Job runtime",
                handle: jobs,
            },
            OwnedTask {
                name: "Events runtime",
                handle: events,
            },
            OwnedTask {
                name: "REST API server",
                handle: api,
            },
            OwnedTask {
                name: "Matrix sync loop",
                handle: matrix,
            },
        ]
    }

    /// Given a task that ended on its own, When shutdown runs, Then the
    /// consumed handle is not awaited again (that panics) and nothing lags.
    #[tokio::test(start_paused = true)]
    async fn stopping_after_an_early_exit_does_not_rejoin_the_ended_task() {
        for ended_name in ["Job runtime", "REST API server", "Events runtime"] {
            let shutdown = CancellationToken::new();
            let waiter = |s: CancellationToken| tokio::spawn(async move { s.cancelled().await });
            let early = |name: &str| {
                if name == ended_name {
                    tokio::spawn(async {})
                } else {
                    waiter(shutdown.clone())
                }
            };
            let mut api = early("REST API server");
            let mut matrix = waiter(shutdown.clone());
            let mut jobs = early("Job runtime");
            let mut events = early("Events runtime");

            let ended =
                supervise_front_doors(&shutdown, &mut api, &mut matrix, &mut jobs, &mut events)
                    .await;
            assert_eq!(ended, Some(ended_name));

            let started = Instant::now();
            let laggards = stop_remaining(
                started + Duration::from_secs(15),
                tasks_of(api, matrix, jobs, events),
                ended,
            )
            .await;
            assert!(laggards.is_empty(), "{ended_name}: {laggards:?}");
        }
    }
}
