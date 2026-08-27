use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use super::client::{AiClient, ResolvedAiConfig};
use super::models::{ChatCompletionMessage, ChatCompletionRequest};
use super::repository::AiRepository;
use super::tools::ToolRegistry;
use crate::events::EventBroadcaster;
use crate::events::SystemEvent;
use crate::utils::html::html_escape;

pub const MAX_HISTORY_TURNS: i64 = 15;
pub const SESSION_TTL_SECONDS: i64 = 1800; // 30 minutes
pub const MAX_PROMPT_LENGTH: usize = 4000;

#[derive(Debug, Clone)]
pub struct AiTurnResult {
    pub conversation_id: String,
    pub message: String,
    pub model_used: String,
    pub tool_calls_executed: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_ai_command(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    room_id: &str,
    owner_id: &str,
    _sender_name: &str,
    command: &str,
    raw_args: &str,
    _full_body: &str,
    _is_direct: bool,
) -> Option<String> {
    let cmd_lower = command.to_lowercase();
    if cmd_lower == "ai" || cmd_lower == "ask" {
        let trimmed = raw_args.trim();
        if trimmed.is_empty() {
            return Some(format_help());
        }
        if trimmed.eq_ignore_ascii_case("reset") || trimmed.eq_ignore_ascii_case("clear") {
            return Some(handle_reset(pool, broadcaster, owner_id, room_id).await);
        }
        if trimmed.eq_ignore_ascii_case("config") {
            return Some(handle_show_config(pool, owner_id).await);
        }
        if let Some(rest) = trimmed.strip_prefix("model") {
            let model_arg = rest.trim();
            if model_arg.is_empty() {
                return Some(handle_show_config(pool, owner_id).await);
            }
            if model_arg.len() > 500 {
                return Some(format_error("Model name exceeds maximum length of 500 characters."));
            }
            return Some(handle_set_model(pool, broadcaster, owner_id, model_arg).await);
        }

        if trimmed.len() > MAX_PROMPT_LENGTH {
            return Some(format_error(&format!(
                "Prompt exceeds maximum length of {} characters.",
                MAX_PROMPT_LENGTH
            )));
        }

        match run_ai_turn(pool, broadcaster, owner_id, room_id, None, trimmed).await {
            Ok(result) => Some(format_response(&result.message, &result.model_used)),
            Err(err) => {
                error!("AI execution failed: {}", err);
                Some(format_error(
                    "Unable to reach the AI model provider. Please verify your AI settings or endpoint.",
                ))
            }
        }
    } else {
        None
    }
}

pub async fn run_ai_turn(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    conversation_id_override: Option<&str>,
    prompt: &str,
) -> Result<AiTurnResult, String> {
    let prompt_trimmed = prompt.trim();
    if prompt_trimmed.is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }
    if prompt_trimmed.len() > MAX_PROMPT_LENGTH {
        return Err(format!(
            "Prompt exceeds maximum length of {} characters",
            MAX_PROMPT_LENGTH
        ));
    }

    info!(
        "AI turn started for owner {} in room {} (prompt length: {})",
        owner_id,
        room_id,
        prompt_trimmed.len()
    );

    let user_config = AiRepository::get_user_config(pool, owner_id)
        .await
        .map_err(|e| e.to_string())?;
    let resolved_config = ResolvedAiConfig::from_user_config(user_config.as_ref());

