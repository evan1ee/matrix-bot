use anyhow::Result;
use chrono::Utc;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::{Client, LoopCtrl};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::application::AppServices;
use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::channels::matrix::client::BotClient;
use crate::interface::channels::matrix::handler::register_handlers;
use crate::interface::channels::matrix::verification::{
    register_verification_handlers, VerificationPolicy,
};
use crate::interface::chat::briefing::BriefingModule;
use crate::interface::chat::core::CoreModule;
use crate::interface::chat::finance::FinanceModule;
use crate::interface::chat::footprints::FootprintsModule;
use crate::interface::chat::habits::HabitsModule;
use crate::interface::chat::journal::JournalModule;
use crate::interface::chat::life::LifeModule;
use crate::interface::chat::portfolio::PortfolioModule;
use crate::interface::chat::recurring_bills::RecurringBillsModule;
use crate::interface::chat::reminders::ReminderModule;
use crate::interface::chat::settings::SettingsModule;
use crate::interface::chat::todos::TodoModule;
use crate::interface::chat::webhooks::WebhookModule;
use crate::modules::{ModuleContext, ModuleRegistry};

pub mod attachments;
pub mod client;
pub mod command_flow;
pub mod dispatcher;
pub mod handler;
pub mod handles;
pub mod invite;
pub mod linking;
pub mod media;
pub mod notify;
pub mod quote;
pub mod reaction;
pub mod reaction_targets;
pub mod reply;
pub mod self_verify;
pub mod stage;
pub mod sync_health;
pub mod sync_loop;
pub mod verification;

pub struct MatrixBot {
    client: Client,
    db: Database,
    services: AppServices,
    registry: Arc<ModuleRegistry>,
    broadcaster: Option<EventBroadcaster>,
    /// Shared with the API, so a merge confirmed in the web UI is the same one
    /// `!link` recorded.
    handles: crate::interface::channels::matrix::handles::BotHandles,
    config: Config,
}

impl MatrixBot {
    pub async fn new_full(
        config: &Config,
        db: Database,
        services: AppServices,
        broadcaster: Option<EventBroadcaster>,
        handles: crate::interface::channels::matrix::handles::BotHandles,
    ) -> Result<Self> {
        let client = BotClient::build_and_login_with_token_handles(
            config,
            Some(handles.access_token.clone()),
            Some(handles.client.clone()),
        )
        .await?;

        *handles.client.write().await = Some(client.clone());

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
        registry.register(JournalModule::new());
        registry.register(crate::interface::chat::bookmarks::BookmarksModule::new());
        registry.register(RecurringBillsModule::new());
        registry.register(SettingsModule::new());
        registry.register(BriefingModule::new());
        registry.register(WebhookModule::new());
        registry.register(crate::modules::mcp::McpModule::new());
        // Must precede AiModule: `AiModule::handle_command` claims any
        // non-empty text sent as (or mentioning it in) a direct message, not
        // just `!ai` — see `modules/ai/handler.rs::handle_ai_command_with_stage`.
        // Registered after it, `!say` in a DM would be swallowed and answered
        // by the AI instead of TTS. Covered by
        // `modules::tests::test_tts_must_be_registered_before_a_direct_message_catch_all`.
        registry.register(crate::interface::chat::tts::TtsModule::new(
            handles.client.clone(),
            config.tts_client(),
            config.tts_voice.clone(),
        ));
        registry.register(crate::modules::ai::AiModule::new(
            handles.client.clone(),
            config.ai.clone(),
        ));
        registry.register(crate::interface::chat::account::AccountModule::new(
            handles.client.clone(),
            config.web_url.clone(),
        ));

        let registry_arc = Arc::new(registry);
        *registry_ref.write().await = Some(registry_arc.clone());

        Ok(Self {
            client,
            db,
            services,
            registry: registry_arc,
            broadcaster,
            handles,
            config: config.clone(),
        })
    }

    pub async fn run(self) -> Result<()> {
        let module_ctx = ModuleContext {
            client: self.client.clone(),
            db: self.db.clone(),
            services: self.services.clone(),
            broadcaster: self.broadcaster.clone(),
        };

        // Register Matrix event handlers
        register_handlers(
            &self.client,
            &self.config,
            self.registry.clone(),
            self.db.clone(),
            self.services.clone(),
            self.broadcaster.clone(),
            Some(self.handles.pending_merges.clone()),
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

        // `sync_with_callback` (rather than plain `sync`) is the hook `/health/ready`
        // needs: it runs the callback after every successful response, which is
        // this loop's only clean signal that "a sync completed" — the point at
        // which the client's local state is caught up with the homeserver.
        let sync_health = self.handles.sync_health.clone();
        self.client
            .sync_with_callback(SyncSettings::default(), move |_response| {
                sync_health.record_sync(Utc::now());
                async { LoopCtrl::Continue }
            })
            .await?;

        Ok(())
    }
}
