use anyhow::Result;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::bot::client::BotClient;
use crate::bot::handler::register_handlers;
use crate::bot::verification::{register_verification_handlers, VerificationPolicy};
use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::core::CoreModule;
use crate::modules::finance::FinanceModule;
use crate::modules::footprints::FootprintsModule;
use crate::modules::habits::HabitsModule;
use crate::modules::life::LifeModule;
use crate::modules::portfolio::PortfolioModule;
use crate::modules::recurring_bills::RecurringBillsModule;
use crate::modules::reminder::ReminderModule;
use crate::modules::settings::SettingsModule;
use crate::modules::todo::TodoModule;
use crate::modules::webhook::WebhookModule;
use crate::modules::{ModuleContext, ModuleRegistry};

pub mod client;
pub mod dispatcher;
pub mod handler;
pub mod linking;
pub mod media;
pub mod notify;
pub mod quote;
pub mod self_verify;
pub mod stage;
pub mod verification;

pub struct MatrixBot {
    client: Client,
    db: Database,
    registry: Arc<ModuleRegistry>,
    broadcaster: Option<EventBroadcaster>,
    /// Shared with the API, so a merge confirmed in the web UI is the same one
    /// `!link` recorded.
    pending_merges: Option<Arc<crate::bot::linking::PendingMerges>>,
}

impl MatrixBot {
    pub async fn new_full(
        config: &Config,
        db: Database,
        broadcaster: Option<EventBroadcaster>,
        token_handle: Option<Arc<RwLock<Option<String>>>>,
        client_handle: Option<Arc<RwLock<Option<Client>>>>,
        pending_merges: Option<Arc<crate::bot::linking::PendingMerges>>,
    ) -> Result<Self> {
        let client = BotClient::build_and_login_with_token_handles(
            config,
            token_handle,
            client_handle.clone(),
        )
        .await?;

        if let Some(ref handle) = client_handle {
            *handle.write().await = Some(client.clone());
        }

        // Answer verification requests so the bot's session can be signed into
        // another user's trust; without this its sessions stay Unverified.
        let own_user = client
            .user_id()
            .map(|u| u.to_string())
            .unwrap_or_else(|| config.username.clone());
        register_verification_handlers(
            &client,
            VerificationPolicy::new(own_user, config.verification_allowlist.clone()),
        );

        let registry_ref = Arc::new(RwLock::new(None));
        let mut registry = ModuleRegistry::new();

        registry.register(CoreModule::new(registry_ref.clone()));
        registry.register(TodoModule::new());
        registry.register(ReminderModule::new());
        registry.register(FinanceModule::new());
        registry.register(PortfolioModule::new());
        registry.register(FootprintsModule::new());
        registry.register(HabitsModule::new());
        registry.register(LifeModule::new());
        registry.register(crate::modules::journal::JournalModule::new());
        registry.register(crate::modules::bookmarks::BookmarksModule::new());
        registry.register(RecurringBillsModule::new());
        registry.register(SettingsModule::new());
        registry.register(WebhookModule::new());
        registry.register(crate::modules::ai::AiModule::new(
            client_handle.clone().unwrap_or_default(),
        ));
        registry.register(crate::modules::account::AccountModule::new(
            client_handle.clone().unwrap_or_default(),
            config.web_url.clone(),
        ));

        let registry_arc = Arc::new(registry);
        *registry_ref.write().await = Some(registry_arc.clone());

        Ok(Self {
            client,
            db,
            registry: registry_arc,
            broadcaster,
            pending_merges,
        })
    }

    pub async fn run(self) -> Result<()> {
        let module_ctx = ModuleContext {
            client: self.client.clone(),
            db: self.db.clone(),
            broadcaster: self.broadcaster.clone(),
        };

        // Register Matrix event handlers
        register_handlers(
            &self.client,
            self.registry.clone(),
            self.db.clone(),
            self.broadcaster.clone(),
            self.pending_merges.clone(),
        );

        // Start background workers for all modules (e.g., reminder scheduler).
        // The guard stops them when this function returns, by any path: they
        // hold this generation's client, and the caller reconnects in a loop, so
        // leaving them detached accumulated one scheduler per reconnect and
        // delivered every reminder once per copy.
        let _workers = self
            .registry
            .spawn_all_background_workers(&module_ctx)
            .await;

        tracing::info!("Matrix Bot is active and listening for events...");
        self.client.sync(SyncSettings::default()).await?;

        Ok(())
    }
}
