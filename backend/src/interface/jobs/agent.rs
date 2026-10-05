//! `agent.execute_run`: executes an autonomous agent run step-by-step.
//!
//! Evaluates agent steps (native tool calling), validates sandboxing, halts on sensitive tools
//! for human approval, and notifies origin conversations via `Outbound`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::{deliverable, message, parse_payload, Outbound};
use crate::application::AppServices;
use crate::config::AiEnvConfig;
use crate::db::DbPool;
use crate::domain::agent::{AgentRunId, AgentStep, RunStatus, StepStatus};
use crate::domain::jobs::{AgentExecutePayload, JobType};
use crate::interface::channels::message::Target;
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::llm::agent::{ReActEngine, StepContext, StepOutcome};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::utils::html::html_escape;
use crate::utils::time::now_at_storage_precision;

pub struct AgentExecuteRunHandler {
    services: AppServices,
    pool: DbPool,
    outbound: Arc<dyn Outbound>,
    engine: ReActEngine,
    ai_env_config: AiEnvConfig,
}

impl AgentExecuteRunHandler {
    pub fn new(
        services: AppServices,
        pool: DbPool,
        outbound: Arc<dyn Outbound>,
        engine: ReActEngine,
        ai_env_config: AiEnvConfig,
    ) -> Self {
        Self {
            services,
            pool,
            outbound,
            engine,
            ai_env_config,
        }
    }

    async fn deliver_message(&self, conversation: Option<&str>, plain: &str, html: &str) {
        let Some(room_str) = conversation else {
            return;
        };
        let Some((target_address, _channel)) = deliverable(room_str) else {
            return;
        };
        let msg = message(Target::Conversation(target_address), plain, html);
        let _ = self.outbound.send(msg).await;
    }
}

