//! Composition root: builds every long-lived component once, starts the front
//! doors (the Axum API server and every channel adapter: Matrix, and Discord when
//! configured) and the job and events runtimes, and stops them within a fixed
//! bound when shutdown is requested.
//!
//! Shutdown is one `CancellationToken` tree. The caller (`main.rs`) owns the
//! root and cancels it on SIGINT/SIGTERM; every front door and runtime gets a
//! child token, so cancelling the root reaches all of them and none can cancel
//! another.

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::application::AppServices;
use crate::config::{Config, TelegramMode};
use crate::db::Database;
use crate::domain::channels::Channel;
use crate::interface::channels::adapter::ChannelAdapter;
use crate::interface::channels::discord::adapter::DiscordAdapter;
use crate::interface::channels::email::EmailAdapter;
use crate::interface::channels::inbound::Inbound;
use crate::interface::channels::matrix::adapter::MatrixAdapter;
use crate::interface::channels::matrix::handles::BotHandles;
use crate::interface::channels::matrix::modules::build_modules;
use crate::interface::channels::registry::{check_unique, ChannelRegistry};
use crate::interface::channels::telegram::adapter::TelegramAdapter;
use crate::interface::http;
use crate::interface::jobs::Outbound;
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

/// Aborts the task it watches when dropped.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// One channel adapter's supervised `run` task.
pub(crate) struct AdapterTask {
    pub name: &'static str,
    pub handle: JoinHandle<()>,
}

/// The name the supervisor and the shutdown log use for an adapter's task;
/// distinct per channel, because `stop_remaining` skips the ended task by name.
fn front_door_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Matrix => "Matrix sync loop",
        Channel::Discord => "Discord gateway",
        Channel::Telegram => "Telegram updates",
        Channel::Email => "Email poller",
    }
}

/// Resolves when the first adapter task ends (never, with none), with its name.
async fn first_adapter_to_end(
    adapters: &mut [AdapterTask],
) -> (&'static str, Result<(), tokio::task::JoinError>) {
    if adapters.is_empty() {
        return std::future::pending().await;
    }
    let (result, index, rest) =
        futures_util::future::select_all(adapters.iter_mut().map(|a| &mut a.handle)).await;
    drop(rest);
    (adapters[index].name, result)
}

/// Adapts an adapter's `run` task to the supervisor's `JoinHandle<()>`.
///
/// An `Err` is logged here (an adapter that gives up is a front door that
/// stopped), a panic is re-raised so the supervisor reports it as one, and
/// aborting the returned handle aborts `run` too: a bare `JoinHandle` dropped
/// by the wrapper would detach it instead.
fn supervised(name: &'static str, handle: JoinHandle<anyhow::Result<()>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _abort_with_us = AbortOnDrop(handle.abort_handle());
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => error!("{name} failed: {err:#}"),
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(_cancelled) => {}
        }
    })
}

