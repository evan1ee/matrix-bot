use sqlx::SqlitePool;
use std::sync::Arc;
#[cfg(not(test))]
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub use super::alert::{clear_ai_error_state, is_ai_error_alerted, notify_dev_room_ai_error};
use super::client::{AiClient, ResolvedAiConfig};
pub use super::config_cmd::{
    format_error, handle_set_history, handle_set_model, handle_show_config,
};
use super::models::{ChatCompletionMessage, ChatCompletionRequest};
use super::repository::AiRepository;
use super::tools::ToolRegistry;
use crate::events::EventBroadcaster;
use crate::events::SystemEvent;
use crate::utils::html::html_escape;

#[cfg(not(test))]
static AI_CLIENT: OnceLock<AiClient> = OnceLock::new();

#[cfg(not(test))]
pub fn get_ai_client() -> AiClient {
    AI_CLIENT.get_or_init(AiClient::new).clone()
}

#[cfg(test)]
pub fn get_ai_client() -> AiClient {
    AiClient::new()
}

pub const MAX_HISTORY_TURNS: i64 = 10;
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
    client_handle: Option<&Arc<RwLock<Option<matrix_sdk::Client>>>>,
    room_id: &str,
    owner_id: &str,
    _sender_name: &str,
    command: &str,
    raw_args: &str,
    full_body: &str,
    is_direct: bool,
    is_mentioned: bool,
    quoted: Option<&crate::bot::quote::QuotedMessage>,
) -> Option<String> {
    let cmd_lower = command.to_lowercase();
    let prompt = if cmd_lower == "ai" || cmd_lower == "ask" {
        let trimmed = raw_args.trim();
        if trimmed.is_empty() && quoted.is_none() {
            return Some(format_help());
        }
        if trimmed.eq_ignore_ascii_case("retro")
            || trimmed.eq_ignore_ascii_case("retrospective")
            || trimmed.eq_ignore_ascii_case("weekly")
        {
            return Some(super::retrospective::handle_retrospective_command(pool, owner_id).await);
        }
        if trimmed.eq_ignore_ascii_case("reset") || trimmed.eq_ignore_ascii_case("clear") {
            return Some(handle_reset(pool, broadcaster, owner_id, room_id).await);
        }
        if trimmed.eq_ignore_ascii_case("config") {
            return Some(handle_show_config(pool, owner_id).await);
        }
        if let Some(rest) = trimmed.strip_prefix("model") {
            let model_arg = rest.trim();
            if model_arg.is_empty() { return Some(handle_show_config(pool, owner_id).await); }
            if model_arg.len() > 500 { return Some(format_error("Model name exceeds maximum length of 500 characters.")); }
            return Some(handle_set_model(pool, broadcaster, owner_id, model_arg).await);
        }
        if let Some(rest) = trimmed.strip_prefix("history").or_else(|| trimmed.strip_prefix("turns")) {
            let num_arg = rest.trim();
            if num_arg.is_empty() { return Some(handle_show_config(pool, owner_id).await); }
            return Some(handle_set_history(pool, broadcaster, owner_id, num_arg).await);
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

    match run_ai_turn(
        pool,
        broadcaster,
        owner_id,
        room_id,
        None,
        &effective_prompt,
    )
    .await
    {
        Ok(result) => {
            clear_ai_error_state();
            Some(format_response(&result.message, &result.model_used))
        }
        Err(err) => {
            error!("AI execution failed: {}", err);
            notify_dev_room_ai_error(pool, client_handle, owner_id, &err).await;
            None
        }
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

    // Record user message in SQLite if not an exact immediate retry of the latest message
    let is_immediate_duplicate =
        match AiRepository::get_recent_messages(pool, &conversation.id, 1).await {
            Ok(msgs) => msgs
                .last()
                .map(|m| m.role == "user" && m.content == prompt_trimmed)
                .unwrap_or(false),
            Err(_) => false,
        };

    if !is_immediate_duplicate {
        if let Err(e) =
            AiRepository::add_message(pool, &conversation.id, "user", prompt_trimmed, None, None)
                .await
        {
            warn!("Failed to persist user message: {}", e);
        }
    }

    let settings =
        crate::modules::settings::repository::SettingsRepository::get_or_default(pool, owner_id)
            .await;
    let max_history = settings.max_history_turns.clamp(1, 100);

    // Load recent messages for context
    let recent_messages = AiRepository::get_recent_messages(pool, &conversation.id, max_history)
        .await
        .unwrap_or_default();

    let mut chat_messages = Vec::new();

    let sys_prompt = super::prompt::build_system_prompt(pool, &resolved_config, owner_id, room_id).await;

    chat_messages.push(ChatCompletionMessage {
        role: "system".to_string(),
        content: Some(sys_prompt),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    for msg in recent_messages {
        // Skip tool messages and intermediate empty assistant messages from past turns.
        // The OpenAI/xAI API requires every message with role 'tool' to immediately follow an
        // assistant message with 'tool_calls'. Stripping intermediate tool rounds preserves
        // a clean, valid conversation history of user prompts and final assistant replies.
        if msg.role == "tool" || (msg.role == "assistant" && msg.content.trim().is_empty()) {
            continue;
        }
        chat_messages.push(ChatCompletionMessage {
            role: msg.role,
            content: Some(msg.content),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }

    let client = get_ai_client();
    let mut tool_values: Vec<serde_json::Value> = ToolRegistry::get_tool_definitions()
        .into_iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    if super::search::should_enable_web_search(&resolved_config, prompt_trimmed, None) {
        info!("Enabling web_search tool for prompt: {}", prompt_trimmed);
        tool_values.push(super::search::web_search_tool_definition());
    }

    let mut iteration = 0;
    let max_iterations = 5;
    let mut final_content = String::new();
    let mut tool_calls_executed = Vec::new();

    while iteration < max_iterations {
        iteration += 1;
        let request = ChatCompletionRequest {
            model: resolved_config.model_name.clone(),
            messages: chat_messages.clone(),
            temperature: Some(resolved_config.temperature),
            max_tokens: Some(resolved_config.max_tokens),
            tools: Some(tool_values.clone()),
            tool_choice: None,
            stream: Some(false),
        };

        match client.chat_completion(&resolved_config, &request).await {
            Ok(response) => {
                if let Some(choice) = response.choices.into_iter().next() {
                    let msg = choice.message;
                    if let Some(tool_calls) = msg.tool_calls.filter(|t| !t.is_empty()) {
                        // Persist assistant tool-call message in SQLite only if it contains non-empty text
                        let tool_desc = msg.content.as_deref().unwrap_or("").trim();
                        if !tool_desc.is_empty() {
                            let _ = AiRepository::add_message(
                                pool,
                                &conversation.id,
                                "assistant",
                                tool_desc,
                                None,
                                None,
                            )
                            .await;
                        }

                        chat_messages.push(ChatCompletionMessage {
                            role: "assistant".to_string(),
                            content: msg.content.clone(),
                            tool_calls: Some(tool_calls.clone()),
                            tool_call_id: None,
                            name: None,
                        });

                        let tool_futures = tool_calls.iter().map(|tool_call| {
                            let func_name = tool_call.function.name.clone();
                            let tool_call_id = tool_call.id.clone();
                            let args = tool_call.function.arguments.clone();
                            async move {
                                info!(
                                    "AI turn [{}/{}] executing tool: {} for owner {}",
                                    iteration, max_iterations, func_name, owner_id
                                );
                                let result_str = match ToolRegistry::execute_tool(
                                    pool,
                                    broadcaster,
                                    owner_id,
                                    room_id,
                                    &func_name,
                                    &args,
                                )
                                .await
                                {
                                    Ok(res) => res,
                                    Err(err) => format!("Error executing tool: {}", err),
                                };
                                (tool_call_id, func_name, result_str)
                            }
                        });

                        let execution_results = futures_util::future::join_all(tool_futures).await;

                        for (tool_call_id, func_name, result_str) in execution_results {
                            tool_calls_executed.push(func_name.clone());

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
                        let is_hallucinated_tool = tool_calls_executed.is_empty()
                            && iteration == 1
                            && (text.contains("record_finance_entry")
                                || text.contains("add_expense")
                                || text.contains("add_todo")
                                || text.contains("create_reminder")
                                || text.contains("Tool called:")
                                || text.contains("Receipt recorded")
                                || text.contains("Receipt logged"));

                        if is_hallucinated_tool {
                            warn!("AI simulated tool call in text without invoking function API; correcting model in next turn");
                            chat_messages.push(ChatCompletionMessage {
                                role: "assistant".to_string(),
                                content: Some(text),
                                tool_calls: None,
                                tool_call_id: None,
                                name: None,
                            });
                            chat_messages.push(ChatCompletionMessage {
                                role: "user".to_string(),
                                content: Some(
                                    "SYSTEM: You wrote text describing or claiming that a tool was called, but you did NOT actually invoke any tool function via the function calling API! No records were saved to the database. You MUST execute the real tool call (such as `record_finance_entry` or `add_todo`) now via the function calling API so it can be saved.".to_string(),
                                ),
                                tool_calls: None,
                                tool_call_id: None,
                                name: None,
                            });
                            continue;
                        }

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

    if final_content.trim().is_empty() && !tool_calls_executed.is_empty() {
        let final_req = ChatCompletionRequest {
            model: resolved_config.model_name.clone(),
            messages: chat_messages.clone(),
            temperature: Some(resolved_config.temperature),
            max_tokens: Some(resolved_config.max_tokens),
            tools: None,
            tool_choice: None,
            stream: Some(false),
        };
        if let Ok(resp) = client.chat_completion(&resolved_config, &final_req).await {
            if let Some(t) = resp.choices.into_iter().next().and_then(|c| c.message.content) {
                if !t.trim().is_empty() {
                    final_content = t;
                }
            }
        }
    }

    if final_content.trim().is_empty() {
        if !tool_calls_executed.is_empty() {
            final_content = format!("Completed actions: {}", tool_calls_executed.join(", "));
        } else {
            final_content = "Received empty response from AI model.".to_string();
        }
    }

    if tool_calls_executed.is_empty()
        && (final_content.contains("Receipt recorded")
            || final_content.contains("Receipt logged")
            || final_content.contains("Tool called:"))
    {
        final_content = "⚠️ **Action Not Recorded**: The AI assistant generated a text summary but did not execute the database tool. Please retry sending the image or use the `!ai` command to log the expense.".to_string();
    }

    if let Err(e) = AiRepository::add_message(
        pool,
        &conversation.id,
        "assistant",
        &final_content,
        None,
        None,
    )
    .await
    {
        warn!("Failed to persist assistant message: {}", e);
    }

    if let Some(b) = broadcaster {
        b.send(SystemEvent::AiConversationUpdated {
            id: conversation.id.clone(),
            owner_id: owner_id.to_string(),
        });
    }

    info!(
        "AI turn completed for owner {} in room {} (tools: {}, reply chars: {})",
        owner_id,
        room_id,
        tool_calls_executed.len(),
        final_content.len()
    );

    Ok(AiTurnResult {
        conversation_id: conversation.id,
        message: final_content,
        model_used: resolved_config.model_name,
        tool_calls_executed,
    })
}

async fn handle_reset(pool: &SqlitePool, broadcaster: Option<&EventBroadcaster>, owner_id: &str, room_id: &str) -> String {
    if let Ok(conv) = AiRepository::get_or_create_conversation(pool, owner_id, room_id, SESSION_TTL_SECONDS).await {
        if let Ok(true) = AiRepository::delete_conversation(pool, &conv.id, owner_id).await {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::AiConversationDeleted { id: conv.id, owner_id: owner_id.to_string() });
            }
        }
    }
    "🧹 <b>AI conversation history reset.</b> Next message starts a fresh turn.".to_string()
}

fn format_response(markdown_text: &str, model_name: &str) -> String {
    let text = if markdown_text.trim().is_empty() { "Received empty response from AI model." } else { markdown_text };
    let html = crate::modules::webhook::renderer::markdown_to_html(text);
    format!("{}<br/><font color=\"gray\" size=\"-2\">🤖 <em>{}</em></font>", html, html_escape(model_name))
}

fn format_help() -> String {
    "🤖 <b>AI Assistant Commands:</b><br/>\
     • <code>!ai &lt;prompt&gt;</code> — Chat with AI assistant (supports multi-action capture)<br/>\
     • <code>!ai retro</code> — Generate weekly executive retrospective<br/>\
     • <code>!ai reset</code> — Clear conversational memory in this room<br/>\
     • <code>!ai model &lt;name&gt;</code> — Switch active model<br/>\
     • <code>!ai history &lt;1-100&gt;</code> — Set max context history messages<br/>\
     • <code>!ai config</code> — View active AI endpoint & model".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup_test() -> (Database, String) {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();
        crate::db::schema::run_migrations(&db.pool).await.unwrap();
        let owner =
            crate::accounts::AccountRepository::resolve_or_provision(&db.pool, "@user:mbm.one")
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

        let config = handle_ai_command(
            &db.pool,
            None,
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
        assert!(config.contains("AI Assistant Configuration"));
        assert!(config.contains("Max History Turns"));
    }

    #[tokio::test]
    async fn test_ai_model_switch_and_reset_commands() {
        let (db, owner) = setup_test().await;

        let res = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "model gpt-4o-mini",
            "",
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(res.contains("AI model updated to: <code>gpt-4o-mini</code>"));

        let hist = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "ai",
            "history 20",
            "",
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(hist.contains("AI max history turns updated to: <code>20</code>"));

        let config2 = handle_ai_command(
            &db.pool,
            None,
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
        assert!(config2.contains("Max History Turns:</b> <code>20</code>"));

        let reset = handle_ai_command(
            &db.pool,
            None,
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
        assert!(reset.contains("conversation history reset"));
    }

    #[tokio::test]
    async fn test_ai_error_suppressed_from_chat_and_sent_once() {
        let (db, owner) = setup_test().await;
        clear_ai_error_state();
        assert!(!is_ai_error_alerted());

        // Configure unreachable endpoint to guarantee failure regardless of live environment
        let input = crate::modules::ai::models::UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1:1".to_string()),
            api_key: Some("invalid-key".to_string()),
            model_name: Some("test-model".to_string()),
            ..Default::default()
        };
        AiRepository::upsert_user_config(&db.pool, &owner, &input)
            .await
            .unwrap();

        let result1 = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:general",
            &owner,
            "User",
            "ai",
            "hello world",
            "",
            false,
            false,
            None,
        )
        .await;

        // Must return None so error is never posted to general chat room
        assert!(result1.is_none());
        assert!(
            is_ai_error_alerted(),
            "Error state should be marked as alerted"
        );

        // Second failure during same outage
        let result2 = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:general",
            &owner,
            "User",
            "ai",
            "second prompt",
            "",
            false,
            false,
            None,
        )
        .await;
        assert!(result2.is_none());

        // Reset clears it
        clear_ai_error_state();
        assert!(!is_ai_error_alerted());
    }

    #[tokio::test]
    async fn test_multiturn_history_filters_tool_messages() {
        let (db, owner) = setup_test().await;
        let conv = AiRepository::get_or_create_conversation(&db.pool, &owner, "!room:1", 1800)
            .await
            .unwrap();

        // 1. User message
        AiRepository::add_message(
            &db.pool,
            &conv.id,
            "user",
            "Add a task: buy milk",
            None,
            None,
        )
        .await
        .unwrap();
        // 2. Assistant tool call with empty content
        AiRepository::add_message(&db.pool, &conv.id, "assistant", "", None, None)
            .await
            .unwrap();
        // 3. Tool result
        AiRepository::add_message(
            &db.pool,
            &conv.id,
            "tool",
            "{\"id\":1,\"status\":\"ok\"}",
            Some("call_1"),
            Some("add_todo"),
        )
        .await
        .unwrap();
        // 4. Assistant final reply
        AiRepository::add_message(
            &db.pool,
            &conv.id,
            "assistant",
            "I added your task: buy milk.",
            None,
            None,
        )
        .await
        .unwrap();

        let recent = AiRepository::get_recent_messages(&db.pool, &conv.id, 10)
            .await
            .unwrap();
        assert_eq!(recent.len(), 3);

        let mut filtered: Vec<ChatCompletionMessage> = Vec::new();
        for msg in recent {
            if msg.role == "tool" || (msg.role == "assistant" && msg.content.trim().is_empty()) {
                continue;
            }
            filtered.push(ChatCompletionMessage {
                role: msg.role,
                content: Some(msg.content),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].role, "user");
        assert_eq!(filtered[0].content.as_deref(), Some("Add a task: buy milk"));
        assert_eq!(filtered[1].role, "assistant");
        assert_eq!(
            filtered[1].content.as_deref(),
            Some("I added your task: buy milk.")
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_grok_tool_calling() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::dotenv();

        let config = ResolvedAiConfig::from_user_config(None);
        if config.api_key.is_none() {
            return; // Skip test if no live key is provided in env
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

        // Test with pure environment config (no user DB override)
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

        println!("\n=== Live xAI Grok Tool Calling Result ===");
        println!("Model Used: {}", result.model_used);
        println!("Tools Executed: {:?}", result.tool_calls_executed);
        println!("Response:\n{}", result.message);

        assert!(
            !result.tool_calls_executed.is_empty(),
            "Tool should have been executed"
        );
        assert!(result
            .tool_calls_executed
            .contains(&"get_my_tasks".to_string()));
        assert!(
            result.message.contains("Deploy Matrix Bot")
                || result.message.contains("Audit cryptographic"),
            "Response should mention tasks"
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_grok_mutation_tools() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::dotenv();

        let config = ResolvedAiConfig::from_user_config(None);
        if config.api_key.is_none() {
            return;
        }

        let (db, owner) = setup_test().await;

        // 1. Test Natural Language Task Creation via Environment Config
        let task_result = run_ai_turn(
            &db.pool,
            None,
            &owner,
            "!room:mbm.one",
            None,
            "Please add a high priority todo for me: Prepare slide deck for Friday demo",
        )
        .await
        .expect("Natural language add_todo should succeed");

        println!("\n=== Live xAI Grok Add Todo Result ===");
        println!("Tools Executed: {:?}", task_result.tool_calls_executed);
        println!("Response:\n{}", task_result.message);

        assert!(task_result
            .tool_calls_executed
            .contains(&"add_todo".to_string()));

        // Verify task in SQLite DB
        let tasks = crate::modules::todo::repository::TodoRepository::list_all_user(
            &db.pool, &owner, None, 10, 0,
        )
        .await
        .unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].text.contains("Prepare slide deck"));
        assert_eq!(tasks[0].priority, "high");

        // 2. Test Natural Language Expense Recording
        let exp_result = run_ai_turn(
            &db.pool,
            None,
            &owner,
            "!room:mbm.one",
            None,
            "Record an expense of 18.50 USD for team lunch in the Food category",
        )
        .await
        .expect("Natural language record_finance_entry should succeed");

        println!("\n=== Live xAI Grok Record Expense Result ===");
        println!("Tools Executed: {:?}", exp_result.tool_calls_executed);
        println!("Response:\n{}", exp_result.message);

        assert!(exp_result
            .tool_calls_executed
            .contains(&"record_finance_entry".to_string()));

        // Verify transaction in SQLite DB
        let from = chrono::Utc::now() - chrono::Duration::days(1);
        let to = chrono::Utc::now() + chrono::Duration::days(1);
        let summary = crate::modules::finance::repository::FinanceRepository::summary(
            &db.pool, &owner, from, to,
        )
        .await
        .unwrap();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].expense_minor, 1850);
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_grok_multi_turn_and_context() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::dotenv();

        let config = ResolvedAiConfig::from_user_config(None);
        if config.api_key.is_none() {
            return;
        }

        let (db, owner) = setup_test().await;

        // Turn 1: Add a task
        let turn1 = run_ai_turn(
            &db.pool,
            None,
            &owner,
            "!room:mbm.one",
            None,
            "I need to finish the quarterly budget review by tomorrow evening. Can you add it as a todo with high priority?",
        )
        .await
        .expect("Turn 1 should succeed");

        println!("\n=== Multi-turn Turn 1 ===");
        println!("Tools: {:?}", turn1.tool_calls_executed);
        println!("Response:\n{}", turn1.message);
        assert!(turn1.tool_calls_executed.contains(&"add_todo".to_string()));

        // Turn 2: Follow-up referencing Turn 1 in the same room
        let turn2 = run_ai_turn(
            &db.pool,
            None,
            &owner,
            "!room:mbm.one",
            None,
            "What was the task I just asked you to add?",
        )
        .await
        .expect("Turn 2 should succeed");

        println!("\n=== Multi-turn Turn 2 ===");
        println!("Tools: {:?}", turn2.tool_calls_executed);
        println!("Response:\n{}", turn2.message);
        let msg_lower = turn2.message.to_lowercase();
        assert!(
            msg_lower.contains("budget") || msg_lower.contains("quarterly"),
            "Turn 2 should recall the budget task from conversation context"
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_grok_currency_rules_and_multi_action() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::dotenv();

        let config = ResolvedAiConfig::from_user_config(None);
        if config.api_key.is_none() {
            return;
        }

        let (db, owner) = setup_test().await;

        let result = run_ai_turn(
            &db.pool,
            None,
            &owner,
            "!room:mbm.one",
            None,
            "Log an expense of 45 CNY for coffee, another expense of 12 USD for lunch, and set a reminder in 45 minutes to submit receipts",
        )
        .await
        .expect("Multi-action turn should succeed");

        println!("\n=== Multi-Action Quick Capture Result ===");
        println!("Tools Executed: {:?}", result.tool_calls_executed);
        println!("Response:\n{}", result.message);

        // Verify tools executed
        assert!(result
            .tool_calls_executed
            .contains(&"record_finance_entry".to_string()));
        assert!(result
            .tool_calls_executed
            .contains(&"set_reminder".to_string()));

        // Verify DB transactions
        let from = chrono::Utc::now() - chrono::Duration::days(1);
        let to = chrono::Utc::now() + chrono::Duration::days(1);
        let summary = crate::modules::finance::repository::FinanceRepository::summary(
            &db.pool, &owner, from, to,
        )
        .await
        .unwrap();
        // Should have 2 separate currencies recorded: CNY and USD
        assert_eq!(
            summary.len(),
            2,
            "Must record separate currencies without blending"
        );
        let currencies: Vec<_> = summary.iter().map(|s| s.currency.as_str()).collect();
        assert!(currencies.contains(&"CNY"));
        assert!(currencies.contains(&"USD"));

        // Verify reminder in DB
        let reminders =
            crate::modules::reminder::repository::ReminderRepository::list_user_pending(
                &db.pool, &owner, 10, 0,
            )
            .await
            .unwrap();
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].message.to_lowercase().contains("receipt"));

        // Verify response never sums different currencies together
        assert!(
            !result.message.contains("57"),
            "Different currencies must never be added together"
        );
    }

    #[tokio::test]
    async fn test_ai_mention_natural_language_routing() {
        let (db, owner) = setup_test().await;

        clear_ai_error_state();
        assert!(!is_ai_error_alerted());

        // When neither mentioned nor direct, unrecognized command returns None immediately
        let unhandled = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "明天早上8点提醒我写邮件",
            "",
            "明天早上8点提醒我写邮件",
            false,
            false,
            None,
        )
        .await;
        assert!(unhandled.is_none());
        assert!(!is_ai_error_alerted());

        // Configure invalid endpoint so run_ai_turn triggers error alert if called
        let input = crate::modules::ai::models::UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1:1".to_string()),
            api_key: Some("invalid-key".to_string()),
            model_name: Some("test-model".to_string()),
            ..Default::default()
        };
        AiRepository::upsert_user_config(&db.pool, &owner, &input)
            .await
            .unwrap();

        // When mentioned in a room, prompt is routed to AI
        let _ = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:mbm.one",
            &owner,
            "User",
            "明天早上8点提醒我写邮件",
            "",
            "明天早上8点提醒我写邮件",
            false,
            true, // is_mentioned = true
            None,
        )
        .await;

        assert!(
            is_ai_error_alerted(),
            "Mentioned natural language message should be routed to AI engine"
        );
        clear_ai_error_state();
    }

    #[tokio::test]
    async fn test_ai_quoted_context_prompt_routing() {
        let (db, owner) = setup_test().await;
        clear_ai_error_state();

        let input = crate::modules::ai::models::UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1:1".to_string()),
            api_key: Some("invalid-key".to_string()),
            model_name: Some("test-model".to_string()),
            ..Default::default()
        };
        AiRepository::upsert_user_config(&db.pool, &owner, &input)
            .await
            .unwrap();

        let quote = crate::bot::quote::QuotedMessage {
            author: Some("@bob:mbm.one".to_string()),
            body: "The earth is flat and gravity is an illusion.".to_string(),
            event_id: None,
        };

        // When quoted and mentioned with "fact check this", routes to AI with error trigger on unreachable host
        let _ = handle_ai_command(
            &db.pool,
            None,
            None,
            "!room:mbm.one",
            &owner,
            "Alice",
            "fact",
            "check this",
            "fact check this",
            false,
            true,
            Some(&quote),
        )
        .await;

        assert!(
            is_ai_error_alerted(),
            "Quoted reply should trigger AI processing"
        );
        clear_ai_error_state();
    }
}
