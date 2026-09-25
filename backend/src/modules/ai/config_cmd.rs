use crate::application::settings::SettingsService;
use crate::db::DbPool;
use tracing::error;

use super::alert::clear_ai_error_state;
use super::repository::AiRepository;
use super::resolved_config::ResolvedAiConfig;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::utils::html::html_escape;

pub fn format_error(msg: &str) -> String {
    format!("⚠️ <b>AI Assistant Error:</b> {}", html_escape(msg))
}

pub async fn handle_show_config(
    pool: &DbPool,
    settings: &SettingsService,
    ai: &crate::config::AiEnvConfig,
    owner_id: &str,
) -> String {
    let user_config = AiRepository::get_user_config(pool, owner_id)
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
    owner_id: &str,
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
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    model_name: &str,
) -> String {
    let input = crate::modules::ai::models::UpdateAiConfigInput {
        model_name: Some(model_name.to_string()),
        ..Default::default()
    };

    match AiRepository::upsert_user_config(pool, owner_id, &input).await {
        Ok(cfg) => {
            clear_ai_error_state();
            if let Some(b) = broadcaster {
                b.send(SystemEvent::AiConfigUpdated {
                    owner_id: owner_id.to_string(),
                });
            }
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

pub async fn handle_reset(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
) -> String {
    if let Ok(conv) = AiRepository::get_or_create_conversation(pool, owner_id, room_id, 1800).await
    {
        if let Ok(true) = AiRepository::delete_conversation(pool, &conv.id, owner_id).await {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::AiConversationDeleted {
                    id: conv.id,
                    owner_id: owner_id.to_string(),
                });
            }
        }
    }
    "🧹 <b>AI conversation history reset.</b> Next message starts a fresh turn.".to_string()
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

#[allow(clippy::too_many_arguments)]
pub async fn handle_ai_subcommand(
    pool: &DbPool,
    settings: &SettingsService,
    ai: &crate::config::AiEnvConfig,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    trimmed: &str,
) -> Option<String> {
    if trimmed.eq_ignore_ascii_case("retro")
        || trimmed.eq_ignore_ascii_case("retrospective")
        || trimmed.eq_ignore_ascii_case("weekly")
    {
        return Some(
            super::retrospective::handle_retrospective_command(pool, settings, owner_id).await,
        );
    }
    if trimmed.eq_ignore_ascii_case("reset") || trimmed.eq_ignore_ascii_case("clear") {
        return Some(handle_reset(pool, broadcaster, owner_id, room_id).await);
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
        return Some(handle_set_model(pool, broadcaster, owner_id, model_arg).await);
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
