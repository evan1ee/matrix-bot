//! ReAct reasoning and execution engine for autonomous agent runs.

use crate::application::AppServices;
use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::domain::agent::AgentStep;
use crate::infrastructure::inference::llm::{ChatCompletionRequest, LlmClient};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::llm::tools::ToolRegistry;

use super::prompts::{self, ParsedReact};
use super::sandbox;

/// The outcome of evaluating a single ReAct step.
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
    /// Step evaluation failed or proposed an illegal/unknown tool.
    Failed {
        thought: Option<String>,
        error: String,
        tokens_used: i32,
    },
}

/// Execution context for evaluating a single ReAct step.
pub struct StepContext<'a> {
    pub account_id: &'a AccountId,
    pub room_id: &'a str,
    pub goal: &'a str,
    pub steps: &'a [AgentStep],
    pub allowed_tools: Option<&'a [String]>,
    pub custom_sensitive_tools: Option<&'a [String]>,
    pub config: &'a ResolvedAiConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentEngineError {
    #[error("LLM request error: {0}")]
    Llm(String),
    #[error("Step execution error: {0}")]
    Execution(String),
}

/// The ReAct engine evaluating iterative Think -> Act -> Observe steps.
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

    /// Evaluates a single ReAct step for the given goal and history.
    pub async fn evaluate_step(
        &self,
        services: &AppServices,
        pool: &DbPool,
        ctx: &StepContext<'_>,
    ) -> Result<StepOutcome, AgentEngineError> {
        let all_tool_defs = ToolRegistry::get_tool_definitions();
        let tool_defs = sandbox::filter_tool_definitions(&all_tool_defs, ctx.allowed_tools);

        let messages = prompts::build_messages(ctx.goal, ctx.steps, &tool_defs);

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

        // 1. Check for native tool calls
        if let Some(tool_calls) = &choice.message.tool_calls {
            if let Some(first) = tool_calls.first() {
                let tool_name = first.function.name.clone();
                let tool_input = serde_json::from_str(&first.function.arguments).unwrap_or_else(
                    |_| serde_json::json!({ "arguments": first.function.arguments }),
                );
                let thought = choice
                    .message
                    .content
                    .clone()
                    .filter(|c| !c.trim().is_empty());

                return Ok(self
                    .process_tool_call(
                        services,
                        pool,
                        ctx,
                        thought,
                        tool_name,
                        tool_input,
                        tokens_used,
                    )
                    .await);
            }
        }

        // 2. Parse text content for ReAct tags (Thought: / Action: / Final Answer:)
        let content = choice.message.content.as_deref().unwrap_or("").trim();
        let parsed = prompts::parse_react_response(content);

        match parsed {
            ParsedReact::Action {
                thought,
                tool_name,
                tool_input,
            } => Ok(self
                .process_tool_call(
                    services,
                    pool,
                    ctx,
                    thought,
                    tool_name,
                    tool_input,
                    tokens_used,
                )
                .await),
            ParsedReact::FinalAnswer {
                thought,
                final_answer,
            } => Ok(StepOutcome::Completed {
                thought,
                final_answer,
                tokens_used,
            }),
        }
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

        // Safe tool execution
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
