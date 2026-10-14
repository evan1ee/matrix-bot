//! Use cases. A service method is the unit of transaction: it validates a
//! command through the domain, opens the transaction, calls repositories,
//! publishes the resulting event and commits. HTTP, Matrix, MCP and LLM
//! tools all call the same method. Never imports a transport
//! (`tests/architecture.rs`).

pub mod accounts;
pub mod agent;
pub mod bookmarks;
pub mod briefing;
pub mod config;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod identity;
pub mod knowledge;
pub mod life;
pub mod llm;
pub mod mcp;
pub mod outbox;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod secrets;
pub mod settings;
pub mod todos;
pub mod webhooks;

use std::sync::Arc;

use sqlx::PgPool;

use crate::infrastructure::storage::StorageService;

use self::accounts::AccountService;
use self::agent::AgentService;
use self::bookmarks::BookmarkService;
use self::briefing::BriefingService;
use self::config::AppConfigService;
use self::finance::FinanceService;
use self::footprints::FootprintService;
use self::habits::HabitService;
use self::identity::{IdentityService, LinkService};
use self::knowledge::{NoteService, SearchService};
use self::life::LifeService;
use self::llm::LlmService;
use self::mcp::McpService;
use self::outbox::OutboxService;
use self::portfolio::PortfolioService;
use self::recurring_bills::RecurringBillsService;
use self::reminders::ReminderService;
use self::rooms::RoomService;
use self::secrets::SecretService;
use self::settings::SettingsService;
use self::todos::TodoService;
use self::webhooks::WebhookService;

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
/// `ApiState`, the Matrix module registry and the job handlers — so no
/// adapter constructs a service of its own.
///
/// Each service is cheap to clone (a `PgPool` is a handle), so this is too.
/// A feature joins by adding its service as a field here and building it in
/// [`AppServices::new`].
#[derive(Clone)]
pub struct AppServices {
    pub accounts: AccountService,
    pub agent: AgentService,
    pub bookmarks: BookmarkService,
    pub briefing: BriefingService,
    pub finance: FinanceService,
    pub footprints: FootprintService,
    pub habits: HabitService,
    pub identity: IdentityService,
    pub life: LifeService,
    pub link: LinkService,
    pub llm: LlmService,
    pub mcp: McpService,
    pub outbox: OutboxService,
    pub portfolio: PortfolioService,
    pub recurring_bills: RecurringBillsService,
    pub reminders: ReminderService,
    pub rooms: RoomService,
    pub settings: SettingsService,
    pub storage: Arc<dyn StorageService>,
    pub todos: TodoService,
    pub webhooks: WebhookService,
    pub notes: NoteService,
    pub knowledge_search: SearchService,
    pub app_config: AppConfigService,
    pub secrets: SecretService,
}

