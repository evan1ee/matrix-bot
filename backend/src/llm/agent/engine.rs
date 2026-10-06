//! Step evaluation for autonomous agent runs: native tool calling, sandboxing and approval interception.

use crate::application::AppServices;
use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::domain::agent::AgentStep;
use crate::infrastructure::inference::llm::{ChatCompletionRequest, LlmClient};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::llm::tools::ToolRegistry;

use super::prompts;
pub use super::prompts::MAX_TOOL_OUTPUT_PROMPT_CHARS;
use super::sandbox;

/// The outcome of evaluating a single agent step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// Safe tool was executed and produced an observation.
    Executed {
        thought: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_output: serde_json::Value,
        tokens_used: i32,
    },
    /// Action is sensitive and requires human confirmation before execution.
    ApprovalRequired {
        thought: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
        tokens_used: i32,
    },
    /// Agent produced a final answer or completed its goal.
    Completed {
        thought: Option<String>,
        final_answer: String,
        tokens_used: i32,
    },
    /// The model asked for a tool but its arguments are not a valid JSON
    /// object. The call is never executed; the run records the failure and
    /// continues so the model can correct itself.
    InvalidCall {
        thought: Option<String>,
        tool_name: String,
        raw_arguments: String,
        error: String,
        tokens_used: i32,
    },
    /// Step evaluation failed or proposed an illegal/unknown tool.
    Failed {
        thought: Option<String>,
        error: String,
        tokens_used: i32,
    },
}

/// Execution context for evaluating a single agent step.
pub struct StepContext<'a> {
    pub account_id: &'a AccountId,
    pub room_id: &'a str,
    pub goal: &'a str,
    pub steps: &'a [AgentStep],
    pub allowed_tools: Option<&'a [String]>,
    pub custom_sensitive_tools: Option<&'a [String]>,
    /// The run's step budget, stated in the system prompt.
    pub max_steps: i32,
    pub config: &'a ResolvedAiConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentEngineError {
    #[error("LLM request error: {0}")]
    Llm(String),
    #[error("Step execution error: {0}")]
    Execution(String),
}

