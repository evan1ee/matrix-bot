//! The chat modules Matrix registers, in dispatch order.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::Config;
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
use crate::interface::chat::ModuleRegistry;

/// Builds the registry once per process. Returned shared because `CoreModule`
/// reads the registry back for `!help`.
pub async fn build_modules(config: &Config) -> Arc<ModuleRegistry> {
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
    registry.register(crate::interface::chat::mcp::McpModule::new());
    // Must precede LlmModule: `LlmModule::handle_command` claims any
    // non-empty text sent as (or mentioning it in) a direct message, not
    // just `!ai` — see `interface/chat/llm.rs`.
    // Registered after it, `!say` in a DM would be swallowed and answered
    // by the AI instead of TTS. Covered by
    // `interface::chat::tests::test_tts_must_be_registered_before_a_direct_message_catch_all`.
    registry.register(crate::interface::chat::tts::TtsModule::new(
        config.tts_client(),
        config.tts_voice.clone(),
    ));
    registry.register(crate::interface::chat::llm::LlmModule::new(
        config.ai.clone(),
    ));
    registry.register(crate::interface::chat::account::AccountModule::new(
        config.web_url.clone(),
    ));

    let registry = Arc::new(registry);
    *registry_ref.write().await = Some(registry.clone());
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The order is behaviour: the first module to claim a command wins.
    #[tokio::test]
    async fn the_registration_order_is_the_one_matrix_always_had() {
        let modules = build_modules(&Config::for_test()).await;
        let names: Vec<String> = modules.get_all_help().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names.first().map(String::as_str), Some("Core"));
        assert_eq!(names.last().map(String::as_str), Some("Account"));
        let position = |name: &str| names.iter().position(|n| n == name);
        let tts = position("Text-to-Speech");
        let ai = position("AI Assistant");
        assert!(tts.is_some() && ai.is_some(), "{names:?}");
        assert!(tts < ai, "TTS must precede the AI catch-all: {names:?}");
        assert_eq!(names.len(), 18, "{names:?}");
    }
}
