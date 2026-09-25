//! Use cases. A service method is the unit of transaction: it validates a
//! command through the domain, opens the transaction, calls repositories,
//! publishes the resulting event and commits. HTTP, Matrix, MCP and LLM
//! tools all call the same method. Never imports a transport
//! (`tests/architecture.rs`).

pub mod rooms;
pub mod settings;

use sqlx::PgPool;

use self::rooms::RoomService;
use self::settings::SettingsService;
use crate::events::EventBroadcaster;

/// Why a use case failed, by boundary rather than by cause.
///
/// Adapters decide what each variant means to their caller:
/// `interface/http/app_error.rs` maps it to `ApiError`,
/// `interface/chat::error_reply` to a chat reply. The message of the first
/// four is safe to show a user; the payload of the last two is diagnostic
/// and must never reach one.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The input broke a domain rule. A feature's own domain error converts
    /// into this with a `From` impl in `application/<feature>`.
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Conflict(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// A dependency outside the database failed. `service` names it (e.g.
    /// "ocr") so an adapter can match on it without comparing strings.
    #[error("{service} failed")]
    External {
        service: &'static str,
        #[source]
        source: anyhow::Error,
    },
}

/// Every application service, built once by the composition root (`app.rs`
/// and `main.rs`'s stdio MCP mode) and handed to each front door —
/// `ApiState` and the Matrix `ModuleContext` — so no adapter constructs a
/// service of its own.
///
/// Each service is cheap to clone (a `PgPool` is a handle), so this is too.
/// A feature joins by adding its service as a field here and building it in
/// [`AppServices::with_events`].
#[derive(Clone)]
pub struct AppServices {
    pub rooms: RoomService,
    pub settings: SettingsService,
}

impl AppServices {
    /// Test-only: builds a private, per-call `EventBroadcaster` that nothing
    /// else subscribes to. Every real composition root uses
    /// [`Self::with_events`] instead — a build that silently drops every
    /// `SystemEvent` a service publishes is a trap for whichever adapter
    /// reaches for the constructor with the fewest arguments, not a
    /// legitimate production shape.
    #[cfg(test)]
    pub fn new(pool: &PgPool) -> Self {
        Self::with_events(pool, &EventBroadcaster::default())
    }

    /// Built by a composition root that already has the process's real
    /// `EventBroadcaster` (`app.rs`), so a publishing service sends on the
    /// same channel HTTP and chat subscribe to.
    pub fn with_events(pool: &PgPool, events: &EventBroadcaster) -> Self {
        let settings = SettingsService::new(pool.clone(), events.clone());
        Self {
            rooms: RoomService::new(pool.clone(), settings.clone()),
            settings,
        }
    }
}
