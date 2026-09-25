//! Use cases. A service method is the unit of transaction: it validates a
//! command through the domain, opens the transaction, calls repositories,
//! publishes the resulting event and commits. HTTP, Matrix, MCP and LLM
//! tools all call the same method. Never imports a transport
//! (`tests/architecture.rs`).

pub mod rooms;

use sqlx::PgPool;

use self::rooms::RoomService;

/// Why a use case failed, by boundary rather than by cause.
///
/// Adapters decide what each variant means to their caller (`interface/http`
/// maps it to `ApiError`); the message of the first four is safe to show a
/// user, the payload of the last two is diagnostic and must not be.
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
    #[error("external service error: {0}")]
    External(String),
}

/// Every application service, built once by the composition root (`app.rs`)
/// and handed to each front door — `ApiState` and the Matrix `ModuleContext`
/// — so no adapter constructs a service of its own.
///
/// Each service is cheap to clone (a `PgPool` is a handle), so this is too.
/// A feature joins by adding its service as a field here and building it in
/// [`AppServices::new`].
#[derive(Clone)]
pub struct AppServices {
    pub rooms: RoomService,
}

impl AppServices {
    pub fn new(pool: &PgPool) -> Self {
        Self {
            rooms: RoomService::new(pool.clone()),
        }
    }
}
