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
use crate::domain::accounts::AccountId;
use crate::domain::agent::{
    interrupted_approved_execution, model_steps, pending_approved_call, step_budget, AgentRun,
    AgentRunId, AgentStep, RunStatus, StepStatus,
};
use crate::domain::jobs::{AgentExecutePayload, JobType};
use crate::interface::channels::message::Target;
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::llm::agent::{ReActEngine, StepContext, StepOutcome};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::utils::html::html_escape;
use crate::utils::time::now_at_storage_precision;

/// What `resume_approved` did, so the run loop knows how to go on.
enum Resume {
    /// No approved call is waiting: evaluate the next model step.
    NotApplicable,
    /// An approved call ran and its result is recorded: read the steps again.
    Continue,
    /// The run was failed (or is already over): stop.
    Stop,
}

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

    /// Fails the run with a recorded step and tells the origin conversation.
    async fn stop_run(
        &self,
        run: &AgentRun,
        payload: &AgentExecutePayload,
        step_number: i32,
        error: &str,
        total_tokens: i32,
    ) -> Result<(), JobError> {
        let step = AgentStep::new(
            run.id,
            step_number,
            None,
            None,
            None,
            Some(serde_json::json!({ "error": error })),
            StepStatus::Failed,
            0,
            now_at_storage_precision(),
        );
        self.services
            .agent
            .record_step(run.id, &step)
            .await
            .map_err(JobError::from)?;
        self.services
            .agent
            .fail_run(run.id, error, total_tokens)
            .await
            .map_err(JobError::from)?;
        self.deliver_message(
            payload.origin_conversation.as_deref(),
            &format!("❌ Agent Goal Failed:\n\n{error}"),
            &format!(
                "<p>❌ <b>Agent Goal Failed</b></p><p>{}</p>",
                html_escape(error)
            ),
        )
        .await;
        Ok(())
    }

    /// Runs the call the user approved, at most once.
    ///
    /// Order matters: a "started" step is committed BEFORE the tool runs, and
    /// the result step is recorded after. A retry that finds the started step
    /// last cannot tell whether the tool ran, so it fails the run for the user
    /// to check instead of running the tool again. The call comes from the
    /// approval the user saw (carried by the marker) and must equal the held
    /// step; the model is not asked and no sensitivity check applies.
    async fn resume_approved(
        &self,
        owner: &AccountId,
        run: &AgentRun,
        payload: &AgentExecutePayload,
        steps: &[AgentStep],
        config: &ResolvedAiConfig,
        total_tokens: i32,
    ) -> Result<Resume, JobError> {
        let next = steps.len() as i32 + 1;
        if interrupted_approved_execution(steps) {
            let error = "An approved action may have run, but its result was not recorded. \
                         Please check manually whether it took effect before repeating it.";
            self.stop_run(run, payload, next, error, total_tokens)
                .await?;
            return Ok(Resume::Stop);
        }
        let Some((held, marker)) = pending_approved_call(steps) else {
            return Ok(Resume::NotApplicable);
        };
        let held_call = (
            held.tool_name.clone().unwrap_or_default(),
            held.tool_input
                .clone()
                .unwrap_or_else(|| serde_json::json!({})),
        );
        if marker.approved_call().as_ref() != Some(&held_call) {
            let error = "The approved action does not match the held call; nothing was run.";
            self.stop_run(run, payload, next, error, total_tokens)
                .await?;
            return Ok(Resume::Stop);
        }
        // Cancelled since this job was queued: the held action must not run.
        let current = self
            .services
            .agent
            .get_run_internal(run.id)
            .await
            .map_err(JobError::from)?;
        if current.status.is_terminal() {
            return Ok(Resume::Stop);
        }

        let started = AgentStep::execution_started(run.id, next, now_at_storage_precision());
        self.services
            .agent
            .record_step(run.id, &started)
            .await
            .map_err(JobError::from)?;

        let step_ctx = StepContext {
            account_id: owner,
            room_id: payload
                .origin_conversation
                .as_deref()
                .unwrap_or("agent:internal"),
            goal: &run.goal,
            steps,
            allowed_tools: None,
            custom_sensitive_tools: None,
            max_steps: run.max_steps,
            config,
        };
        let outcome = self
            .engine
            .execute_approved(
                &self.services,
                &self.pool,
                &step_ctx,
                held_call.0,
                held_call.1,
            )
            .await;
        match outcome {
            StepOutcome::Executed {
                thought,
                tool_name,
                tool_input,
                tool_output,
                tokens_used,
            } => {
                let step = AgentStep::new(
                    run.id,
                    next + 1,
                    thought,
                    Some(tool_name),
                    Some(tool_input),
                    Some(tool_output),
                    StepStatus::Success,
                    tokens_used,
                    now_at_storage_precision(),
                );
                self.services
                    .agent
                    .record_step(run.id, &step)
                    .await
                    .map_err(JobError::from)?;
                Ok(Resume::Continue)
            }
            other => {
                let error = match other {
                    StepOutcome::Failed { error, .. } => error,
                    _ => "The approved action could not be run".to_string(),
                };
                self.stop_run(run, payload, next + 1, &error, total_tokens)
                    .await?;
                Ok(Resume::Stop)
            }
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

            // An approved call (or one interrupted mid-run) is handled before
            // anything else: it is not a model step, so the budget does not
            // stop it, and the model is not asked to decide it again.
            match self
                .resume_approved(&owner, &run, &p, &steps, &resolved_config, total_tokens)
                .await?
            {
                Resume::NotApplicable => {}
                Resume::Continue => continue,
                Resume::Stop => return Ok(()),
            }

            // Only the model's own steps use the budget; the markers and the
            // result of an approved call do not (see `step_budget`).
            let budget = step_budget(&steps, run.max_steps);
            if model_steps(&steps) as i32 + 1 > budget {
                let error_msg = format!("Max steps limit ({budget}) reached without completion");
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
                max_steps: budget,
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
        approval_fixture_with(seed, replies, None).await
    }

    async fn approval_fixture_with(
        seed: &str,
        replies: Vec<serde_json::Value>,
        max_steps: Option<i32>,
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
            .start_user_goal(owner, None, "Deposit 100 USD", max_steps)
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
    async fn approved_action_runs_once_without_asking_the_model_again() {
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
    async fn overlapping_approve_and_cancel_never_run_the_action() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) = approval_fixture(
            "agent-approve-cancel-race",
            vec![deposit_call(), text_reply("Deposited.")],
        )
        .await;
        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();

        let (approved, cancelled) = tokio::join!(
            services.agent.approve_action(&owner, pending[0].id),
            services.agent.cancel_run(&owner, run_id),
        );
        // Whichever commits first, a cancelled run stays cancelled and its
        // held action never runs.
        let _ = (approved, cancelled);
        run_job(&handler, owner, run_id).await;

        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
        assert!(cash_minor(&services, &owner).await.is_empty());
    }

    #[tokio::test]
    async fn concurrent_double_approve_runs_the_action_once() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) = approval_fixture(
            "agent-concurrent-approve",
            vec![deposit_call(), text_reply("Deposited.")],
        )
        .await;
        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();

        let (first, second) = tokio::join!(
            services.agent.approve_action(&owner, pending[0].id),
            services.agent.approve_action(&owner, pending[0].id),
        );

        assert_eq!(
            [first.is_ok(), second.is_ok()]
                .iter()
                .filter(|ok| **ok)
                .count(),
            1,
            "exactly one approval wins: {first:?} {second:?}"
        );
        run_job(&handler, owner, run_id).await;
        assert_eq!(cash_minor(&services, &owner).await, vec![10_000]);
    }

    #[tokio::test]
    async fn an_approved_call_at_the_last_step_still_gets_a_final_answer() {
        use crate::llm::test_support::{text_reply, tool_reply};
        // Two model steps allowed: a read, then the held deposit.
        let (handler, services, owner, run_id, _seen) = approval_fixture_with(
            "agent-budget-gap",
            vec![
                tool_reply("get_my_tasks", serde_json::json!({})),
                deposit_call(),
                text_reply("Deposited 100 USD."),
            ],
            Some(2),
        )
        .await;

        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
        services
            .agent
            .approve_action(&owner, pending[0].id)
            .await
            .unwrap();
        run_job(&handler, owner, run_id).await;

        assert_eq!(cash_minor(&services, &owner).await, vec![10_000]);
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
        let steps = services.agent.get_run_steps(&owner, run_id).await.unwrap();
        let last = steps.last().unwrap();
        assert_eq!(
            last.tool_output.as_ref().unwrap()["final_answer"],
            "Deposited 100 USD."
        );
    }

    #[tokio::test]
    async fn a_retry_after_a_failed_result_record_does_not_run_the_tool_twice() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) = approval_fixture(
            "agent-at-most-once",
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

        // Only an event that names a tool fails, so the "started" step is
        // recorded and the result step is not: a crash after the tool ran.
        sqlx::raw_sql(
            "CREATE FUNCTION fail_result_event() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN IF COALESCE(NEW.payload->'payload'->>'tool_name', NEW.payload->>'tool_name') IS NOT NULL THEN \
             RAISE EXCEPTION 'result record forced to fail'; END IF; RETURN NEW; END $$; \
             CREATE TRIGGER fail_result_event BEFORE INSERT ON outbox_events \
             FOR EACH ROW EXECUTE FUNCTION fail_result_event()",
        )
        .execute(&handler.pool)
        .await
        .unwrap();
        let ctx = JobContext {
            job_id: JobId(201),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        let payload = serde_json::json!({
            "run_id": run_id.as_uuid(),
            "origin_conversation": None::<String>,
        });
        assert!(
            handler.run(&ctx, payload).await.is_err(),
            "the result could not be recorded"
        );
        assert_eq!(
            cash_minor(&services, &owner).await,
            vec![10_000],
            "the tool ran once"
        );

        sqlx::raw_sql("DROP TRIGGER fail_result_event ON outbox_events")
            .execute(&handler.pool)
            .await
            .unwrap();
        run_job(&handler, owner, run_id).await;

        assert_eq!(
            cash_minor(&services, &owner).await,
            vec![10_000],
            "never run twice"
        );
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(
            run.error.as_deref().unwrap_or_default().contains("check"),
            "the user is told to check manually: {:?}",
            run.error
        );
    }

    #[tokio::test]
    async fn a_marker_that_does_not_match_the_held_step_runs_nothing() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) =
            approval_fixture("agent-marker-mismatch", vec![text_reply("unused")]).await;
        // A held step for 100, but the approval the user saw is for 999.
        let held = AgentStep::new(
            run_id,
            1,
            None,
            Some("deposit_portfolio_cash".to_string()),
            Some(serde_json::json!({"amount": "100", "currency": "USD"})),
            None,
            StepStatus::Blocked,
            0,
            now_at_storage_precision(),
        );
        services.agent.record_step(run_id, &held).await.unwrap();
        let approval = services
            .agent
            .require_approval(
                run_id,
                "deposit_portfolio_cash",
                serde_json::json!({"amount": "999", "currency": "USD"}),
            )
            .await
            .unwrap();
        services
            .agent
            .approve_action(&owner, approval.id)
            .await
            .unwrap();

        run_job(&handler, owner, run_id).await;

        assert!(
            cash_minor(&services, &owner).await.is_empty(),
            "nothing ran"
        );
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
    }

    async fn step_events_after_deciding(seed: &str, approve: bool) -> (usize, usize) {
        use crate::llm::test_support::text_reply;
        let count =
            |events: Vec<String>| events.iter().filter(|e| *e == "AgentStepCompleted").count();
        let (handler, services, owner, run_id, _seen) =
            approval_fixture(seed, vec![deposit_call(), text_reply("ok")]).await;
        run_job(&handler, owner, run_id).await;
        let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
        let events = || {
            crate::infrastructure::postgres::repositories::outbox::event_types_for(
                &handler.pool,
                &owner,
            )
        };
        let before = count(events().await.unwrap());
        if approve {
            services
                .agent
                .approve_action(&owner, pending[0].id)
                .await
                .unwrap();
        } else {
            services
                .agent
                .reject_action(&owner, pending[0].id, None)
                .await
                .unwrap();
        }
        (before, count(events().await.unwrap()))
    }

    #[tokio::test]
    async fn approving_publishes_a_step_event() {
        let (before, after) = step_events_after_deciding("agent-step-events-approve", true).await;
        assert_eq!(after, before + 1, "the approval marker is announced");
    }

    #[tokio::test]
    async fn rejecting_publishes_a_step_event() {
        let (before, after) = step_events_after_deciding("agent-step-events-reject", false).await;
        assert_eq!(after, before + 1, "the rejection step is announced");
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
