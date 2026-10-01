use async_trait::async_trait;

use crate::interface::channels::message::Reply;
use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::error;

use crate::application::settings::SettingsService;
use crate::application::AppServices;
use crate::db::{Database, DbPool};
use crate::domain::accounts::AccountId;
use crate::infrastructure::postgres::repositories::llm as llm_repo;
use crate::interface::chat::llm_alert::{clear_ai_error_state, notify_dev_room_ai_error};
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
use crate::llm::context::LlmTurnContext;
use crate::llm::prompt::format_response;
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::llm::turn::{run_turn, MAX_PROMPT_LENGTH};
use crate::utils::html::html_escape;

pub struct LlmModule {
    client_handle: Arc<RwLock<Option<Client>>>,
    ai: crate::config::AiEnvConfig,
}

impl Default for LlmModule {
    fn default() -> Self {
        Self::new(
            Arc::new(RwLock::new(None)),
            crate::config::AiEnvConfig::default(),
        )
    }
}

impl LlmModule {
    pub fn new(client_handle: Arc<RwLock<Option<Client>>>, ai: crate::config::AiEnvConfig) -> Self {
        Self { client_handle, ai }
    }
}

pub type AiModule = LlmModule;

#[async_trait]
impl BotModule for LlmModule {
    fn name(&self) -> &str {
        "AI Assistant"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "ai".to_string(),
                usage: "!ai <prompt>".to_string(),
                description: "Chat with the AI assistant".to_string(),
            },
            CommandHelp {
                name: "ai reset".to_string(),
                usage: "!ai reset".to_string(),
                description: "Reset conversational memory in this room".to_string(),
            },
            CommandHelp {
                name: "ai model".to_string(),
                usage: "!ai model [name]".to_string(),
                description: "View or switch the active AI model".to_string(),
            },
            CommandHelp {
                name: "ai config".to_string(),
                usage: "!ai config".to_string(),
                description: "View AI backend endpoint and configuration".to_string(),
            },
            CommandHelp {
                name: "ai retro".to_string(),
                usage: "!ai retro".to_string(),
                description: "Generate a weekly executive retrospective".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        db: &Database,
    ) -> Option<Reply> {
        handle_ai_command_with_stage(
            services,
            &db.pool,
            &self.ai,
            Some(&self.client_handle),
            &ctx.room_key(),
            &ctx.owner_id,
            ctx.sender_name(),
            &ctx.command,
            &ctx.raw_args,
            &ctx.full_body,
            ctx.is_direct,
            ctx.is_mentioned,
            ctx.quoted.as_ref(),
            ctx.progress.as_deref(),
        )
        .await
        .map(Reply::from)
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_ai_command(
    services: &AppServices,
    pool: &DbPool,
    ai: &crate::config::AiEnvConfig,
    client_handle: Option<&Arc<RwLock<Option<Client>>>>,
    room_id: &str,
    owner_id: &AccountId,
    sender_name: &str,
    command: &str,
    raw_args: &str,
    full_body: &str,
    is_direct: bool,
    is_mentioned: bool,
    quoted: Option<&crate::domain::chat::QuotedMessage>,
) -> Option<String> {
    handle_ai_command_with_stage(
        services,
        pool,
        ai,
        client_handle,
        room_id,
        owner_id,
        sender_name,
        command,
        raw_args,
        full_body,
        is_direct,
        is_mentioned,
        quoted,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_ai_command_with_stage(
    services: &AppServices,
    pool: &DbPool,
    ai: &crate::config::AiEnvConfig,
    client_handle: Option<&Arc<RwLock<Option<Client>>>>,
    room_id: &str,
    owner_id: &AccountId,
    _sender_name: &str,
    command: &str,
    raw_args: &str,
    full_body: &str,
    is_direct: bool,
    is_mentioned: bool,
    quoted: Option<&crate::domain::chat::QuotedMessage>,
    progress: Option<&dyn crate::llm::progress::ProgressSink>,
) -> Option<String> {
    let cmd_lower = command.to_lowercase();
    let prompt = if cmd_lower == "ai" || cmd_lower == "ask" {
        let trimmed = raw_args.trim();
        if trimmed.is_empty() && quoted.is_none() {
            return Some(format_help());
        }
        if let Some(sub_reply) =
            handle_ai_subcommand(pool, services, ai, owner_id, room_id, trimmed).await
        {
            return Some(sub_reply);
        }
        trimmed
    } else if is_direct || is_mentioned {
        let trimmed = full_body.trim();
        if trimmed.is_empty() {
            if quoted.is_some() {
                ""
            } else {
                return None;
            }
        } else {
            trimmed
        }
    } else {
        return None;
    };

    let effective_prompt = if let Some(q) = quoted {
        q.to_context_prompt(prompt)
    } else {
        prompt.to_string()
    };

    if effective_prompt.len() > MAX_PROMPT_LENGTH {
        return Some(format_error(&format!(
            "Prompt exceeds maximum length of {} characters.",
            MAX_PROMPT_LENGTH
        )));
    }

    let turn_ctx = LlmTurnContext::new(services, pool, ai, owner_id, room_id, progress);

    match run_turn(&turn_ctx, &effective_prompt, None).await {
        Ok(result) => {
            clear_ai_error_state();
            crate::interface::channels::matrix::reaction_targets::record_ai_tool_targets(
                pool,
                progress.and_then(|p| p.message_ref()).as_deref(),
                room_id,
                owner_id,
                &result.tool_results,
            )
            .await;
            Some(format_response(
                &result.message,
                &result.model_used,
                None,
                &result.tool_calls_executed,
            ))
        }
        Err(err) => {
            error!("AI execution failed: {}", err);
            notify_dev_room_ai_error(
                &services.settings,
                client_handle,
                owner_id,
                &err.to_string(),
            )
            .await;
            None
        }
    }
}

pub fn format_error(msg: &str) -> String {
    format!("⚠️ <b>AI Assistant Error:</b> {}", html_escape(msg))
}

pub fn format_help() -> String {
    "🤖 <b>AI Assistant Commands:</b><br/>\
     • <code>!ai &lt;prompt&gt;</code> — Chat with AI assistant (supports multi-action capture)<br/>\
     • <code>!ai retro</code> — Generate weekly executive retrospective<br/>\
     • <code>!ai reset</code> — Clear conversational memory in this room<br/>\
     • <code>!ai model &lt;name&gt;</code> — Switch active model<br/>\
     • <code>!ai history &lt;1-100&gt;</code> — Set max context history messages<br/>\
     • <code>!ai config</code> — View active AI endpoint & model".to_string()
}

pub async fn handle_ai_subcommand(
    pool: &DbPool,
    services: &AppServices,
    ai: &crate::config::AiEnvConfig,
    owner_id: &AccountId,
    room_id: &str,
    trimmed: &str,
) -> Option<String> {
    let settings = &services.settings;
    if trimmed.eq_ignore_ascii_case("retro")
        || trimmed.eq_ignore_ascii_case("retrospective")
        || trimmed.eq_ignore_ascii_case("weekly")
    {
        return Some(
            crate::llm::retrospective::handle_retrospective_command(pool, settings, owner_id).await,
        );
    }
    if trimmed.eq_ignore_ascii_case("reset") || trimmed.eq_ignore_ascii_case("clear") {
        return Some(handle_reset(services, owner_id, room_id).await);
    }
    if trimmed.eq_ignore_ascii_case("config") {
        return Some(handle_show_config(pool, settings, ai, owner_id).await);
    }
    if let Some(rest) = trimmed.strip_prefix("model") {
        let model_arg = rest.trim();
        if model_arg.is_empty() {
            return Some(handle_show_config(pool, settings, ai, owner_id).await);
        }
        if model_arg.len() > 500 {
            return Some(format_error(
                "Model name exceeds maximum length of 500 characters.",
            ));
        }
        return Some(handle_set_model(services, owner_id, model_arg).await);
    }
    if let Some(rest) = trimmed
        .strip_prefix("history")
        .or_else(|| trimmed.strip_prefix("turns"))
    {
        let num_arg = rest.trim();
        if num_arg.is_empty() {
            return Some(handle_show_config(pool, settings, ai, owner_id).await);
        }
        return Some(handle_set_history(settings, owner_id, num_arg).await);
    }
    None
}

pub async fn handle_show_config(
    pool: &DbPool,
    settings: &SettingsService,
    ai: &crate::config::AiEnvConfig,
    owner_id: &AccountId,
) -> String {
    let user_config = llm_repo::get_user_config(pool, owner_id)
        .await
        .unwrap_or(None);
    let resolved = ResolvedAiConfig::from_user_config(ai, user_config.as_ref());
    let settings = settings.get_or_default(owner_id).await;

    let key_status = if resolved.api_key.is_some() {
        "Configured (Hidden)"
    } else {
        "None"
    };

    format!(
        "🤖 <b>AI Assistant Configuration:</b><br/>\
         • <b>Model:</b> <code>{}</code><br/>\
         • <b>Endpoint:</b> <code>{}</code><br/>\
         • <b>API Key:</b> {}<br/>\
         • <b>Temperature:</b> <code>{}</code><br/>\
         • <b>Max History Turns:</b> <code>{}</code>",
        html_escape(&resolved.model_name),
        html_escape(&resolved.api_base_url),
        key_status,
        resolved.temperature,
        settings.max_history_turns
    )
}

pub async fn handle_set_history(
    settings: &SettingsService,
    owner_id: &AccountId,
    turns_str: &str,
) -> String {
    let turns: i64 = match turns_str.parse() {
        Ok(n) if (1..=100).contains(&n) => n,
        _ => return format_error("Max history turns must be an integer between 1 and 100."),
    };

    match settings
        .modify(owner_id, |current| {
            current.max_history_turns = turns;
            Ok(())
        })
        .await
    {
        Ok(saved) => format!(
            "✅ AI max history turns updated to: <code>{}</code>",
            saved.max_history_turns
        ),
        Err(e) => {
            error!("Failed to update max history turns: {}", e);
            format_error("Failed to update max history turns setting.")
        }
    }
}

pub async fn handle_set_model(
    services: &AppServices,
    owner_id: &AccountId,
    model_name: &str,
) -> String {
    let input = crate::domain::llm::UpdateAiConfigInput {
        model_name: Some(model_name.to_string()),
        ..Default::default()
    };

    match services.llm.update_user_config(owner_id, &input).await {
        Ok(cfg) => {
            clear_ai_error_state();
            let model = cfg.model_name.unwrap_or_else(|| model_name.to_string());
            format!(
                "✅ AI model updated to: <code>{}</code>",
                html_escape(&model)
            )
        }
        Err(e) => {
            error!("Failed to update AI model: {}", e);
            format_error("Failed to update AI model configuration.")
        }
    }
}

pub async fn handle_reset(services: &AppServices, owner_id: &AccountId, room_id: &str) -> String {
    if let Ok(conv) = services
        .llm
        .get_or_create_conversation(owner_id, room_id, 1800)
        .await
    {
        // The service publishes AiConversationDeleted when a row was deleted.
        let _ = services.llm.delete_conversation(&conv.id, owner_id).await;
    }
    "🧹 <b>AI conversation history reset.</b> Next message starts a fresh turn.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test() -> (Database, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let db = Database { pool };
        let owner = crate::application::AppServices::new(&db.pool)
            .identity
            .provision_matrix_for_test("@user:mbm.one")
            .await;
        (db, owner)
    }

    #[tokio::test]
    async fn test_ai_help_and_config_commands() {
        let (db, owner) = setup_test().await;

        let help = handle_ai_command(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &crate::config::AiEnvConfig::default(),
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "",
            "",
            false,
            false,
            None,
        )
        .await
        .unwrap();

        assert!(help.contains("AI Assistant Commands"));
        assert!(help.contains("!ai &lt;prompt&gt;"));

        let cfg = handle_ai_command(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &crate::config::AiEnvConfig::default(),
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "config",
            "",
            false,
            false,
            None,
        )
        .await
        .unwrap();

        assert!(cfg.contains("AI Assistant Configuration"));
    }

    #[tokio::test]
    async fn test_ai_model_switch_and_reset_commands() {
        let (db, owner) = setup_test().await;

        let model_reply = handle_ai_command(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &crate::config::AiEnvConfig::default(),
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "model claude-3-5-sonnet-20241022",
            "",
            false,
            false,
            None,
        )
        .await
        .unwrap();

        assert!(
            model_reply.contains("AI model updated to: <code>claude-3-5-sonnet-20241022</code>")
        );

        let saved = llm_repo::get_user_config(&db.pool, &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.model_name.as_deref(),
            Some("claude-3-5-sonnet-20241022")
        );

        let conv = llm_repo::get_or_create_conversation(&db.pool, &owner, "!room:mbm.one", 1800)
            .await
            .unwrap();
        assert_eq!(conv.owner_id, owner.to_string());

        let reset_reply = handle_ai_command(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &crate::config::AiEnvConfig::default(),
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "reset",
            "",
            false,
            false,
            None,
        )
        .await
        .unwrap();

        assert!(reset_reply.contains("AI conversation history reset"));
        assert_eq!(
            crate::infrastructure::postgres::repositories::outbox::event_types_for(
                &db.pool, &owner
            )
            .await
            .unwrap(),
            vec!["AiConfigUpdated", "AiConversationDeleted"]
        );
    }

    #[tokio::test]
    async fn test_ai_mention_natural_language_routing() {
        let (db, owner) = setup_test().await;

        let result = handle_ai_command(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &crate::config::AiEnvConfig::default(),
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "",
            "",
            "",
            false,
            false,
            None,
        )
        .await;
        assert!(result.is_none());

        let help = handle_ai_command(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &crate::config::AiEnvConfig::default(),
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "",
            "!ai",
            false,
            false,
            None,
        )
        .await;
        assert!(help.is_some());
    }
}