    let conversation = if let Some(conv_id) = conversation_id_override {
        AiRepository::get_conversation(pool, conv_id, owner_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Conversation not found".to_string())?
    } else {
        AiRepository::get_or_create_conversation(pool, owner_id, room_id, SESSION_TTL_SECONDS)
            .await
            .map_err(|e| e.to_string())?
    };

    // Record user message in SQLite
    if let Err(e) = AiRepository::add_message(pool, &conversation.id, "user", prompt_trimmed, None, None).await {
        warn!("Failed to persist user message: {}", e);
    }

    // Load recent messages for context
    let recent_messages = AiRepository::get_recent_messages(pool, &conversation.id, MAX_HISTORY_TURNS)
        .await
        .unwrap_or_default();

    let mut chat_messages = Vec::new();

    let sys_prompt = resolved_config.system_prompt.clone().unwrap_or_else(|| {
        format!(
            "You are an intelligent, helpful AI assistant integrated into Matrix. \
             You have read-only tools to check the user's tasks, reminders, recurring bills, and financial ledger. \
             The current user's ID is '{}'. Keep answers concise, clear, and nicely formatted with Markdown.",
            owner_id
        )
    });

    chat_messages.push(ChatCompletionMessage {
        role: "system".to_string(),
        content: Some(sys_prompt),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    for msg in recent_messages {
        chat_messages.push(ChatCompletionMessage {
            role: msg.role,
            content: Some(msg.content),
            tool_calls: None,
            tool_call_id: msg.tool_call_id,
            name: msg.tool_name,
        });
    }

    let client = AiClient::new();
    let tools = ToolRegistry::get_tool_definitions();

    let mut iteration = 0;
    let max_iterations = 3;
    let mut final_content = String::new();
    let mut tool_calls_executed = Vec::new();

    while iteration < max_iterations {
        iteration += 1;
        let request = ChatCompletionRequest {
            model: resolved_config.model_name.clone(),
            messages: chat_messages.clone(),
            temperature: Some(resolved_config.temperature),
            max_tokens: Some(resolved_config.max_tokens),
            tools: Some(tools.clone()),
            tool_choice: None,
            stream: Some(false),
        };

        match client.chat_completion(&resolved_config, &request).await {
            Ok(response) => {
                if let Some(choice) = response.choices.into_iter().next() {
                    let msg = choice.message;
                    if let Some(tool_calls) = msg.tool_calls.filter(|t| !t.is_empty()) {
                        // Persist assistant tool-call message in SQLite
                        let tool_desc = msg.content.as_deref().unwrap_or("");
                        let _ = AiRepository::add_message(
                            pool,
                            &conversation.id,
                            "assistant",
                            tool_desc,
                            None,
                            None,
                        )
                        .await;

                        chat_messages.push(ChatCompletionMessage {
                            role: "assistant".to_string(),
                            content: msg.content.clone(),
                            tool_calls: Some(tool_calls.clone()),
                            tool_call_id: None,
                            name: None,
                        });

                        for tool_call in tool_calls {
                            let func_name = tool_call.function.name.clone();
                            let tool_call_id = tool_call.id.clone();
                            tool_calls_executed.push(func_name.clone());
                            debug!("Executing AI tool: {} (id: {})", func_name, tool_call_id);

                            let result_str = match ToolRegistry::execute_tool(
                                pool,
                                owner_id,
                                &func_name,
                                &tool_call.function.arguments,
                            )
                            .await
                            {
                                Ok(res) => res,
                                Err(err) => format!("Error executing tool: {}", err),
                            };

                            // Persist tool result message in SQLite
                            let _ = AiRepository::add_message(
                                pool,
                                &conversation.id,
                                "tool",
                                &result_str,
                                Some(&tool_call_id),
                                Some(&func_name),
                            )
                            .await;

                            chat_messages.push(ChatCompletionMessage {
                                role: "tool".to_string(),
                                content: Some(result_str),
                                tool_calls: None,
                                tool_call_id: Some(tool_call_id),
                                name: Some(func_name),
                            });
                        }
                        continue;
                    }

                    if let Some(text) = msg.content {
                        final_content = text;
                        break;
                    }
                }
                final_content = "Received empty response from AI model.".to_string();
                break;
            }
            Err(err) => {
                error!("AI completion failed: {}", err);
                return Err(err.to_string());
            }
        }
    }

    if let Err(e) = AiRepository::add_message(pool, &conversation.id, "assistant", &final_content, None, None).await {
        warn!("Failed to persist assistant message: {}", e);
    }

    if let Some(b) = broadcaster {
        b.send(SystemEvent::AiConversationUpdated {
            id: conversation.id.clone(),
            owner_id: owner_id.to_string(),
        });
    }

    Ok(AiTurnResult {
        conversation_id: conversation.id,
        message: final_content,
        model_used: resolved_config.model_name,
        tool_calls_executed,
    })
}

async fn handle_reset(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
) -> String {
    if let Ok(conv) = AiRepository::get_or_create_conversation(pool, owner_id, room_id, SESSION_TTL_SECONDS).await {
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

async fn handle_show_config(pool: &SqlitePool, owner_id: &str) -> String {
    let user_config = AiRepository::get_user_config(pool, owner_id).await.unwrap_or(None);
    let resolved = ResolvedAiConfig::from_user_config(user_config.as_ref());

    let key_status = if resolved.api_key.is_some() { "Configured (Hidden)" } else { "None" };

    format!(
        "🤖 <b>AI Assistant Configuration:</b><br/>\
         • <b>Model:</b> <code>{}</code><br/>\
         • <b>Endpoint:</b> <code>{}</code><br/>\
         • <b>API Key:</b> {}<br/>\
         • <b>Temperature:</b> <code>{}</code>",
        html_escape(&resolved.model_name),
        html_escape(&resolved.api_base_url),
        key_status,
        resolved.temperature
    )
}

async fn handle_set_model(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    model_name: &str,
) -> String {
    let input = crate::modules::ai::models::UpdateAiConfigInput {
        api_base_url: None,
        api_key: None,
        model_name: Some(model_name.to_string()),
        system_prompt: None,
        temperature: None,
        max_tokens: None,
    };

    match AiRepository::upsert_user_config(pool, owner_id, &input).await {
        Ok(cfg) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::AiConfigUpdated {
                    owner_id: owner_id.to_string(),
                });
            }
            let model = cfg.model_name.unwrap_or_else(|| model_name.to_string());
            format!("✅ AI model updated to: <code>{}</code>", html_escape(&model))
        }
        Err(e) => {
            error!("Failed to update AI model: {}", e);
            format_error("Failed to update AI model configuration.")
        }
    }
}

fn format_response(markdown_text: &str, model_name: &str) -> String {
    let html_rendered = crate::modules::webhook::renderer::markdown_to_html(markdown_text);
    format!(
        "{}<br/><font color=\"gray\" size=\"-2\">🤖 <em>{}</em></font>",
        html_rendered,
        html_escape(model_name)
    )
}

fn format_error(msg: &str) -> String {
    format!("⚠️ <b>AI Assistant Error:</b> {}", html_escape(msg))
}

fn format_help() -> String {
    "🤖 <b>AI Assistant Commands:</b><br/>\
     • <code>!ai &lt;prompt&gt;</code> — Chat with AI assistant<br/>\
     • <code>!ai reset</code> — Clear conversational memory in this room<br/>\
     • <code>!ai model &lt;name&gt;</code> — Switch active model<br/>\
     • <code>!ai config</code> — View active AI endpoint & model"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup_test() -> (Database, String) {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();
        crate::db::schema::run_migrations(&db.pool).await.unwrap();
        let owner = crate::accounts::AccountRepository::resolve_or_provision(&db.pool, "@user:mbm.one")
            .await
            .unwrap();
        (db, owner)
    }

    #[tokio::test]
    async fn test_ai_help_and_config_commands() {
        let (db, owner) = setup_test().await;

        let help = handle_ai_command(
            &db.pool,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "",
            "",
            false,
        )
        .await
        .unwrap();
        assert!(help.contains("AI Assistant Commands"));

        let config = handle_ai_command(
            &db.pool,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "config",
            "",
            false,
        )
        .await
        .unwrap();
        assert!(config.contains("AI Assistant Configuration"));
    }

    #[tokio::test]
    async fn test_ai_model_switch_and_reset_commands() {
        let (db, owner) = setup_test().await;

        let res = handle_ai_command(
            &db.pool,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "model gpt-4o-mini",
            "",
            false,
        )
        .await
        .unwrap();
        assert!(res.contains("AI model updated to: <code>gpt-4o-mini</code>"));

        let reset = handle_ai_command(
            &db.pool,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "reset",
            "",
            false,
        )
        .await
        .unwrap();
        assert!(reset.contains("conversation history reset"));
    }

    #[tokio::test]
    #[ignore] // Live integration test with real xAI API
    async fn test_live_xai_grok_tool_calling() {
        if std::env::var("https_proxy").is_err() {
            std::env::set_var("https_proxy", "http://127.0.0.1:7897");
            std::env::set_var("http_proxy", "http://127.0.0.1:7897");
        }

        let (db, owner) = setup_test().await;

        crate::modules::todo::repository::TodoRepository::add(
            &db.pool,
            "!room:mbm.one",
            &owner,
            "Tester",
            "Deploy Matrix Bot to staging",
            crate::modules::todo::models::TodoPriority::High,
            None,
            None,
        )
        .await
        .unwrap();

        crate::modules::todo::repository::TodoRepository::add(
            &db.pool,
            "!room:mbm.one",
            &owner,
            "Tester",
            "Audit cryptographic recovery keys",
            crate::modules::todo::models::TodoPriority::Medium,
            None,
            None,
        )
        .await
        .unwrap();

        let api_key = match std::env::var("XAI_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => return, // Skip test if no live key is provided in env
        };

        let input = crate::modules::ai::models::UpdateAiConfigInput {
            api_base_url: Some("https://api.x.ai/v1".to_string()),
            api_key: Some(api_key),
            model_name: Some("grok-4.6".to_string()),
            system_prompt: None,
            temperature: Some(0.7),
            max_tokens: Some(1024),
        };

        AiRepository::upsert_user_config(&db.pool, &owner, &input)
            .await
            .unwrap();

        let result = run_ai_turn(
            &db.pool,
            None,
            &owner,
            "!room:mbm.one",
            None,
            "What tasks do I have pending in my todo list?",
        )
        .await
        .expect("Live xAI Grok turn should succeed");

        println!("\n=== Live xAI Grok Result ===");
        println!("Model Used: {}", result.model_used);
        println!("Tools Executed: {:?}", result.tool_calls_executed);
        println!("Response:\n{}", result.message);

        assert!(!result.tool_calls_executed.is_empty(), "Tool should have been executed");
        assert!(result.tool_calls_executed.contains(&"get_my_tasks".to_string()));
        assert!(
            result.message.contains("Deploy Matrix Bot") || result.message.contains("Audit cryptographic"),
            "Response should mention tasks"
        );
    }
}
