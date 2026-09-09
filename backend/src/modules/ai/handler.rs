use sqlx::SqlitePool;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

pub use super::alert::{clear_ai_error_state, is_ai_error_alerted, notify_dev_room_ai_error};
use super::client::{AiClient, ResolvedAiConfig};
use super::models::{ChatCompletionMessage, ChatCompletionRequest};
use super::repository::AiRepository;
use super::tools::ToolRegistry;
use crate::events::EventBroadcaster;
use crate::events::SystemEvent;
use crate::utils::html::html_escape;

static AI_CLIENT: OnceLock<AiClient> = OnceLock::new();

pub fn get_ai_client() -> &'static AiClient {
    AI_CLIENT.get_or_init(AiClient::new)
}

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
    client_handle: Option<&Arc<RwLock<Option<matrix_sdk::Client>>>>,
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
        if trimmed.eq_ignore_ascii_case("retro") || trimmed.eq_ignore_ascii_case("retrospective") || trimmed.eq_ignore_ascii_case("weekly") {
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
            "You are the central Brain and intelligent Personal Operating System for matrix-bot. \
             You have full bidirectional sensory and action access across all modules: tasks & to-dos, reminders, recurring bills & subscriptions, investment portfolio & cash balances, financial ledger & categories, daily habits & streaks, travel footprints, bookmarks & reading list, inbound webhooks, user settings, and daily logs / lifespan reflections. \
             MULTIPLE ACTIONS & QUICK CAPTURE: When the user requests multiple actions in a single message (e.g. logging an expense, checking off a habit, setting a reminder, adding a task, or logging today's mood), call all relevant tools in sequence or parallel. \
             Provide a consolidated, clear confirmation of every action taken with bullet points. \
             SYNTHESIS & BRAIN CORRELATION: Proactively synthesize context across modules when asked open-ended questions like 'How was my day?', 'What should I focus on today?', or 'Give me a life audit'. \
             CURRENCY RULES: Currencies must NEVER be added or subtracted together without exchange rates. Always report spending, income, and cashflow separately per currency (e.g. 'CNY 45.00, USD 16.50', never 'CNY 61.50'). \
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

                        for tool_call in tool_calls {
                            let func_name = tool_call.function.name.clone();
                            let tool_call_id = tool_call.id.clone();
                            tool_calls_executed.push(func_name.clone());
                            debug!("Executing AI tool: {} (id: {})", func_name, tool_call_id);

                            let result_str = match ToolRegistry::execute_tool(
                                pool,
                                broadcaster,
                                owner_id,
                                room_id,
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
            clear_ai_error_state();
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
     • <code>!ai &lt;prompt&gt;</code> — Chat with AI assistant (supports multi-action capture)<br/>\
     • <code>!ai retro</code> — Generate weekly executive retrospective<br/>\
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
    async fn test_ai_error_suppressed_from_chat_and_sent_once() {
        let (db, owner) = setup_test().await;
        clear_ai_error_state();
        assert!(!is_ai_error_alerted());

        // In test mode without external API server, calling !ai prompt will fail
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
        )
        .await;

        // Must return None so error is never posted to general chat room
        assert!(result1.is_none());
        assert!(is_ai_error_alerted(), "Error state should be marked as alerted");

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
        AiRepository::add_message(&db.pool, &conv.id, "user", "Add a task: buy milk", None, None)
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

    #[tokio::test]
    #[ignore] // Live integration test with real xAI API for natural language mutations
    async fn test_live_xai_grok_mutation_tools() {
        if std::env::var("https_proxy").is_err() {
            std::env::set_var("https_proxy", "http://127.0.0.1:7897");
            std::env::set_var("http_proxy", "http://127.0.0.1:7897");
        }

        let (db, owner) = setup_test().await;

        let api_key = match std::env::var("XAI_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => return,
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

        // 1. Test Natural Language Task Creation
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

        assert!(task_result.tool_calls_executed.contains(&"add_todo".to_string()));

        // Verify task in SQLite DB
        let tasks = crate::modules::todo::repository::TodoRepository::list_all_user(&db.pool, &owner, None, 10, 0)
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

        assert!(exp_result.tool_calls_executed.contains(&"record_finance_entry".to_string()));

        // Verify transaction in SQLite DB
        let from = chrono::Utc::now() - chrono::Duration::days(1);
        let to = chrono::Utc::now() + chrono::Duration::days(1);
        let summary = crate::modules::finance::repository::FinanceRepository::summary(&db.pool, &owner, from, to)
            .await
            .unwrap();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].expense_minor, 1850);
    }
}