/// Waits for `shutdown` to be cancelled, or for the API server, a channel
/// adapter or a runtime to end on its own — which is always an error, since each holds a child of
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
    adapters: &mut [AdapterTask],
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
        (name, result) = first_adapter_to_end(adapters) => {
            log_early_exit(name, result);
            shutdown.cancel();
            Some(name)
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
    let storage = crate::infrastructure::storage::build_storage(&config.storage)
        .map_err(|e| anyhow::anyhow!("Failed to initialize object storage: {e}"))?;

    let secret_protector: Arc<dyn crate::infrastructure::crypto::SecretProtector> =
        match crate::infrastructure::crypto::AesGcmSecretProtector::from_key_strings(
            config.secrets_encryption_key.as_deref(),
            config.secrets_encryption_keys.as_deref(),
        ) {
            Ok(protector) => Arc::new(protector),
            Err(e) => {
                warn!("Secret encryption not configured ({e}); secret operations will fail closed");
                Arc::new(crate::infrastructure::crypto::FailClosedSecretProtector)
            }
        };

    let app_config = crate::application::config::AppConfigService::load(db.pool.clone())
        .await
        .unwrap_or_else(|e| {
            warn!("Failed to preload app_settings on startup ({e}); using defaults");
            crate::application::config::AppConfigService::new(db.pool.clone())
        });

    let services = AppServices::new(&db.pool)
        .with_app_config(app_config)
        .with_secret_protector(secret_protector)
        .with_link_config(config.web_url.clone(), config.link_signup_allowlist.clone())
        .with_storage(storage)
        .with_embedder(config.embedding_provider())
        .with_embedding_model(&config.embedding_model)
        .with_configured_channels(config.configured_channels());
    // Built once and cloned into every reconnect generation, so `SyncHealth`
    // survives a Matrix reconnect.
    let mut handles = BotHandles::default();
    if !config.matrix_enabled() {
        handles.sync_health.mark_disabled();
    }
    let modules = build_modules(&config).await;
    let matrix = config.matrix_enabled().then(|| {
        Arc::new(MatrixAdapter::new(
            config.clone(),
            services.clone(),
            db.clone(),
            handles.clone(),
        ))
    });
    let discord = config
        .discord
        .clone()
        .map(|discord| Arc::new(DiscordAdapter::new(discord)));
    let telegram = config
        .telegram
        .clone()
        .map(|c| TelegramAdapter::new(c, config.web_url.clone()))
        .transpose()?
        .map(Arc::new);
    let email = config
        .email
        .clone()
        .map(|c| {
            EmailAdapter::new(
                c,
                config.link_signup_allowlist.get(&Channel::Email).cloned(),
            )
        })
        .transpose()?
        .map(Arc::new);
    let configured = config.configured_channels();
    check_unique(&configured)?;
    // The pipeline holds a `Weak` of the registry for out-of-band sends, so the
    // registry is built with `new_cyclic` (A5). Uniqueness was checked just
    // above, which is what makes the infallible constructor sound.
    let channels = Arc::new_cyclic(|weak| {
        let inbound = Arc::new(Inbound::new(
            services.clone(),
            db.clone(),
            modules,
            weak.clone(),
        ));
        let mut adapters: Vec<Arc<dyn ChannelAdapter>> = Vec::new();
        if let Some(matrix) = &matrix {
            matrix.attach_inbound(Arc::clone(&inbound));
            adapters.push(matrix.clone() as Arc<dyn ChannelAdapter>);
        }
        if let Some(discord) = &discord {
            discord.attach_inbound(Arc::clone(&inbound));
            adapters.push(discord.clone() as Arc<dyn ChannelAdapter>);
        }
        if let Some(telegram) = &telegram {
            telegram.attach_inbound(Arc::clone(&inbound));
            adapters.push(telegram.clone() as Arc<dyn ChannelAdapter>);
        }
        if let Some(email) = &email {
            email.attach_inbound(Arc::clone(&inbound));
            adapters.push(email.clone() as Arc<dyn ChannelAdapter>);
        }
        ChannelRegistry::from_unique(adapters)
    });
    handles.outbound = Some(Arc::downgrade(&channels));
    let ocr = crate::infrastructure::inference::ocr::OcrClient::new(config.ocr_base_url.clone());
    let registry = jobs::Registry::new(crate::interface::jobs::handlers(
        &services,
        db.pool.clone(),
        channels.clone() as Arc<dyn Outbound>,
        ocr,
        config.ai.clone(),
    ))?;
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
    let sink: Arc<dyn crate::events::DeliverySink> =
        Arc::new(crate::interface::events::ChannelSink::new(channels.clone()));
    let events_runtime = crate::events::EventsRuntime::new(
        db.pool.clone(),
        services.clone(),
        hub.clone(),
        sink,
        crate::events::EventsConfig::production(),
    );

    let listener = http::bind(&config).await?;
    // Kept here as well as in the API state: the turn tasks outlive a request,
    // so `run` drains them itself once the server has stopped.
    let ai_turns = http::ai_stream::AiTurns::new(http::ai_stream::MAX_TURNS_PER_ACCOUNT);
    let api = http::start_api_server(
        &config,
        db.clone(),
        services.clone(),
        hub.clone(),
        handles.clone(),
        telegram.as_ref().and_then(|t| t.ingress()),
        listener,
        shutdown.child_token(),
        ai_turns.clone(),
    )?;
    let mut api = api;
    // Spawned only once nothing fallible remains before supervision: an earlier
    // `?` would leave it detached, its token never cancelled.
    let mut jobs_task = tokio::spawn(runtime.run(shutdown.child_token()));
    // After the API server is up (R35): SSE subscribers exist before events flow.
    let mut events_task = tokio::spawn(events_runtime.run(shutdown.child_token()));
    // Matrix first, so the shutdown log keeps its order.
    let mut spawned = channels.spawn_all(&shutdown);
    spawned.sort_by_key(|(channel, _)| *channel != Channel::Matrix);
    let mut adapters: Vec<AdapterTask> = spawned
        .into_iter()
        .map(|(channel, handle)| {
            let name = front_door_name(channel);
            AdapterTask {
                name,
                handle: supervised(name, handle),
            }
        })
        .collect();

    let ended = supervise_front_doors(
        &shutdown,
        &mut api,
        &mut adapters,
        &mut jobs_task,
        &mut events_task,
    )
    .await;
    info!("Shutting down Matrix Bot & Web API server...");
    let deadline = Instant::now() + SHUTDOWN_GRACE;

    // The job runtime goes first: its shutdown releases running jobs, which
    // needs the pool open. The events runtime follows it, then the API server
    // (whose SSE streams end on their own token). All share the one deadline.
    let mut tasks = vec![
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
    ];
    tasks.extend(adapters.into_iter().map(|a| OwnedTask {
        name: a.name,
        handle: a.handle,
    }));
    let laggards = stop_remaining(deadline, tasks, ended).await;
    if !laggards.is_empty() {
        warn!(
            "Did not stop within {:?}, aborted: {}",
            SHUTDOWN_GRACE,
            laggards.join(", ")
        );
    }

    // Streaming AI turns run on their own tasks, not the server's. Each one
    // got a child of the server's token, so they are already stopping; wait
    // for them (they store a stopped marker) before the pool closes.
    let tracker = ai_turns.tracker();
    tracker.close();
    match tokio::time::timeout_at(deadline, tracker.wait()).await {
        Ok(()) => info!("AI turns stopped."),
        Err(_) => warn!(
            "{} AI turn(s) still running at the shutdown deadline.",
            tracker.len()
        ),
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
    if config.matrix_enabled() {
        info!(homeserver = %config.homeserver, "Matrix: enabled");
    } else {
        info!("Matrix: disabled (MATRIX_USERNAME/MATRIX_PASSWORD unset)");
    }
    match &config.discord {
        Some(discord) if discord.guild_id.is_some() => {
            info!("Discord: enabled (commands registered to one guild)")
        }
        Some(_) => info!("Discord: enabled (global commands)"),
        None => info!("Discord: disabled (DISCORD_TOKEN unset)"),
    }
    match &config.telegram {
        Some(telegram) => match telegram.mode {
            TelegramMode::Webhook { .. } => info!("Telegram: enabled (webhook mode)"),
            TelegramMode::Polling => info!("Telegram: enabled (polling mode)"),
        },
        None => info!("Telegram: disabled (TELEGRAM_BOT_TOKEN unset)"),
    }
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
        let matrix = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut adapters = matrix_only(matrix);

        let mut jobs = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

        assert!(
            shutdown.is_cancelled(),
            "an unrequested exit must cancel shutdown so the process stops \
             instead of running with a missing front door"
        );
        // The other task must now observe the cancellation and finish too.
        tokio::time::timeout(Duration::from_secs(1), &mut adapters[0].handle)
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
        let matrix = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut adapters = matrix_only(matrix);

        let mut jobs = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });

        supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

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
        let matrix = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let mut adapters = matrix_only(matrix);
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
            supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events),
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
        let matrix = waiter(shutdown.clone());
        let mut adapters = matrix_only(matrix);
        let mut jobs = tokio::spawn(async {}); // ends immediately, unrequested

        supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

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
        let matrix = waiter(shutdown.clone());
        let mut adapters = matrix_only(matrix);
        let mut jobs = waiter(shutdown.clone());
        let mut events = tokio::spawn(async {}); // ends immediately, unrequested

        let ended =
            supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

        assert_eq!(ended, Some("Events runtime"));
        assert!(shutdown.is_cancelled());
    }

    fn matrix_only(handle: JoinHandle<()>) -> Vec<AdapterTask> {
        vec![AdapterTask {
            name: "Matrix sync loop",
            handle,
        }]
    }

    fn tasks_of(
        api: JoinHandle<()>,
        adapters: Vec<AdapterTask>,
        jobs: JoinHandle<()>,
        events: JoinHandle<()>,
    ) -> Vec<OwnedTask> {
        let mut tasks = vec![
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
        ];
        tasks.extend(adapters.into_iter().map(|a| OwnedTask {
            name: a.name,
            handle: a.handle,
        }));
        tasks
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
            let matrix = waiter(shutdown.clone());
            let mut adapters = matrix_only(matrix);
            let mut jobs = early("Job runtime");
            let mut events = early("Events runtime");

            let ended =
                supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events)
                    .await;
            assert_eq!(ended, Some(ended_name));

            let started = Instant::now();
            let laggards = stop_remaining(
                started + Duration::from_secs(15),
                tasks_of(api, adapters, jobs, events),
                ended,
            )
            .await;
            assert!(laggards.is_empty(), "{ended_name}: {laggards:?}");
        }
    }
    #[tokio::test(start_paused = true)]
    async fn an_adapter_other_than_matrix_ending_unrequested_stops_the_process() {
        let shutdown = CancellationToken::new();
        let waiter = |s: CancellationToken| tokio::spawn(async move { s.cancelled().await });
        let mut api = waiter(shutdown.clone());
        let mut jobs = waiter(shutdown.clone());
        let mut events = waiter(shutdown.clone());
        let mut adapters = vec![
            AdapterTask {
                name: front_door_name(Channel::Matrix),
                handle: waiter(shutdown.clone()),
            },
            AdapterTask {
                name: front_door_name(Channel::Discord),
                handle: tokio::spawn(async {}), // ends immediately, unrequested
            },
        ];

        let ended =
            supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

        assert_eq!(ended, Some("Discord gateway"));
        assert!(shutdown.is_cancelled());
        let started = Instant::now();
        let laggards = stop_remaining(
            started + Duration::from_secs(15),
            tasks_of(api, adapters, jobs, events),
            ended,
        )
        .await;
        assert!(laggards.is_empty(), "{laggards:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_telegram_adapter_task_ending_unrequested_stops_the_process() {
        let shutdown = CancellationToken::new();
        let waiter = |s: CancellationToken| tokio::spawn(async move { s.cancelled().await });
        let mut api = waiter(shutdown.clone());
        let mut jobs = waiter(shutdown.clone());
        let mut events = waiter(shutdown.clone());
        let mut adapters = vec![
            AdapterTask {
                name: front_door_name(Channel::Matrix),
                handle: waiter(shutdown.clone()),
            },
            AdapterTask {
                name: front_door_name(Channel::Telegram),
                handle: tokio::spawn(async {}), // ends immediately, unrequested
            },
        ];

        let ended =
            supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

        assert_eq!(ended, Some("Telegram updates"));
        assert!(shutdown.is_cancelled());
        let started = Instant::now();
        let laggards = stop_remaining(
            started + Duration::from_secs(15),
            tasks_of(api, adapters, jobs, events),
            ended,
        )
        .await;
        assert!(laggards.is_empty(), "{laggards:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_email_adapter_task_ending_unrequested_stops_the_process() {
        let shutdown = CancellationToken::new();
        let waiter = |s: CancellationToken| tokio::spawn(async move { s.cancelled().await });
        let mut api = waiter(shutdown.clone());
        let mut jobs = waiter(shutdown.clone());
        let mut events = waiter(shutdown.clone());
        let mut adapters = vec![
            AdapterTask {
                name: front_door_name(Channel::Matrix),
                handle: waiter(shutdown.clone()),
            },
            AdapterTask {
                name: front_door_name(Channel::Email),
                handle: tokio::spawn(async {}), // ends immediately, unrequested
            },
        ];

        let ended =
            supervise_front_doors(&shutdown, &mut api, &mut adapters, &mut jobs, &mut events).await;

        assert_eq!(ended, Some("Email poller"));
        assert!(shutdown.is_cancelled());
        let started = Instant::now();
        let laggards = stop_remaining(
            started + Duration::from_secs(15),
            tasks_of(api, adapters, jobs, events),
            ended,
        )
        .await;
        assert!(laggards.is_empty(), "{laggards:?}");
    }

    #[test]
    fn front_door_names_are_distinct_per_channel() {
        let names: Vec<_> = [
            Channel::Matrix,
            Channel::Discord,
            Channel::Telegram,
            Channel::Email,
        ]
        .into_iter()
        .map(front_door_name)
        .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        assert_eq!(names[0], "Matrix sync loop");
    }

    #[tokio::test(start_paused = true)]
    async fn aborting_the_supervised_handle_aborts_the_adapter_run() {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        let run = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<anyhow::Result<()>>().await
        });
        let wrapper = supervised("adapter", run);
        tokio::task::yield_now().await;

        wrapper.abort();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        assert!(dropped.load(Ordering::SeqCst), "run must not be detached");
    }

    #[tokio::test]
    async fn an_adapter_that_returns_an_error_ends_its_supervised_task() {
        let wrapper = supervised(
            "adapter",
            tokio::spawn(async { anyhow::bail!("login refused") }),
        );
        assert!(wrapper.await.is_ok(), "the error is logged, not re-raised");
    }

    #[tokio::test]
    async fn an_adapter_panic_surfaces_as_a_panic_of_the_supervised_task() {
        let wrapper = supervised(
            "adapter",
            tokio::spawn(async {
                let healthy = std::hint::black_box(false);
                assert!(healthy, "adapter bug");
                Ok(())
            }),
        );
        assert!(wrapper.await.unwrap_err().is_panic());
    }
}