/// Parses a native call's arguments: empty means "no arguments", anything
/// else must be a JSON object.
fn parse_arguments(raw: &str) -> Result<serde_json::Value, String> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) if value.is_object() => Ok(value),
        Ok(_) => Err("arguments must be a JSON object".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

/// The agent engine: one model call per step, native tool calling, with
/// sandboxing and approval interception around each call. (Named after the
/// ReAct loop it grew from; it no longer parses a text protocol.)
#[derive(Clone)]
pub struct ReActEngine {
    llm_client: LlmClient,
}

impl ReActEngine {
    pub fn new(llm_client: LlmClient) -> Self {
        Self { llm_client }
    }

    pub fn from_proxy(proxy: Option<&str>) -> Self {
        Self::new(LlmClient::new(proxy))
    }

    /// Evaluates one step for the given goal and history.
    pub async fn evaluate_step(
        &self,
        services: &AppServices,
        pool: &DbPool,
        ctx: &StepContext<'_>,
    ) -> Result<StepOutcome, AgentEngineError> {
        let all_tool_defs = ToolRegistry::get_tool_definitions();
        let tool_defs = sandbox::filter_tool_definitions(&all_tool_defs, ctx.allowed_tools);

        let messages = prompts::build_messages(
            ctx.goal,
            ctx.steps,
            &tool_defs,
            ctx.steps.len() as i32 + 1,
            ctx.max_steps,
        );

        let tools_json = if tool_defs.is_empty() {
            None
        } else {
            Some(
                tool_defs
                    .iter()
                    .map(|t| serde_json::to_value(t).unwrap_or_default())
                    .collect(),
            )
        };

        let request = ChatCompletionRequest {
            model: ctx.config.model_name.clone(),
            messages,
            temperature: Some(ctx.config.temperature),
            max_tokens: Some(ctx.config.max_tokens),
            tools: tools_json,
            tool_choice: None,
            stream: Some(false),
        };

        let res = self
            .llm_client
            .chat_completion(ctx.config.endpoint(), &request)
            .await
            .map_err(|e| AgentEngineError::Llm(e.to_string()))?;

        let tokens_used = res.usage.map(|u| u.total_tokens).unwrap_or(0);

        let choice = res
            .choices
            .first()
            .ok_or_else(|| AgentEngineError::Llm("Empty LLM response choices".to_string()))?;

        let content = choice.message.content.as_deref().unwrap_or("").trim();

        // Native tool calls are the only way to act; the first one is taken.
        if let Some(first) = choice
            .message
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
        {
            let thought = Some(content.to_string()).filter(|c| !c.is_empty());
            let tool_name = first.function.name.clone();
            return Ok(match parse_arguments(&first.function.arguments) {
                Ok(tool_input) => {
                    self.process_tool_call(
                        services,
                        pool,
                        ctx,
                        thought,
                        tool_name,
                        tool_input,
                        tokens_used,
                    )
                    .await
                }
                // Arguments that do not parse are never coerced into a call.
                Err(error) => StepOutcome::InvalidCall {
                    thought,
                    error: format!("Invalid arguments for tool '{tool_name}': {error}"),
                    tool_name,
                    raw_arguments: first.function.arguments.clone(),
                    tokens_used,
                },
            });
        }

        // Plain text is the final answer; there is no text protocol to parse.
        if content.is_empty() {
            return Ok(StepOutcome::Failed {
                thought: None,
                error: "The model returned neither a tool call nor an answer".to_string(),
                tokens_used,
            });
        }
        Ok(StepOutcome::Completed {
            thought: None,
            final_answer: content.to_string(),
            tokens_used,
        })
    }

    /// Validates sandboxing, intercepts sensitive tools, or executes safe tools.
    #[allow(clippy::too_many_arguments)]
    pub async fn process_tool_call(
        &self,
        services: &AppServices,
        pool: &DbPool,
        ctx: &StepContext<'_>,
        thought: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
        tokens_used: i32,
    ) -> StepOutcome {
        // Whitelist validation
        if !sandbox::is_tool_allowed(&tool_name, ctx.allowed_tools) {
            return StepOutcome::Failed {
                thought,
                error: format!("Tool '{tool_name}' is not allowed by task policy"),
                tokens_used,
            };
        }

        // Sensitive tool interception
        if sandbox::is_sensitive_tool(&tool_name, ctx.custom_sensitive_tools) {
            return StepOutcome::ApprovalRequired {
                thought,
                tool_name,
                tool_input,
                tokens_used,
            };
        }

        self.run_tool(
            services,
            pool,
            ctx,
            thought,
            tool_name,
            tool_input,
            tokens_used,
        )
        .await
    }

    /// Runs a call the user approved: the held call from an earlier step, with
    /// no model call and no sensitivity check (the user just decided). The
    /// task whitelist still applies, and the owner comes from the run's
    /// account, as for any call.
    pub async fn execute_approved(
        &self,
        services: &AppServices,
        pool: &DbPool,
        ctx: &StepContext<'_>,
        tool_name: String,
        tool_input: serde_json::Value,
    ) -> StepOutcome {
        let thought = Some("Running the action the user approved".to_string());
        if !sandbox::is_tool_allowed(&tool_name, ctx.allowed_tools) {
            return StepOutcome::Failed {
                thought,
                error: format!("Tool '{tool_name}' is not allowed by task policy"),
                tokens_used: 0,
            };
        }
        self.run_tool(services, pool, ctx, thought, tool_name, tool_input, 0)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_tool(
        &self,
        services: &AppServices,
        pool: &DbPool,
        ctx: &StepContext<'_>,
        thought: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
        tokens_used: i32,
    ) -> StepOutcome {
        let args_json = tool_input.to_string();
        let exec_result = ToolRegistry::execute_tool(
            services,
            pool,
            ctx.account_id,
            ctx.room_id,
            &tool_name,
            &args_json,
        )
        .await;

        let tool_output = match exec_result {
            Ok(output_str) => serde_json::from_str(&output_str)
                .unwrap_or_else(|_| serde_json::json!({ "output": output_str })),
            Err(err_str) => serde_json::json!({ "error": err_str }),
        };

        StepOutcome::Executed {
            thought,
            tool_name,
            tool_input,
            tool_output,
            tokens_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::llm::test_support::*;

    async fn evaluate_against(
        seed: &str,
        replies: Vec<serde_json::Value>,
    ) -> (StepOutcome, crate::db::DbPool, AccountId, Seen) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let account = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &account).await;
        let (url, seen) = scripted_replies(replies).await;
        let config = ResolvedAiConfig::from_user_config(&test_config(url), None);
        let ctx = StepContext {
            account_id: &account,
            room_id: "agent:internal",
            goal: "Add milk to my todos",
            steps: &[],
            allowed_tools: None,
            custom_sensitive_tools: None,
            max_steps: 10,
            config: &config,
        };
        let outcome = ReActEngine::from_proxy(None)
            .evaluate_step(&services, &pool, &ctx)
            .await
            .expect("the scripted step should evaluate");
        (outcome, pool, account, seen)
    }

    #[tokio::test]
    async fn malformed_native_arguments_never_execute_tool() {
        let (outcome, pool, account, _) = evaluate_against(
            "agent-invalid-args",
            vec![
                serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
                "message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1",
                "type":"function","function":{"name":"add_todo",
                "arguments":"{\"text\": \"Buy milk\""}}]},"finish_reason":"tool_calls"}]}),
            ],
        )
        .await;

        match outcome {
            StepOutcome::InvalidCall {
                tool_name,
                raw_arguments,
                ..
            } => {
                assert_eq!(tool_name, "add_todo");
                assert!(raw_arguments.contains("Buy milk"));
            }
            other => panic!("Expected InvalidCall, got {other:?}"),
        }
        let todos = AppServices::new(&pool)
            .todos
            .list(&account, None, None, 10, 0)
            .await
            .unwrap();
        assert!(todos.is_empty(), "nothing may run on unparseable arguments");
    }

    #[tokio::test]
    async fn text_reply_is_final_answer() {
        let (outcome, _, _, _) = evaluate_against(
            "agent-text-final",
            vec![text_reply("Thought: nothing to do\nFinal Answer: x")],
        )
        .await;

        match outcome {
            StepOutcome::Completed { final_answer, .. } => {
                assert_eq!(final_answer, "Thought: nothing to do\nFinal Answer: x");
            }
            other => panic!("Expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_empty_reply_fails_the_step() {
        let (outcome, _, _, _) = evaluate_against("agent-empty", vec![text_reply("   ")]).await;
        assert!(matches!(outcome, StepOutcome::Failed { .. }), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_native_call_with_valid_arguments_still_executes() {
        let (outcome, pool, account, _) = evaluate_against(
            "agent-valid-call",
            vec![tool_reply(
                "add_todo",
                serde_json::json!({"text": "Buy milk"}),
            )],
        )
        .await;

        assert!(
            matches!(outcome, StepOutcome::Executed { .. }),
            "{outcome:?}"
        );
        let todos = AppServices::new(&pool)
            .todos
            .list(&account, None, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(todos.len(), 1);
    }

    #[tokio::test]
    async fn process_tool_call_intercepts_sensitive_tool() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let account = AccountId::from_seed("test_agent_engine");
        crate::db::pool::seed_account(&pool, &account).await;

        let engine = ReActEngine::from_proxy(None);
        let config =
            ResolvedAiConfig::from_user_config(&crate::config::AiEnvConfig::default(), None);

        let ctx = StepContext {
            account_id: &account,
            room_id: "matrix:!room:server",
            goal: "Rebalance portfolio",
            steps: &[],
            allowed_tools: None,
            custom_sensitive_tools: None,
            max_steps: 10,
            config: &config,
        };

        let outcome = engine
            .process_tool_call(
                &services,
                &pool,
                &ctx,
                Some("I must buy Apple stock".to_string()),
                "buy_portfolio_holding".to_string(),
                serde_json::json!({"symbol": "AAPL", "quantity": 10}),
                120,
            )
            .await;

        match outcome {
            StepOutcome::ApprovalRequired {
                thought,
                tool_name,
                tool_input,
                tokens_used,
            } => {
                assert_eq!(thought, Some("I must buy Apple stock".to_string()));
                assert_eq!(tool_name, "buy_portfolio_holding");
                assert_eq!(tool_input["symbol"], "AAPL");
                assert_eq!(tokens_used, 120);
            }
            other => panic!("Expected ApprovalRequired, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn process_tool_call_blocks_unauthorized_tool() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let account = AccountId::from_seed("test_agent_engine_2");
        crate::db::pool::seed_account(&pool, &account).await;

        let engine = ReActEngine::from_proxy(None);
        let config =
            ResolvedAiConfig::from_user_config(&crate::config::AiEnvConfig::default(), None);

        let allowed = vec!["get_my_bookmarks".to_string()];
        let ctx = StepContext {
            account_id: &account,
            room_id: "matrix:!room:server",
            goal: "Search bookmarks only",
            steps: &[],
            allowed_tools: Some(&allowed),
            custom_sensitive_tools: None,
            max_steps: 10,
            config: &config,
        };

        let outcome = engine
            .process_tool_call(
                &services,
                &pool,
                &ctx,
                Some("Attempting disallowed tool".to_string()),
                "add_todo".to_string(),
                serde_json::json!({"title": "Sneaky todo"}),
                50,
            )
            .await;

        match outcome {
            StepOutcome::Failed { error, .. } => {
                assert!(error.contains("not allowed"));
            }
            other => panic!("Expected Failed, got {:?}", other),
        }
    }
}