impl AppServices {
    /// Built once per process by a composition root. Services publish
    /// their events through the outbox inside their own transactions, so no
    /// event plumbing is passed in.
    pub fn new(pool: &PgPool) -> Self {
        let accounts = AccountService::new(pool.clone());
        let agent = AgentService::new(pool.clone());
        let bookmarks = BookmarkService::new(pool.clone());
        let settings = SettingsService::new(pool.clone());
        let briefing = BriefingService::new(pool.clone(), settings.clone(), accounts.clone());
        let finance = FinanceService::new(pool.clone());
        let footprints = FootprintService::new(pool.clone(), settings.clone());
        let habits = HabitService::new(pool.clone());
        let identity = IdentityService::new(pool.clone());
        let life = LifeService::new(pool.clone());
        // Boot wiring replaces this with the deployment's `WEB_URL` and
        // allowlist through `with_link_config`.
        let app_config = AppConfigService::new(pool.clone());
        let link = LinkService::new(pool.clone(), String::new(), Default::default())
            .with_app_config(app_config.clone());
        let llm = LlmService::new(pool.clone());
        let mcp = McpService::new(pool.clone());
        let rooms = RoomService::new(pool.clone(), settings.clone(), identity.clone());
        let reminders = ReminderService::new(pool.clone());
        let todos = TodoService::new(pool.clone(), reminders.clone());
        let recurring_bills =
            RecurringBillsService::new(pool.clone(), reminders.clone(), finance.clone());
        let webhooks = WebhookService::new(pool.clone());
        let portfolio = PortfolioService::new(pool.clone());
        let outbox = OutboxService::new(pool.clone());
        let protector: Arc<dyn crate::infrastructure::crypto::SecretProtector> =
            Arc::new(crate::infrastructure::crypto::FailClosedSecretProtector);
        let secrets = SecretService::new(pool.clone(), protector);
        let storage = crate::infrastructure::storage::LocalStorageService::new(
            "data/storage",
            None,
            "ephemeral-signing-secret",
        )
        .map(|s| Arc::new(s) as Arc<dyn StorageService>)
        .unwrap_or_else(|_| {
            let tmp = std::env::temp_dir().join("matrix_bot_storage");
            Arc::new(
                crate::infrastructure::storage::LocalStorageService::new(
                    tmp,
                    None,
                    "ephemeral-signing-secret",
                )
                .expect("temp storage fallback must initialize"),
            )
        });
        Self {
            accounts,
            agent,
            bookmarks,
            briefing,
            finance,
            footprints,
            habits,
            identity,
            life,
            link,
            llm,
            mcp,
            outbox,
            portfolio,
            recurring_bills,
            reminders,
            rooms,
            settings,
            storage,
            todos,
            webhooks,
            notes: NoteService::new(pool.clone()),
            knowledge_search: SearchService::new(
                pool.clone(),
                Arc::new(
                    crate::infrastructure::inference::embedding::MockEmbeddingProvider::default(),
                ),
            )
            .with_app_config(app_config.clone()),
            app_config,
            secrets,
        }
    }

    /// Gives the link flow the deployment's web origin (where the landing page
    /// is served) and its signup allowlist (spec D4). Called by the
    /// composition roots; the default has neither.
    pub fn with_link_config(
        mut self,
        web_url: String,
        signup_allowlist: std::collections::HashMap<crate::domain::channels::Channel, Vec<String>>,
    ) -> Self {
        self.link = self.link.with_config(web_url, signup_allowlist);
        self
    }

    /// Gives the services tree the configured object storage service.
    pub fn with_storage(mut self, storage: Arc<dyn StorageService>) -> Self {
        self.storage = storage;
        self
    }

    /// Sets the configured embedding model: new chunks are tagged with it and
    /// vector search reads only its chunks.
    pub fn with_embedding_model(mut self, model: &str) -> Self {
        self.knowledge_search = self.knowledge_search.with_model(model);
        self
    }

    /// Configures the embedding provider used for semantic and hybrid knowledge searches.
    pub fn with_embedder(
        mut self,
        embedder: Arc<dyn crate::infrastructure::inference::embedding::EmbeddingProvider>,
    ) -> Self {
        self.knowledge_search = self.knowledge_search.with_embedder(embedder);
        self
    }

    /// Configures the secret protector used for credential encryption and decryption.
    pub fn with_secret_protector(
        mut self,
        protector: Arc<dyn crate::infrastructure::crypto::SecretProtector>,
    ) -> Self {
        self.secrets = self.secrets.with_protector(protector);
        self
    }

    /// Sets a pre-loaded application configuration service.
    pub fn with_app_config(mut self, app_config: AppConfigService) -> Self {
        self.link = self.link.with_app_config(app_config.clone());
        self.knowledge_search = self.knowledge_search.with_app_config(app_config.clone());
        self.app_config = app_config;
        self
    }

    /// Restricts room and destination resolution to the channels configured on
    /// this deployment.
    pub fn with_configured_channels(
        mut self,
        channels: impl IntoIterator<Item = crate::domain::channels::Channel>,
    ) -> Self {
        self.rooms = self.rooms.with_configured_channels(channels);
        self
    }
}
