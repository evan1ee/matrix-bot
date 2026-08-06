use anyhow::Result;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::bot::client::BotClient;
use crate::bot::handler::register_handlers;
use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::core::CoreModule;
use crate::modules::reminder::ReminderModule;
use crate::modules::todo::TodoModule;
use crate::modules::{ModuleContext, ModuleRegistry};

pub mod client;
pub mod dispatcher;
pub mod handler;

pub struct MatrixBot {
    client: Client,
    db: Database,
    registry: Arc<ModuleRegistry>,
    broadcaster: Option<EventBroadcaster>,
}

impl MatrixBot {
    #[allow(dead_code)]
    pub async fn new(config: &Config) -> Result<Self> {
        let db = Database::connect(&config.database_url, 10).await?;
        Self::new_with_db(config, db).await
    }

    #[allow(dead_code)]
    pub async fn new_with_db(config: &Config, db: Database) -> Result<Self> {
        Self::new_with_db_and_broadcaster(config, db, None).await
    }

    pub async fn new_with_db_and_broadcaster(
        config: &Config,
        db: Database,
        broadcaster: Option<EventBroadcaster>,
    ) -> Result<Self> {
        Self::new_full(config, db, broadcaster, None, None).await
    }

    pub async fn new_full(
        config: &Config,
        db: Database,
        broadcaster: Option<EventBroadcaster>,
        token_handle: Option<Arc<RwLock<Option<String>>>>,
        client_handle: Option<Arc<RwLock<Option<Client>>>>,
    ) -> Result<Self> {
        let client = BotClient::build_and_login_with_token_handles(config, token_handle, client_handle.clone()).await?;

        if let Some(ref handle) = client_handle {
            *handle.write().await = Some(client.clone());
        }

        let registry_ref = Arc::new(RwLock::new(None));
        let mut registry = ModuleRegistry::new();

        // Register Core, Todo, and Reminder modules
        registry.register(CoreModule::new(registry_ref.clone()));
        registry.register(TodoModule::new());
        registry.register(ReminderModule::new());

        let registry_arc = Arc::new(registry);
        *registry_ref.write().await = Some(registry_arc.clone());

        Ok(Self {
            client,
            db,
            registry: registry_arc,
            broadcaster,
        })
    }

    #[allow(dead_code)]
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn run(self) -> Result<()> {
        let module_ctx = ModuleContext {
            client: self.client.clone(),
            db: self.db.clone(),
            broadcaster: self.broadcaster.clone(),
        };

        // Register Matrix event handlers
        register_handlers(&self.client, self.registry.clone(), self.db.clone());

        // Start background workers for all modules (e.g., reminder scheduler)
        self.registry.spawn_all_background_workers(&module_ctx).await;

        tracing::info!("Matrix Bot is active and listening for events...");
        self.client.sync(SyncSettings::default()).await?;

        Ok(())
    }
}
