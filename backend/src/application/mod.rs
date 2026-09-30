//! Use cases. A service method is the unit of transaction: it validates a
//! command through the domain, opens the transaction, calls repositories,
//! publishes the resulting event and commits. HTTP, Matrix, MCP and LLM
//! tools all call the same method. Never imports a transport
//! (`tests/architecture.rs`).

pub mod accounts;
pub mod bookmarks;
pub mod briefing;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod life;
pub mod llm;
pub mod outbox;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todos;
pub mod webhooks;

use sqlx::PgPool;

use self::accounts::AccountService;
use self::bookmarks::BookmarkService;
use self::briefing::BriefingService;
use self::finance::FinanceService;
use self::footprints::FootprintService;
use self::habits::HabitService;
use self::life::LifeService;
use self::llm::LlmService;
use self::outbox::OutboxService;
use self::portfolio::PortfolioService;
use self::recurring_bills::RecurringBillsService;
use self::reminders::ReminderService;
use self::rooms::RoomService;
use self::settings::SettingsService;
use self::todos::TodoService;
use self::webhooks::WebhookService;
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

impl From<&'static str> for AppError {
    fn from(s: &'static str) -> Self {
        AppError::Validation(s.to_string())
    }
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
    pub accounts: AccountService,
    pub bookmarks: BookmarkService,
    pub briefing: BriefingService,
    pub finance: FinanceService,
    pub footprints: FootprintService,
    pub habits: HabitService,
    pub life: LifeService,
    pub llm: LlmService,
    pub outbox: OutboxService,
    pub portfolio: PortfolioService,
    pub recurring_bills: RecurringBillsService,
    pub reminders: ReminderService,
    pub rooms: RoomService,
    pub settings: SettingsService,
    pub todos: TodoService,
    pub webhooks: WebhookService,
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
        let accounts = AccountService::new(pool.clone(), events.clone());
        let bookmarks = BookmarkService::new(pool.clone(), events.clone());
        let settings = SettingsService::new(pool.clone(), events.clone());
        let briefing = BriefingService::new(
            pool.clone(),
            events.clone(),
            settings.clone(),
            accounts.clone(),
        );
        let finance = FinanceService::new(pool.clone());
        let footprints = FootprintService::new(pool.clone(), events.clone(), settings.clone());
        let habits = HabitService::new(pool.clone(), events.clone());
        let life = LifeService::new(pool.clone(), events.clone());
        let llm = LlmService::new(pool.clone(), events.clone());
        let rooms = RoomService::new(pool.clone(), settings.clone(), accounts.clone());
        let reminders = ReminderService::new(pool.clone());
        let todos = TodoService::new(pool.clone(), reminders.clone());
        let recurring_bills =
            RecurringBillsService::new(pool.clone(), reminders.clone(), finance.clone());
        let webhooks = WebhookService::new(pool.clone(), events.clone());
        let portfolio = PortfolioService::new(pool.clone());
        let outbox = OutboxService::new(pool.clone());
        Self {
            accounts,
            bookmarks,
            briefing,
            finance,
            footprints,
            habits,
            life,
            llm,
            outbox,
            portfolio,
            recurring_bills,
            reminders,
            rooms,
            settings,
            todos,
            webhooks,
        }
    }
}
