//! `agent.execute_run`: executes an autonomous agent run step-by-step.
//!
//! Evaluates agent steps (native tool calling), validates sandboxing, halts on sensitive tools
//! for human approval, and notifies origin conversations via `Outbound`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

mod audio;
mod resume;

use super::{deliverable, message, parse_payload, Outbound};
use crate::application::AppServices;
use crate::config::AiEnvConfig;
use crate::db::DbPool;
use crate::domain::agent::{
    model_steps, step_budget, AgentRunId, AgentStep, RunStatus, StepStatus,
};
use crate::domain::jobs::{AgentExecutePayload, JobType};
use crate::interface::channels::message::{Attachment, Target};
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::llm::agent::{ReActEngine, StepContext, StepOutcome};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::utils::html::html_escape;
use crate::utils::time::now_at_storage_precision;
use resume::Resume;

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

    async fn deliver_message(
        &self,
        conversation: Option<&str>,
        plain: &str,
        html: &str,
        attachment: Option<Attachment>,
    ) {
        let Some(room_str) = conversation else {
            return;
        };
        let Some((target_address, _channel)) = deliverable(room_str) else {
            return;
        };
        let mut msg = message(Target::Conversation(target_address), plain, html);
        if let Some(att) = attachment {
            msg.content.attachments.push(att);
        }
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
        let allowed_tools = self.services.agent.tool_policy(&run);

        let user_config = self
            .services
            .llm
            .get_user_config(&owner)
            .await
            .map_err(JobError::from)?;
        let resolved_config = ResolvedAiConfig::resolve_effective(
            &self.services,
            &self.ai_env_config,
            user_config.as_ref(),
        )
        .await;

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
                // `budget` is `max_steps` plus the steps earned by approved
                // actions, capped at twice `max_steps`.
                let error_msg = format!(
                    "Max steps limit ({budget}, counting the steps granted after approved \
                     actions) reached without completion"
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
                    None,
                )
                .await;
                return Ok(());
            }

            let step_ctx = StepContext {
                account_id: &owner,
                room_id: p.origin_conversation.as_deref().unwrap_or("agent:internal"),
                goal: &run.goal,
                steps: &steps,
                allowed_tools: allowed_tools.as_deref(),
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

                    self.deliver_message(p.origin_conversation.as_deref(), &plain, &html, None)
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

                    let audio_script = audio::extract_audio_script(&final_answer);
                    let cleaned_answer = audio::strip_audio_script(&final_answer);
                    let attachment = if let Some(script) = &audio_script {
                        let task_voice = if let Some(task_uuid) = run.scheduled_task_id {
                            if let Ok(tasks) = self.services.agent.list_scheduled_tasks(&owner).await {
                                tasks
                                    .into_iter()
                                    .find(|t| t.id == task_uuid)
                                    .and_then(|t| {
                                        t.config
                                            .get("voice")
                                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                                    })
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        audio::compile_audio_digest(
                            &self.services,
                            &owner,
                            script,
                            task_voice.as_deref(),
                        )
                        .await
                    } else {
                        None
                    };

                    let plain = format!("🎯 Agent Goal Completed:\n\n{}", cleaned_answer);
                    let html = format!(
                        "<p>🎯 <b>Agent Goal Completed</b></p><p>{}</p>",
                        html_escape(&cleaned_answer)
                    );

                    self.deliver_message(
                        p.origin_conversation.as_deref(),
                        &plain,
                        &html,
                        attachment,
                    )
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

                    self.deliver_message(p.origin_conversation.as_deref(), &plain, &html, None)
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
    async fn repeated_approved_cycles_end_at_the_step_cap() {
        // One model step allowed: each approved execution earns one more step,
        // but never beyond twice the configured steps in total.
        let (handler, services, owner, run_id, _seen) = approval_fixture_with(
            "agent-step-cap",
            vec![deposit_call(), deposit_call(), deposit_call()],
            Some(1),
        )
        .await;

        run_job(&handler, owner, run_id).await;
        for _ in 0..2 {
            let pending = services.agent.list_pending_approvals(&owner).await.unwrap();
            services
                .agent
                .approve_action(&owner, pending[0].id)
                .await
                .unwrap();
            run_job(&handler, owner, run_id).await;
        }

        assert_eq!(
            cash_minor(&services, &owner).await,
            vec![20_000],
            "two approved deposits"
        );
        assert!(services
            .agent
            .list_pending_approvals(&owner)
            .await
            .unwrap()
            .is_empty());
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(
            run.error
                .as_deref()
                .unwrap_or_default()
                .contains("Max steps"),
            "{:?}",
            run.error
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

    /// A scheduled run of `habit_consistency_review`, started the way the
    /// scheduler starts it. That task may only call `get_my_habits` and
    /// `get_daily_log`.
    async fn scheduled_fixture(
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
        services
            .agent
            .schedule_task(
                &owner,
                crate::domain::agent::TaskId::new("habit_consistency_review").unwrap(),
                "0 20 * * 0",
                serde_json::json!({}),
                crate::utils::time::now_at_storage_precision() - chrono::Duration::minutes(1),
            )
            .await
            .unwrap();
        services.agent.enqueue_due_scheduled_tasks().await.unwrap();
        let run = services
            .agent
            .get_active_run(&owner)
            .await
            .unwrap()
            .expect("the scheduler started a run");
        assert!(run.scheduled_task_id.is_some());
        (handler, services, owner, run.id, seen)
    }

    #[tokio::test]
    async fn scheduled_run_cannot_call_a_tool_outside_its_whitelist() {
        use crate::llm::test_support::{text_reply, tool_reply};
        let (handler, services, owner, run_id, seen) = scheduled_fixture(
            "agent-whitelist-fresh",
            vec![
                tool_reply("add_todo", serde_json::json!({"text": "injected"})),
                text_reply("unused"),
            ],
        )
        .await;

        run_job(&handler, owner, run_id).await;

        let todos = services
            .todos
            .list(&owner, None, None, 10, 0)
            .await
            .unwrap();
        assert!(todos.is_empty(), "the call outside the whitelist never ran");
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(
            run.error
                .as_deref()
                .unwrap_or_default()
                .contains("not allowed by task policy"),
            "{:?}",
            run.error
        );
        let requests = seen.lock().unwrap();
        let offered: Vec<&str> = requests[0]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| t["function"]["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            offered
                .iter()
                .all(|n| ["get_my_habits", "get_daily_log", "evaluate_habit_consistency"].contains(n)),
            "only whitelisted tools are offered: {offered:?}"
        );
    }

    #[tokio::test]
    async fn resume_refuses_an_approved_call_outside_the_task_whitelist() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) =
            scheduled_fixture("agent-whitelist-resume", vec![text_reply("unused")]).await;
        // A held, approved deposit, e.g. one approved before the whitelist
        // was enforced.
        let call = serde_json::json!({"amount": "100", "currency": "USD"});
        let held = AgentStep::new(
            run_id,
            1,
            None,
            Some("deposit_portfolio_cash".to_string()),
            Some(call.clone()),
            None,
            StepStatus::Blocked,
            0,
            crate::utils::time::now_at_storage_precision(),
        );
        services.agent.record_step(run_id, &held).await.unwrap();
        let approval = services
            .agent
            .require_approval(run_id, "deposit_portfolio_cash", call)
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
        assert!(
            run.error
                .as_deref()
                .unwrap_or_default()
                .contains("not permitted by task whitelist"),
            "{:?}",
            run.error
        );
    }

    #[tokio::test]
    async fn completed_goal_with_audio_script_compiles_digest_and_cleans_message() {
        use crate::domain::config::SettingValue;
        use crate::interface::channels::message::AttachmentKind;
        use crate::llm::test_support::*;
        use axum::routing::post;

        let fake_audio = b"fake-digest-ogg-audio".to_vec();
        let app = axum::Router::new().route(
            "/audio/speech",
            post({
                let audio = fake_audio.clone();
                move || {
                    let audio = audio.clone();
                    async move {
                        let mut resp = axum::response::Response::new(axum::body::Body::from(audio));
                        resp.headers_mut().insert(
                            "x-duration-ms",
                            axum::http::HeaderValue::from_static("6000"),
                        );
                        resp
                    }
                }
            }),
        );
        let tts_base_url = crate::infrastructure::inference::test_support::spawn_server(app).await;

        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("agent-audio-digest-test");
        crate::db::pool::seed_account(&pool, &owner).await;

        services
            .app_config
            .update_setting(
                &owner,
                crate::domain::config::SettingKey::InferenceTtsBaseUrl,
                SettingValue::String(tts_base_url),
            )
            .await
            .unwrap();

        let outbound = FakeOutbound::connected();
        let final_text = "Weekly Digest Summary:\n\n<audio_script>Welcome to your audio podcast briefing!</audio_script>\n\nHope you enjoyed!";
        let (url, _seen) = scripted_replies(vec![text_reply(final_text)]).await;
        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            outbound.clone(),
            ReActEngine::from_proxy(None),
            test_config(url),
        );

        let run = services
            .agent
            .start_user_goal(owner, None, "Generate weekly digest", None)
            .await
            .unwrap();

        let ctx = JobContext {
            job_id: JobId(301),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        let payload = serde_json::json!({
            "run_id": run.id.as_uuid(),
            "origin_conversation": "!test_room:matrix.org",
        });

        handler.run(&ctx, payload).await.unwrap();

        let msgs = outbound.messages();
        assert_eq!(msgs.len(), 1, "exactly one completion message delivered");
        let msg = &msgs[0];

        // Delivered text must be stripped of audio tags
        assert!(msg.content.plain.contains("Weekly Digest Summary:"));
        assert!(msg.content.plain.contains("Hope you enjoyed!"));
        assert!(!msg.content.plain.contains("<audio_script>"));
        assert!(!msg.content.plain.contains("Welcome to your audio podcast briefing!"));

        // Attachment must be attached
        assert_eq!(msg.content.attachments.len(), 1);
        let att = &msg.content.attachments[0];
        assert_eq!(att.kind, AttachmentKind::Audio);
        assert_eq!(att.mime, "audio/ogg");
        assert_eq!(att.filename, "weekly-digest.ogg");
        assert_eq!(att.bytes, fake_audio);
        assert_eq!(att.duration_ms, Some(6000));
    }

    #[tokio::test]
    async fn completed_goal_with_audio_script_degrades_gracefully_when_tts_unconfigured() {
        use crate::llm::test_support::*;

        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("agent-audio-unconf-test");
        crate::db::pool::seed_account(&pool, &owner).await;

        let outbound = FakeOutbound::connected();
        let final_text = "Here is the news:\n\n<audio_script>Voice digest.</audio_script>\n\nEnd of news.";
        let (url, _seen) = scripted_replies(vec![text_reply(final_text)]).await;
        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            outbound.clone(),
            ReActEngine::from_proxy(None),
            test_config(url),
        );

        let run = services
            .agent
            .start_user_goal(owner, None, "Generate news", None)
            .await
            .unwrap();

        let ctx = JobContext {
            job_id: JobId(302),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        let payload = serde_json::json!({
            "run_id": run.id.as_uuid(),
            "origin_conversation": "!test_room2:matrix.org",
        });

        handler.run(&ctx, payload).await.unwrap();

        let msgs = outbound.messages();
        assert_eq!(msgs.len(), 1);
        let msg = &msgs[0];

        // Delivered text must be stripped of audio tags
        assert!(msg.content.plain.contains("Here is the news:"));
        assert!(msg.content.plain.contains("End of news."));
        assert!(!msg.content.plain.contains("<audio_script>"));
        assert!(!msg.content.plain.contains("Voice digest."));

        // No attachment because TTS unconfigured
        assert_eq!(msg.content.attachments.len(), 0);
    }

    #[tokio::test]
    async fn completed_scheduled_task_with_voice_config_uses_task_voice() {
        use crate::domain::agent::TaskId;
        use crate::domain::config::SettingValue;
        use crate::llm::test_support::*;
        use axum::routing::post;
        use std::sync::{Arc, Mutex};

        let captured_voice = Arc::new(Mutex::new(None::<String>));
        let fake_audio = b"fake-digest-ogg-audio".to_vec();
        let app = axum::Router::new().route(
            "/audio/speech",
            post({
                let audio = fake_audio.clone();
                let captured = captured_voice.clone();
                move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let audio = audio.clone();
                    let captured = captured.clone();
                    async move {
                        if let Some(v) = body.get("voice").and_then(|v| v.as_str()) {
                            *captured.lock().unwrap() = Some(v.to_string());
                        }
                        let mut resp = axum::response::Response::new(axum::body::Body::from(audio));
                        resp.headers_mut().insert(
                            "x-duration-ms",
                            axum::http::HeaderValue::from_static("3000"),
                        );
                        resp
                    }
                }
            }),
        );
        let tts_base_url = crate::infrastructure::inference::test_support::spawn_server(app).await;

        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("agent-task-voice-test");
        crate::db::pool::seed_account(&pool, &owner).await;

        services
            .app_config
            .update_setting(
                &owner,
                crate::domain::config::SettingKey::InferenceTtsBaseUrl,
                SettingValue::String(tts_base_url),
            )
            .await
            .unwrap();

        let outbound = FakeOutbound::connected();
        let final_text = "Weekly Digest:\n\n<audio_script>Voice digest with custom voice.</audio_script>\n\nDone.";
        let (url, _seen) = scripted_replies(vec![text_reply(final_text)]).await;
        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            outbound.clone(),
            ReActEngine::from_proxy(None),
            test_config(url),
        );

        let task_def = services
            .agent
            .schedule_task(
                &owner,
                TaskId::new("weekly_bookmark_digest").unwrap(),
                "0 9 * * 1",
                serde_json::json!({ "voice": "special-digest-voice" }),
                crate::utils::time::now_at_storage_precision() - chrono::Duration::minutes(1),
            )
            .await
            .unwrap();

        services.agent.enqueue_due_scheduled_tasks().await.unwrap();
        let run = services
            .agent
            .get_active_run(&owner)
            .await
            .unwrap()
            .expect("due scheduled task started a run");
        assert_eq!(run.scheduled_task_id, Some(task_def.id));

        let ctx = JobContext {
            job_id: JobId(303),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        };
        let payload = serde_json::json!({
            "run_id": run.id.as_uuid(),
            "origin_conversation": "!test_room3:matrix.org",
        });

        handler.run(&ctx, payload).await.unwrap();

        let voice = captured_voice.lock().unwrap().clone();
        assert_eq!(voice, Some("special-digest-voice".to_string()));

        let msgs = outbound.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content.attachments.len(), 1);
    }
}
