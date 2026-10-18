use tracing::{error, info, warn};

use super::context::{AiTurnResult, LlmTurnContext};
use super::error::AiError;
use super::guard;
use super::resolved_config::ResolvedAiConfig;
use super::tools::ToolRegistry;
use crate::domain::llm::ToolStepOutcome;
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

    let snapshot = ctx.services.app_config.snapshot();
    let provider_name = snapshot.llm_default_provider();
    if snapshot.is_customized(crate::domain::config::SettingKey::LlmDefaultProvider)
        && provider_name.eq_ignore_ascii_case("custom")
        && ctx
            .config
            .base_url
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
        && user_config
            .as_ref()
            .and_then(|u| u.api_base_url.as_deref())
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(AiError::Validation(
            "Custom LLM provider requires AI_ENDPOINT or a user API base URL".to_string(),
        ));
    }

    let resolved_config =
        ResolvedAiConfig::resolve_effective(ctx.services, ctx.config, user_config.as_ref()).await;

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

    if let Some(sink) = ctx.progress {
        sink.report_conversation(&conversation.id).await;
    }

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

    if let Some(sink) = ctx.progress {
        sink.report_stage("🤔 思考中...").await;
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
    let mut tool_results: Vec<(String, String)> = Vec::new();
    // Quoted text and outside-content tool results can carry instructions;
    // while tainted, sensitive tools wait for the user's confirmation.
    let mut tainted = ctx.untrusted_input;
    let mut any_tool_succeeded = false;
    let mut corrected = false;
    let request_timeout = std::time::Duration::from_secs(resolved_config.request_timeout_seconds);

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

        let step_result = match tokio::time::timeout(
            request_timeout,
            super::stream::execute_stream_or_complete(
                &client,
                &resolved_config,
                &request,
                ctx.progress,
            ),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => Err(format!(
                "LLM request timed out after {}s",
                resolved_config.request_timeout_seconds
            )),
        };

        match step_result {
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

                    if let Some(sink) = ctx.progress {
                        if let Some(first_tc) = tool_calls.first() {
                            sink.report_tool_call(&first_tc.function.name).await;
                        }
                        for tc in &tool_calls {
                            sink.report_tool_started(&tc.id, &tc.function.name).await;
                        }
                    }

                    let outcomes = super::tool_round::run_calls(
                        ctx,
                        &tool_calls,
                        tainted,
                        iteration,
                        max_iterations,
                    )
                    .await;

                    if let Some(sink) = ctx.progress {
                        sink.report_stage("🧠 正在组织回复...").await;
                    }

                    for outcome in outcomes {
                        if let Some(sink) = ctx.progress {
                            sink.report_tool_finished(
                                &outcome.call_id,
                                &outcome.name,
                                (&outcome).into(),
                            )
                            .await;
                        }
                        if !outcome.refused {
                            tool_calls_executed.push(outcome.name.clone());
                            tool_results.push((outcome.name.clone(), outcome.result.clone()));
                        }
                        any_tool_succeeded |= outcome.succeeded;
                        // Outside content taints the rounds after this one,
                        // not the calls already running beside it.
                        tainted |= guard::returns_outside_content(&outcome.name);

                        // Persist tool result message
                        let _ = llm_repo::add_tool_message(
                            ctx.pool,
                            &conversation.id,
                            &outcome.call_id,
                            &outcome.name,
                            &outcome.result,
                            ToolStepOutcome::from(&outcome),
                        )
                        .await;

                        chat_messages.push(ChatCompletionMessage {
                            role: "tool".to_string(),
                            content: Some(guard::wrap_tool_output(&outcome.name, &outcome.result)),
                            tool_calls: None,
                            tool_call_id: Some(outcome.call_id),
                            name: Some(outcome.name),
                        });
                    }
                    continue;
                }

                if !content.trim().is_empty() {
                    if !corrected && guard::claims_unrecorded_success(&content, any_tool_succeeded)
                    {
                        warn!("Reply claims success but no tool succeeded; asking the model to correct itself once");
                        corrected = true;
                        if let Some(sink) = ctx.progress {
                            sink.retract_segment().await;
                        }
                        chat_messages.push(ChatCompletionMessage {
                            role: "assistant".to_string(),
                            content: Some(content),
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                        });
                        chat_messages.push(ChatCompletionMessage {
                            role: "user".to_string(),
                            content: Some(guard::CORRECTION_MESSAGE.to_string()),
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

    // The correction used the last round: the user still gets the notice,
    // not an empty-reply message or a summary of calls that all failed.
    if final_content.trim().is_empty() && corrected && !any_tool_succeeded {
        final_content = guard::UNRECORDED_NOTICE.to_string();
    }

    if final_content.trim().is_empty() && !tool_calls_executed.is_empty() {
        let final_req = ChatCompletionRequest {
            model: resolved_config.model_name.clone(),
            messages: chat_messages.clone(),
            temperature: Some(resolved_config.temperature),
            max_tokens: Some(resolved_config.max_tokens),
            tools: None,
            tool_choice: None,
            stream: Some(ctx.progress.is_some()),
        };
        if let Ok(Ok((content, _))) = tokio::time::timeout(
            request_timeout,
            super::stream::execute_stream_or_complete(
                &client,
                &resolved_config,
                &final_req,
                ctx.progress,
            ),
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

    // The correction is offered once; a reply that still claims success
    // must not reach the user as if something was saved.
    if guard::claims_unrecorded_success(&final_content, any_tool_succeeded) {
        if let Some(sink) = ctx.progress {
            sink.retract_segment().await;
        }
        final_content = guard::UNRECORDED_NOTICE.to_string();
    }

    if let Err(e) = ctx
        .services
        .llm
        .append_assistant_message(ctx.owner_id, &conversation.id, &final_content)
        .await
    {
        warn!("Failed to persist assistant message: {}", e);
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
        tool_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::infrastructure::inference::test_support::spawn_server;
    use crate::llm::progress::ProgressSink;
    use crate::llm::test_support::*;
    use axum::{routing::post, Json, Router};
    use std::sync::Mutex;

    /// Records what the agent tells a channel, in order.
    #[derive(Default)]
    struct RecordingSink {
        seen: Mutex<Vec<String>>,
    }

    impl RecordingSink {
        fn note(&self, entry: String) {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push(entry);
            }
        }
        fn seen(&self) -> Vec<String> {
            self.seen.lock().map(|s| s.clone()).unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl ProgressSink for RecordingSink {
        async fn report_stage(&self, text: &str) {
            self.note(format!("stage:{text}"));
        }
        async fn report_tool_call(&self, tool_name: &str) {
            self.note(format!("tool:{tool_name}"));
        }
        async fn push_delta(&self, delta: &str) {
            self.note(format!("delta:{delta}"));
        }
        async fn flush(&self) {
            self.note("flush".to_string());
        }
        async fn report_conversation(&self, conversation_id: &str) {
            self.note(format!("conv:{conversation_id}"));
        }
        async fn report_tool_started(&self, call_id: &str, name: &str) {
            self.note(format!("start:{call_id}:{name}"));
        }
        async fn report_tool_finished(&self, call_id: &str, _name: &str, outcome: ToolStepOutcome) {
            self.note(format!("finish:{call_id}:{}", outcome.as_str()));
        }
        async fn retract_segment(&self) {
            self.note("retract".to_string());
        }
        fn message_ref(&self) -> Option<String> {
            Some("$stage".to_string())
        }
    }

    /// A scripted OpenAI-compatible endpoint: while the conversation holds no
    /// tool result it asks for one `list_finance_transactions` call, afterwards
    /// it answers. Always replies as an SSE stream, which is what a turn with a
    /// progress sink asks for.
    async fn scripted_llm() -> String {
        let app = Router::new().route(
            "/chat/completions",
            post(|Json(body): Json<serde_json::Value>| async move {
                let has_tool_result = body["messages"]
                    .as_array()
                    .map(|m| m.iter().any(|x| x["role"] == "tool"))
                    .unwrap_or(false);
                let chunk = if has_tool_result {
                    serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
                        "delta":{"content":"Nothing to report."},"finish_reason":null}]})
                } else {
                    serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
                        "delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function",
                        "function":{"name":"list_finance_transactions","arguments":"{}"}}]},
                        "finish_reason":null}]})
                };
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                )
            }),
        );
        spawn_server(app).await
    }

    #[tokio::test]
    async fn a_turn_reports_tool_stages_through_any_progress_sink() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("turn-progress-sink");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let config = crate::config::AiEnvConfig {
            base_url: Some(scripted_llm().await),
            api_key: Some("sk-test".to_string()),
            allow_private_base_url: true,
            ..Default::default()
        };
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        let result = run_turn(&ctx, "what did I spend?", None)
            .await
            .expect("the scripted turn should complete");

        let seen = sink.seen();
        assert!(
            seen.contains(&"tool:list_finance_transactions".to_string()),
            "{seen:?}"
        );
        assert!(
            seen.contains(&"stage:🧠 正在组织回复...".to_string()),
            "{seen:?}"
        );
        assert_eq!(result.tool_results.len(), 1);
        assert_eq!(result.tool_results[0].0, "list_finance_transactions");
        assert_eq!(result.message, "Nothing to report.");
    }

    #[tokio::test]
    async fn a_turn_reports_its_conversation_first() {
        let (pool, owner, _) = pool_with_todo("turn-report-conv").await;
        let services = AppServices::new(&pool);
        let url = scripted_stream_replies(vec![text_reply("Hi.")]).await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        let result = run_turn(&ctx, "hello", None).await.unwrap();

        let seen = sink.seen();
        assert_eq!(
            seen.first().cloned(),
            Some(format!("conv:{}", result.conversation_id)),
            "{seen:?}"
        );
    }

    #[tokio::test]
    async fn every_call_in_a_round_is_reported_started_and_finished() {
        let (pool, owner, _) = pool_with_todo("turn-report-steps").await;
        let services = AppServices::new(&pool);
        let url = scripted_stream_replies(vec![
            tool_calls_reply(&[
                ("list_finance_transactions", serde_json::json!({})),
                ("get_my_tasks", serde_json::json!({})),
            ]),
            text_reply("Done."),
        ])
        .await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        run_turn(&ctx, "summarise", None).await.unwrap();

        let steps: Vec<String> = sink
            .seen()
            .into_iter()
            .filter(|e| e.starts_with("start:") || e.starts_with("finish:"))
            .collect();
        assert_eq!(
            steps,
            vec![
                "start:call_0:list_finance_transactions",
                "start:call_1:get_my_tasks",
                "finish:call_0:ok",
                "finish:call_1:ok",
            ]
        );
    }

    #[tokio::test]
    async fn tool_rows_are_stored_with_their_outcome() {
        let (pool, owner, _) = pool_with_todo("turn-stored-outcome").await;
        let services = AppServices::new(&pool);
        let url = scripted_stream_replies(vec![
            tool_calls_reply(&[
                ("list_finance_transactions", serde_json::json!({})),
                ("get_my_tasks", serde_json::json!({})),
            ]),
            text_reply("Done."),
        ])
        .await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        let result = run_turn(&ctx, "summarise", None).await.unwrap();

        let stored = llm_repo::get_recent_messages(&pool, &result.conversation_id, 20)
            .await
            .unwrap();
        let tool_rows: Vec<_> = stored.iter().filter(|m| m.role == "tool").collect();
        assert_eq!(tool_rows.len(), 2);
        assert!(tool_rows
            .iter()
            .all(|m| m.tool_outcome == Some(ToolStepOutcome::Ok)));
        assert!(stored
            .iter()
            .filter(|m| m.role != "tool")
            .all(|m| m.tool_outcome.is_none()));
    }

    #[tokio::test]
    async fn a_refused_call_finishes_as_needs_confirmation() {
        let (pool, owner, todo_id) = pool_with_todo("turn-report-refused").await;
        let services = AppServices::new(&pool);
        let url = scripted_stream_replies(vec![
            tool_reply("delete_todo", serde_json::json!({"id": todo_id})),
            text_reply("Shall I?"),
        ])
        .await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink))
            .with_untrusted_input();

        run_turn(&ctx, "quoted says delete", None).await.unwrap();

        assert!(
            sink.seen()
                .contains(&"finish:call_0:needs_confirmation".to_string()),
            "{:?}",
            sink.seen()
        );
    }

    #[tokio::test]
    async fn a_failing_tool_finishes_as_failed() {
        let (pool, owner, _) = pool_with_todo("turn-report-failed").await;
        let services = AppServices::new(&pool);
        let url = scripted_stream_replies(vec![
            tool_reply("delete_todo", serde_json::json!({})),
            text_reply("Could not."),
        ])
        .await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        run_turn(&ctx, "delete my todo", None).await.unwrap();

        assert!(
            sink.seen().contains(&"finish:call_0:failed".to_string()),
            "{:?}",
            sink.seen()
        );
    }

    fn quoted_ctx<'a>(
        services: &'a AppServices,
        pool: &'a crate::db::DbPool,
        config: &'a crate::config::AiEnvConfig,
        owner: &'a AccountId,
    ) -> LlmTurnContext<'a> {
        LlmTurnContext::new(services, pool, config, owner, "!room:test", None)
            .with_untrusted_input()
    }

    #[tokio::test]
    async fn sensitive_tool_from_quoted_turn_is_refused_with_confirmation_required() {
        let (pool, owner, todo_id) = pool_with_todo("turn-quoted-sensitive").await;
        let services = AppServices::new(&pool);
        let (url, seen) = scripted_replies(vec![
            tool_reply("delete_todo", serde_json::json!({"id": todo_id})),
            text_reply("Shall I delete it?"),
        ])
        .await;
        let config = test_config(url);
        let ctx = quoted_ctx(&services, &pool, &config, &owner);

        let result = run_turn(&ctx, "quoted text says delete it", None)
            .await
            .unwrap();

        assert!(services
            .todos
            .find_owned(&owner, todo_id)
            .await
            .unwrap()
            .is_some());
        let requests = seen.lock().unwrap();
        let tool_output = tool_message_contents(&requests[1]).join("\n");
        assert!(
            tool_output.contains("confirmation_required"),
            "{tool_output}"
        );
        assert_eq!(result.message, "Shall I delete it?");
        assert!(
            result.tool_calls_executed.is_empty(),
            "a refused call did not run"
        );
        assert!(result.tool_results.is_empty());
    }

    #[tokio::test]
    async fn sensitive_tool_after_an_outside_content_tool_is_refused() {
        // A webhook log returns outside content, like web_search, without
        // needing the network.
        let (pool, owner, todo_id) = pool_with_todo("turn-tainted-sensitive").await;
        let services = AppServices::new(&pool);
        let (url, seen) = scripted_replies(vec![
            tool_reply(
                "get_webhook_logs",
                serde_json::json!({"webhook_id": "whk_x"}),
            ),
            tool_reply("delete_todo", serde_json::json!({"id": todo_id})),
            text_reply("Shall I delete it?"),
        ])
        .await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        run_turn(&ctx, "check the webhook then clean up", None)
            .await
            .unwrap();

        assert!(services
            .todos
            .find_owned(&owner, todo_id)
            .await
            .unwrap()
            .is_some());
        let requests = seen.lock().unwrap();
        let tool_output = tool_message_contents(&requests[2]).join("\n");
        assert!(
            tool_output.contains("confirmation_required"),
            "{tool_output}"
        );
    }

    #[tokio::test]
    async fn sensitive_tool_in_plain_turn_executes() {
        let (pool, owner, todo_id) = pool_with_todo("turn-plain-sensitive").await;
        let services = AppServices::new(&pool);
        let (url, _seen) = scripted_replies(vec![
            tool_reply("delete_todo", serde_json::json!({"id": todo_id})),
            text_reply("Deleted."),
        ])
        .await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        run_turn(&ctx, "delete my rent todo", None).await.unwrap();

        assert!(services
            .todos
            .find_owned(&owner, todo_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn same_round_sensitive_and_outside_content_calls_both_run() {
        // Documented behaviour: calls in one round run concurrently, so
        // outside content only taints the rounds after it.
        let (pool, owner, todo_id) = pool_with_todo("turn-same-round").await;
        let services = AppServices::new(&pool);
        let (url, _seen) = scripted_replies(vec![
            tool_calls_reply(&[
                (
                    "get_webhook_logs",
                    serde_json::json!({"webhook_id": "whk_x"}),
                ),
                ("delete_todo", serde_json::json!({"id": todo_id})),
            ]),
            text_reply("Done."),
        ])
        .await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        let result = run_turn(&ctx, "check the webhook and delete it", None)
            .await
            .unwrap();

        assert!(result
            .tool_calls_executed
            .contains(&"delete_todo".to_string()));
        assert!(result
            .tool_calls_executed
            .contains(&"get_webhook_logs".to_string()));
        assert!(services
            .todos
            .find_owned(&owner, todo_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn checkmark_reply_without_successful_tool_triggers_one_correction() {
        let (pool, owner, _) = pool_with_todo("turn-checkmark-lie").await;
        let services = AppServices::new(&pool);
        let (url, seen) = scripted_replies(vec![
            text_reply("✅ **Task added**\n\n- **Task**: Buy milk"),
            text_reply("I could not save it."),
        ])
        .await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        let result = run_turn(&ctx, "add milk to my tasks", None).await.unwrap();

        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2, "exactly one correction round");
        let last = requests[1]["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(result.message, "I could not save it.");
    }

    #[tokio::test]
    async fn checkmark_reply_after_successful_tool_is_kept() {
        let (pool, owner, _) = pool_with_todo("turn-checkmark-true").await;
        let services = AppServices::new(&pool);
        let (url, seen) = scripted_replies(vec![
            tool_reply("add_todo", serde_json::json!({"text": "Buy milk"})),
            text_reply("✅ **Task added**\n\n- **Task**: Buy milk"),
        ])
        .await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        let result = run_turn(&ctx, "add milk to my tasks", None).await.unwrap();

        assert_eq!(seen.lock().unwrap().len(), 2, "no correction round");
        assert!(result.message.starts_with('✅'), "{}", result.message);
    }

    #[tokio::test]
    async fn fact_check_reply_with_a_midtext_checkmark_is_kept() {
        let (pool, owner, _) = pool_with_todo("turn-factcheck").await;
        let services = AppServices::new(&pool);
        let reply = "Claim 1 is accurate ✅ but claim 2 is false ❌.";
        let (url, seen) = scripted_replies(vec![text_reply(reply)]).await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        let result = run_turn(&ctx, "fact check this", None).await.unwrap();

        assert_eq!(seen.lock().unwrap().len(), 1, "no correction round");
        assert_eq!(result.message, reply);
    }

    #[tokio::test]
    async fn a_reply_that_keeps_claiming_success_after_the_correction_is_replaced() {
        let (pool, owner, _) = pool_with_todo("turn-still-lying").await;
        let services = AppServices::new(&pool);
        let (url, seen) =
            scripted_replies(vec![text_reply("✅ saved"), text_reply("✅ really saved")]).await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        let result = run_turn(&ctx, "add milk", None).await.unwrap();

        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(result.message, guard::UNRECORDED_NOTICE);
    }

    #[tokio::test]
    async fn a_correction_on_the_final_round_still_ends_with_the_notice() {
        let (pool, owner, _) = pool_with_todo("turn-final-round").await;
        let services = AppServices::new(&pool);
        // Failing calls (no id), then a claim on the fifth and last round.
        let mut script: Vec<serde_json::Value> = (0..MAX_TOOL_ROUNDS - 1)
            .map(|_| tool_reply("delete_todo", serde_json::json!({})))
            .collect();
        script.push(text_reply("✅ deleted"));
        let (url, _seen) = scripted_replies(script).await;
        let config = test_config(url);
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", None);

        let result = run_turn(&ctx, "delete my todo", None).await.unwrap();

        assert_eq!(result.message, guard::UNRECORDED_NOTICE);
    }

    #[tokio::test]
    async fn a_reply_the_guard_rejects_is_retracted() {
        let (pool, owner, _) = pool_with_todo("turn-retract-guard").await;
        let services = AppServices::new(&pool);
        let url = scripted_stream_replies(vec![
            text_reply("✅ Added the todo"),
            text_reply("I could not save it."),
        ])
        .await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        let result = run_turn(&ctx, "add milk", None).await.unwrap();

        let seen = sink.seen();
        let flush = seen.iter().position(|e| e == "flush").expect("flushed");
        let retract = seen.iter().position(|e| e == "retract").expect("retracted");
        assert!(flush < retract, "{seen:?}");
        assert_eq!(result.message, "I could not save it.");
    }

    #[tokio::test]
    async fn a_final_reply_replaced_by_the_notice_is_retracted() {
        let (pool, owner, _) = pool_with_todo("turn-retract-notice").await;
        let services = AppServices::new(&pool);
        let url =
            scripted_stream_replies(vec![text_reply("✅ saved"), text_reply("✅ really saved")])
                .await;
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        let result = run_turn(&ctx, "add milk", None).await.unwrap();

        assert_eq!(result.message, guard::UNRECORDED_NOTICE);
        let seen = sink.seen();
        let retracts = seen.iter().filter(|e| *e == "retract").count();
        assert_eq!(retracts, 2, "{seen:?}");
        assert_eq!(seen.last().map(String::as_str), Some("retract"), "{seen:?}");
    }

    #[tokio::test]
    async fn a_stream_that_fails_midway_retracts_then_sends_the_fallback() {
        let app = Router::new().route(
            "/chat/completions",
            post(|Json(body): Json<serde_json::Value>| async move {
                if body["stream"] == true {
                    let chunk = serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
                        "delta":{"content":"partial"},"finish_reason":null}]});
                    // The error follows the delta after a pause, so the first
                    // chunk is delivered before the body breaks.
                    let first = futures_util::stream::once(async move {
                        Ok::<_, std::io::Error>(bytes::Bytes::from(format!(
                            "data: {chunk}

"
                        )))
                    });
                    let broken = futures_util::stream::once(async {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        Err(std::io::Error::other("connection reset"))
                    });
                    axum::response::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from_stream(
                            futures_util::StreamExt::chain(first, broken),
                        ))
                        .unwrap()
                } else {
                    axum::response::IntoResponse::into_response(Json(text_reply("full answer")))
                }
            }),
        );
        let url = spawn_server(app).await;
        let (pool, owner, _) = pool_with_todo("turn-retract-midstream").await;
        let services = AppServices::new(&pool);
        let config = test_config(url);
        let sink = RecordingSink::default();
        let ctx = LlmTurnContext::new(&services, &pool, &config, &owner, "!room:test", Some(&sink));

        let result = run_turn(&ctx, "hello", None).await.unwrap();

        assert_eq!(result.message, "full answer");
        let seen = sink.seen();
        let start = seen
            .iter()
            .position(|e| e == "delta:partial")
            .unwrap_or_else(|| panic!("{seen:?}"));
        assert_eq!(
            &seen[start..],
            ["delta:partial", "retract", "delta:full answer", "flush"],
            "{seen:?}"
        );
    }
}
