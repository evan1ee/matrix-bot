use sqlx::SqlitePool;
use tracing::error;

use super::alert::clear_ai_error_state;
use super::client::ResolvedAiConfig;
use super::repository::AiRepository;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::utils::html::html_escape;

pub fn format_error(msg: &str) -> String {
    format!("⚠️ <b>AI Assistant Error:</b> {}", html_escape(msg))
}

pub async fn handle_show_config(pool: &SqlitePool, owner_id: &str) -> String {
    let user_config = AiRepository::get_user_config(pool, owner_id)
        .await
        .unwrap_or(None);
    let resolved = ResolvedAiConfig::from_user_config(user_config.as_ref());
    let settings =
        crate::modules::settings::repository::SettingsRepository::get_or_default(pool, owner_id)
            .await;

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
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    turns_str: &str,
) -> String {
    let turns: i64 = match turns_str.parse() {
        Ok(n) if (1..=100).contains(&n) => n,
        _ => return format_error("Max history turns must be an integer between 1 and 100."),
    };

    let mut settings =
        crate::modules::settings::repository::SettingsRepository::get_or_default(pool, owner_id)
            .await;
    settings.max_history_turns = turns;

    match crate::modules::settings::repository::SettingsRepository::upsert(
        pool, owner_id, &settings,
    )
    .await
    {
        Ok(saved) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::SettingsUpdated {
                    owner_id: owner_id.to_string(),
                    settings: saved.clone(),
                });
            }
            format!(
                "✅ AI max history turns updated to: <code>{}</code>",
                saved.max_history_turns
            )
        }
        Err(e) => {
            error!("Failed to update max history turns: {}", e);
            format_error("Failed to update max history turns setting.")
        }
    }
}

pub async fn handle_set_model(
    pool: &SqlitePool,
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