#[async_trait]
impl JobHandler for AgentExecuteRunHandler {
    fn job_type(&self) -> JobType {
        JobType::AgentExecuteRun
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(600)
    }

    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError> {
        let p: AgentExecutePayload = parse_payload(payload)?;
        let owner = ctx.account_id.ok_or_else(|| {
            JobError::Permanent(anyhow::anyhow!("agent.execute_run without an account"))
        })?;

        let run = self
            .services
            .agent
            .get_run_internal(AgentRunId::new(p.run_id))
            .await
            .map_err(JobError::from)?;

        if run.status.is_terminal() || run.status == RunStatus::WaitingApproval {
            return Ok(());
        }

        let user_config = self
            .services
            .llm
            .get_user_config(&owner)
            .await
            .map_err(JobError::from)?;
        let resolved_config =
            ResolvedAiConfig::from_user_config(&self.ai_env_config, user_config.as_ref());

        let mut total_tokens = run.total_tokens;

        while !ctx.cancel.is_cancelled() {
            let steps = self
                .services
                .agent
                .get_run_steps(&owner, run.id)
                .await
                .map_err(JobError::from)?;

            let current_step_num = steps.len() as i32 + 1;
            if current_step_num > run.max_steps {
                let error_msg = format!(
                    "Max steps limit ({}) reached without completion",
                    run.max_steps
                );
                self.services
                    .agent
                    .fail_run(run.id, &error_msg, total_tokens)
                    .await
                    .map_err(JobError::from)?;

                self.deliver_message(
                    p.origin_conversation.as_deref(),
                    &format!("Agent run failed: {error_msg}"),
                    &format!(
                        "<p>❌ <b>Agent Run Failed</b>: {}</p>",
                        html_escape(&error_msg)
                    ),
                )
                .await;
                return Ok(());
            }

            let step_ctx = StepContext {
                account_id: &owner,
                room_id: p.origin_conversation.as_deref().unwrap_or("agent:internal"),
                goal: &run.goal,
                steps: &steps,
                allowed_tools: None,
                custom_sensitive_tools: None,
                max_steps: run.max_steps,
                config: &resolved_config,
            };

            let outcome = self
                .engine
                .evaluate_step(&self.services, &self.pool, &step_ctx)
                .await
                .map_err(|e| JobError::Retryable(anyhow::anyhow!("LLM evaluation error: {e}")))?;

            let now = now_at_storage_precision();

            match outcome {
                StepOutcome::Executed {
                    thought,
                    tool_name,
                    tool_input,
                    tool_output,
                    tokens_used,
                } => {
                    total_tokens = total_tokens.saturating_add(tokens_used);
                    let step = AgentStep::new(
                        run.id,
                        current_step_num,
                        thought,
                        Some(tool_name),
                        Some(tool_input),
                        Some(tool_output),
                        StepStatus::Success,
                        tokens_used,
                        now,
                    );
                    self.services
                        .agent
                        .record_step(run.id, &step)
                        .await
                        .map_err(JobError::from)?;
                }
                StepOutcome::InvalidCall {
                    thought,
                    tool_name,
                    error,
                    tokens_used,
                    ..
                } => {
                    // Never executed. Recorded as a failed step so the next
                    // evaluation shows the model its mistake; the run goes on
                    // and the step budget bounds repeated mistakes.
                    total_tokens = total_tokens.saturating_add(tokens_used);
                    let step = AgentStep::new(
                        run.id,
                        current_step_num,
                        thought,
                        Some(tool_name),
                        None,
                        Some(serde_json::json!({ "error": error })),
                        StepStatus::Failed,
                        tokens_used,
                        now,
                    );
                    self.services
                        .agent
                        .record_step(run.id, &step)
                        .await
                        .map_err(JobError::from)?;
                }
                StepOutcome::ApprovalRequired {
                    thought,
                    tool_name,
                    tool_input,
                    tokens_used,
                } => {
                    let step = AgentStep::new(
                        run.id,
                        current_step_num,
                        thought,
                        Some(tool_name.clone()),
                        Some(tool_input.clone()),
                        None,
                        StepStatus::Blocked,
                        tokens_used,
                        now,
                    );
                    self.services
                        .agent
                        .record_step(run.id, &step)
                        .await
                        .map_err(JobError::from)?;

                    let approval = self
                        .services
                        .agent
                        .require_approval(run.id, tool_name.clone(), tool_input.clone())
                        .await
                        .map_err(JobError::from)?;

                    let plain = format!(
                        "⚠️ Agent action requires approval:\nTool: {}\nInput: {}\nTo confirm: !agent confirm {}\nTo reject: !agent reject {}",
                        tool_name, tool_input, approval.id, approval.id
                    );
                    let html = format!(
                        "<p>⚠️ <b>Agent Action Requires Confirmation</b></p><ul><li><b>Tool:</b> <code>{}</code></li><li><b>Input:</b> <code>{}</code></li></ul><p>Reply with <code>!agent confirm {}</code> or <code>!agent reject {}</code></p>",
                        html_escape(&tool_name),
                        html_escape(&tool_input.to_string()),
                        approval.id,
                        approval.id
                    );

                    self.deliver_message(p.origin_conversation.as_deref(), &plain, &html)
                        .await;

                    return Ok(());
                }
                StepOutcome::Completed {
                    thought,
                    final_answer,
                    tokens_used,
                } => {
                    total_tokens = total_tokens.saturating_add(tokens_used);
                    let step = AgentStep::new(
                        run.id,
                        current_step_num,
                        thought,
                        None,
                        None,
                        Some(serde_json::json!({ "final_answer": &final_answer })),
                        StepStatus::Success,
                        tokens_used,
                        now,
                    );
                    self.services
                        .agent
                        .record_step(run.id, &step)
                        .await
                        .map_err(JobError::from)?;
                    self.services
                        .agent
                        .complete_run(run.id, total_tokens)
                        .await
                        .map_err(JobError::from)?;

                    let plain = format!("🎯 Agent Goal Completed:\n\n{}", final_answer);
                    let html = format!(
                        "<p>🎯 <b>Agent Goal Completed</b></p><p>{}</p>",
                        html_escape(&final_answer)
                    );

                    self.deliver_message(p.origin_conversation.as_deref(), &plain, &html)
                        .await;

                    return Ok(());
                }
                StepOutcome::Failed {
                    thought,
                    error,
                    tokens_used,
                } => {
                    total_tokens = total_tokens.saturating_add(tokens_used);
                    let step = AgentStep::new(
                        run.id,
                        current_step_num,
                        thought,
                        None,
                        None,
                        Some(serde_json::json!({ "error": &error })),
                        StepStatus::Failed,
                        tokens_used,
                        now,
                    );
                    self.services
                        .agent
                        .record_step(run.id, &step)
                        .await
                        .map_err(JobError::from)?;
                    self.services
                        .agent
                        .fail_run(run.id, &error, total_tokens)
                        .await
                        .map_err(JobError::from)?;

                    let plain = format!("❌ Agent Goal Failed:\n\n{}", error);
                    let html = format!(
                        "<p>❌ <b>Agent Goal Failed</b></p><p>{}</p>",
                        html_escape(&error)
                    );

                    self.deliver_message(p.origin_conversation.as_deref(), &plain, &html)
                        .await;

                    return Ok(());
                }
            }
        }

        if ctx.cancel.is_cancelled() {
            return Err(JobError::Unavailable(Duration::ZERO));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::jobs::JobId;
    use crate::interface::jobs::fake::FakeOutbound;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn agent_execute_run_handler_handles_terminal_run_gracefully() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_agent_job_1");
        crate::db::pool::seed_account(&pool, &owner).await;

        let outbound = FakeOutbound::connected();
        let engine = ReActEngine::from_proxy(None);
        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            outbound.clone(),
            engine,
            AiEnvConfig::default(),
        );

        let run = services
            .agent
            .start_user_goal(owner, None, "Goal to cancel", None)
            .await
            .unwrap();

        services.agent.cancel_run(&owner, run.id).await.unwrap();

        let ctx = JobContext {
            job_id: JobId(100),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };

        let payload = serde_json::json!({
            "run_id": run.id.as_uuid(),
            "origin_conversation": None::<String>,
        });

        let res = handler.run(&ctx, payload).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn run_continues_after_invalid_call() {
        use crate::llm::test_support::*;
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_agent_job_invalid_call");
        crate::db::pool::seed_account(&pool, &owner).await;
        let broken = serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
            "message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1",
            "type":"function","function":{"name":"add_todo","arguments":"{\"text\": "}}]},
            "finish_reason":"tool_calls"}]});
        let (url, seen) = scripted_replies(vec![broken, text_reply("Could not add it.")]).await;

        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            FakeOutbound::connected(),
            ReActEngine::from_proxy(None),
            test_config(url),
        );
        let run = services
            .agent
            .start_user_goal(owner, None, "Add milk to my todos", None)
            .await
            .unwrap();
        let ctx = JobContext {
            job_id: JobId(101),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        let payload = serde_json::json!({
            "run_id": run.id.as_uuid(),
            "origin_conversation": None::<String>,
        });

        handler.run(&ctx, payload).await.unwrap();

        let steps = services.agent.get_run_steps(&owner, run.id).await.unwrap();
        assert_eq!(steps.len(), 2, "the failed call, then the final answer");
        assert_eq!(steps[0].status, StepStatus::Failed);
        assert!(steps[0].tool_output.as_ref().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("add_todo"));
        assert_eq!(steps[1].status, StepStatus::Success);
        let finished = services.agent.get_run_internal(run.id).await.unwrap();
        assert_eq!(finished.status, RunStatus::Completed);
        let todos = services
            .todos
            .list(&owner, None, None, 10, 0)
            .await
            .unwrap();
        assert!(todos.is_empty(), "the broken call never ran");
        let requests = seen.lock().unwrap();
        let second = requests[1]["messages"].as_array().unwrap();
        assert!(
            second.iter().any(|m| m["role"] == "tool"),
            "the failure is shown as a tool result"
        );
    }
}
