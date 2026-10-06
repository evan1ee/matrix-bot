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
use crate::domain::agent::{pending_approved_call, AgentRunId, AgentStep, RunStatus, StepStatus};
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
            // The user approved a held call: running it is not a model step,
            // so the step budget does not stop it.
            let approved_call = pending_approved_call(&steps);
            if approved_call.is_none() && current_step_num > run.max_steps {
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

            let outcome = match approved_call {
                // Run the stored call itself (no model call, no sensitivity
                // check). It is recorded below as a result step, so the
                // approval marker is no longer last and it never runs twice.
                Some(held) => {
                    self.engine
                        .execute_approved(
                            &self.services,
                            &self.pool,
                            &step_ctx,
                            held.tool_name.clone().unwrap_or_default(),
                            held.tool_input
                                .clone()
                                .unwrap_or_else(|| serde_json::json!({})),
                        )
                        .await
                }
                None => self
                    .engine
                    .evaluate_step(&self.services, &self.pool, &step_ctx)
                    .await
                    .map_err(|e| {
                        JobError::Retryable(anyhow::anyhow!("LLM evaluation error: {e}"))
                    })?,
            };

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

    /// A handler wired to a scripted LLM, with an account and a started run.
    async fn approval_fixture(
        seed: &str,
        replies: Vec<serde_json::Value>,
    ) -> (
        AgentExecuteRunHandler,
        AppServices,
        AccountId,
        AgentRunId,
        crate::llm::test_support::Seen,
    ) {
        use crate::llm::test_support::*;
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let (url, seen) = scripted_replies(replies).await;
        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            FakeOutbound::connected(),
            ReActEngine::from_proxy(None),
            test_config(url),
        );
        let run = services
            .agent
            .start_user_goal(owner, None, "Deposit 100 USD", None)
            .await
            .unwrap();
        (handler, services, owner, run.id, seen)
    }

    async fn run_job(handler: &AgentExecuteRunHandler, owner: AccountId, run_id: AgentRunId) {
        let ctx = JobContext {
            job_id: JobId(200),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        let payload = serde_json::json!({
            "run_id": run_id.as_uuid(),
            "origin_conversation": None::<String>,
        });
        handler.run(&ctx, payload).await.unwrap();
    }

    fn deposit_call() -> serde_json::Value {
        crate::llm::test_support::tool_reply(
            "deposit_portfolio_cash",
            serde_json::json!({"amount": "100", "currency": "USD"}),
        )
    }

    async fn cash_minor(services: &AppServices, owner: &AccountId) -> Vec<i64> {
        services
            .portfolio
            .list_cash(owner)
            .await
            .unwrap()
            .iter()
            .map(|c| c.balance_minor)
            .collect()
    }

    #[tokio::test]
    async fn approved_action_executes_exactly_once_on_resume() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, seen) = approval_fixture(
            "agent-approve-once",
            vec![deposit_call(), text_reply("Deposited.")],
        )
        .await;

        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
        assert_eq!(pending.len(), 1, "the sensitive call is held");
        assert!(
            cash_minor(&services, &owner).await.is_empty(),
            "nothing ran yet"
        );

        services
            .agent
            .approve_action(&owner, pending[0].id)
            .await
            .unwrap();
        run_job(&handler, owner, run_id).await;

        assert_eq!(
            cash_minor(&services, &owner).await,
            vec![10_000],
            "deposited once"
        );
        assert!(
            services
                .agent
                .list_pending_approvals(&owner)
                .await
                .unwrap()
                .is_empty(),
            "no new approval was opened"
        );
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "no model call to re-decide the action"
        );
        let steps = services.agent.get_run_steps(&owner, run_id).await.unwrap();
        let result = steps
            .iter()
            .find(|s| {
                s.tool_name.as_deref() == Some("deposit_portfolio_cash") && s.tool_output.is_some()
            })
            .expect("the executed call is recorded with its result");
        assert_eq!(result.status, StepStatus::Success);
    }

    #[tokio::test]
    async fn rejected_action_is_not_executed() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) = approval_fixture(
            "agent-reject",
            vec![deposit_call(), text_reply("Understood, nothing deposited.")],
        )
        .await;

        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
        services
            .agent
            .reject_action(&owner, pending[0].id, None)
            .await
            .unwrap();
        run_job(&handler, owner, run_id).await;

        assert!(cash_minor(&services, &owner).await.is_empty());
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Completed);
    }

    #[tokio::test]
    async fn double_approve_does_not_execute_twice() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) = approval_fixture(
            "agent-double-approve",
            vec![deposit_call(), text_reply("Deposited.")],
        )
        .await;

        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
        services
            .agent
            .approve_action(&owner, pending[0].id)
            .await
            .unwrap();
        let second = services.agent.approve_action(&owner, pending[0].id).await;
        assert!(second.is_err(), "an approval resolves once");
        run_job(&handler, owner, run_id).await;

        assert_eq!(cash_minor(&services, &owner).await, vec![10_000]);
    }

    #[tokio::test]
    async fn approving_a_cancelled_run_is_refused_and_runs_nothing() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) = approval_fixture(
            "agent-approve-cancelled",
            vec![deposit_call(), text_reply("Deposited.")],
        )
        .await;

        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
        services.agent.cancel_run(&owner, run_id).await.unwrap();

        let approved = services.agent.approve_action(&owner, pending[0].id).await;
        assert!(approved.is_err());
        run_job(&handler, owner, run_id).await;

        assert!(cash_minor(&services, &owner).await.is_empty());
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
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
