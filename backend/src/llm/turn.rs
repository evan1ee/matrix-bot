use tracing::{error, info, warn};

use super::context::{AiTurnResult, LlmTurnContext};
use super::error::AiError;
use super::resolved_config::ResolvedAiConfig;
use super::tools::ToolRegistry;
use crate::events::SystemEvent;
use crate::infrastructure::inference::llm::{ChatCompletionMessage, ChatCompletionRequest};
use crate::infrastructure::postgres::repositories::llm as llm_repo;

pub const MAX_TOOL_ROUNDS: usize = 5;
pub const MAX_PROMPT_LENGTH: usize = 16384;
pub const SESSION_TTL_SECONDS: i64 = 3600;

/// Runs a single LLM conversation turn, executing a bounded tool calling loop
/// (up to `MAX_TOOL_ROUNDS` rounds) and streaming results if a reporter is present.
pub async fn run_turn(
    ctx: &LlmTurnContext<'_>,
    prompt: &str,
    conversation_id_override: Option<&str>,
) -> Result<AiTurnResult, AiError> {
    let prompt_trimmed = prompt.trim();
    if prompt_trimmed.is_empty() {
        return Err(AiError::Validation("Prompt cannot be empty".to_string()));
    }
    if prompt_trimmed.len() > MAX_PROMPT_LENGTH {
        return Err(AiError::Validation(format!(
            "Prompt exceeds maximum length of {} characters",
            MAX_PROMPT_LENGTH
        )));
    }

    info!(
        "LLM turn started for owner {} in room {} (prompt length: {})",
        ctx.owner_id,
        ctx.room_id,
        prompt_trimmed.len()
    );

    let user_config = llm_repo::get_user_config(ctx.pool, ctx.owner_id)
        .await
        .map_err(|e| AiError::Db(e.to_string()))?;
    let resolved_config = ResolvedAiConfig::from_user_config(ctx.config, user_config.as_ref());

    let conversation = if let Some(conv_id) = conversation_id_override {
        llm_repo::get_conversation(ctx.pool, conv_id, ctx.owner_id)
            .await
            .map_err(|e| AiError::Db(e.to_string()))?
            .ok_or_else(|| AiError::Validation("Conversation not found".to_string()))?
    } else {
        llm_repo::get_or_create_conversation(
            ctx.pool,
            ctx.owner_id,
            ctx.room_id,
            SESSION_TTL_SECONDS,
        )
        .await
        .map_err(|e| AiError::Db(e.to_string()))?
    };

    // Record user message if not an exact immediate retry of the latest message
    let is_immediate_duplicate =
        match llm_repo::get_recent_messages(ctx.pool, &conversation.id, 1).await {
            Ok(msgs) => msgs
                .last()
                .map(|m| m.role == "user" && m.content == prompt_trimmed)
                .unwrap_or(false),
            Err(_) => false,
        };

    if !is_immediate_duplicate {
        if let Err(e) = llm_repo::add_message(
            ctx.pool,
            &conversation.id,
            "user",
            prompt_trimmed,
            None,
            None,
        )
        .await
        {
            warn!("Failed to persist user message: {}", e);
        }
    }

    // Retrieve conversation history
    let history_limit = ctx
        .services
        .settings
        .get_or_default(ctx.owner_id)
        .await
        .max_history_turns;
    let recent_messages = llm_repo::get_recent_messages(ctx.pool, &conversation.id, history_limit)
        .await
        .unwrap_or_default();

    let mut chat_messages: Vec<ChatCompletionMessage> = Vec::new();

    // 1. Inject system prompt
    let system_prompt = super::prompt::build_system_prompt(
        ctx.services,
        &resolved_config,
        ctx.owner_id,
        ctx.room_id,
    )
    .await;
    chat_messages.push(ChatCompletionMessage {
        role: "system".to_string(),
        content: Some(system_prompt),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    // 2. Inject recent conversation history
    for msg in recent_messages {
        let is_last_user_prompt = msg.role == "user" && msg.content == prompt_trimmed;
        if is_last_user_prompt {
            continue;
        }

        if msg.role == "user" || msg.role == "assistant" {
            chat_messages.push(ChatCompletionMessage {
                role: msg.role,
                content: Some(msg.content),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }
    }

    // 3. Inject current user prompt
    chat_messages.push(ChatCompletionMessage {
        role: "user".to_string(),
        content: Some(prompt_trimmed.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    if let Some(reporter) = ctx.stage_reporter {
        reporter.report_stage("🤔 思考中...").await;
    }

    let client = crate::infrastructure::inference::llm::shared_client(ctx.config.proxy.as_deref());
    let mut tool_values: Vec<serde_json::Value> = ToolRegistry::get_tool_definitions()
        .into_iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    if super::search::should_enable_web_search(&resolved_config, prompt_trimmed, None) {
        info!("Enabling web_search tool for prompt: {}", prompt_trimmed);
        tool_values.push(super::search::web_search_tool_definition());
    }

    let mut iteration = 0;
    let max_iterations = MAX_TOOL_ROUNDS;
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

        match super::stream::execute_stream_or_complete(
            &client,
            &resolved_config,
            &request,
            ctx.stage_reporter,
        )
        .await
        {
            Ok((content, tool_calls)) => {
                if !tool_calls.is_empty() {
                    let tool_desc = content.trim();
                    if !tool_desc.is_empty() {
                        let _ = llm_repo::add_message(
                            ctx.pool,
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
                        content: if content.is_empty() {
                            None
                        } else {
                            Some(content)
                        },
                        tool_calls: Some(tool_calls.clone()),
                        tool_call_id: None,
                        name: None,
                    });

                    if let Some(reporter) = ctx.stage_reporter {
                        if let Some(first_tc) = tool_calls.first() {
                            reporter.report_tool_call(&first_tc.function.name).await;
                        }
                    }

                    let tool_futures = tool_calls.iter().map(|tool_call| {
                        let func_name = tool_call.function.name.clone();
                        let tool_call_id = tool_call.id.clone();
                        let args = tool_call.function.arguments.clone();
                        let services = ctx.services;
                        let pool = ctx.pool;
                        let broadcaster = ctx.broadcaster;
                        let owner_id = ctx.owner_id;
                        let room_id = ctx.room_id;
                        async move {
                            info!(
                                "LLM turn [{}/{}] executing tool: {} for owner {}",
                                iteration, max_iterations, func_name, owner_id
                            );
                            let result_str = match ToolRegistry::execute_tool(
                                services,
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

                    if let Some(reporter) = ctx.stage_reporter {
                        reporter.report_stage("🧠 正在组织回复...").await;
                    }

                    for (tool_call_id, func_name, result_str) in execution_results {
                        tool_calls_executed.push(func_name.clone());

                        let eid = ctx
                            .stage_reporter
                            .and_then(|r| r.event_id())
                            .map(|e| e.as_str());
                        crate::interface::channels::matrix::reaction_targets::record_ai_tool_target(
                            ctx.pool,
                            eid,
                            ctx.room_id,
                            ctx.owner_id,
                            &func_name,
                            &result_str,
                        )
                        .await;

                        // Persist tool result message
                        let _ = llm_repo::add_message(
                            ctx.pool,
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

                if !content.trim().is_empty() {
                    let is_hallucinated_tool = tool_calls_executed.is_empty()
                        && iteration == 1
                        && (content.contains("record_finance_entry")
                            || content.contains("add_expense")
                            || content.contains("add_todo")
                            || content.contains("create_reminder")
                            || content.contains("Tool called:")
                            || content.contains("Receipt recorded")
                            || content.contains("Receipt logged"));

                    if is_hallucinated_tool {
                        warn!("LLM simulated tool call in text without invoking function API; correcting model in next turn");
                        chat_messages.push(ChatCompletionMessage {
                            role: "assistant".to_string(),
                            content: Some(content),
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

                    final_content = content;
                    break;
                }
                final_content = "Received empty response from AI model.".to_string();
                break;
            }
            Err(err) => {
                error!("LLM completion failed: {}", err);
                return Err(AiError::Provider(err));
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
            stream: Some(ctx.stage_reporter.is_some()),
        };
        if let Ok((content, _)) = super::stream::execute_stream_or_complete(
            &client,
            &resolved_config,
            &final_req,
            ctx.stage_reporter,
        )
        .await
        {
            if !content.trim().is_empty() {
                final_content = content;
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

    if let Err(e) = llm_repo::add_message(
        ctx.pool,
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

    if let Some(b) = ctx.broadcaster {
        b.send(SystemEvent::AiConversationUpdated {
            id: conversation.id.clone(),
            owner_id: ctx.owner_id.to_string(),
        });
    }

    info!(
        "LLM turn completed for owner {} in room {} (tools: {}, reply chars: {})",
        ctx.owner_id,
        ctx.room_id,
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
