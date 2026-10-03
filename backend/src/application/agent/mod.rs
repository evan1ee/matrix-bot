//! Application service for the autonomous agent subsystem.
//!
//! Owns transaction boundaries for starting agent runs, executing ReAct steps,
//! resolving human-in-the-loop approvals, managing task schedules, and publishing
//! transactional outbox events.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::agent::{
    AgentError, AgentRun, AgentRunId, AgentStep, ApprovalId, ApprovalRequest, ApprovalStatus,
    RunStatus, ScheduledTaskDefinition, ScheduledTaskRegistry, StepStatus, TaskId,
};
use crate::domain::events::SystemEvent;
use crate::infrastructure::postgres::repositories;
use crate::utils::time::now_at_storage_precision;

impl From<AgentError> for AppError {
    fn from(err: AgentError) -> Self {
        match err {
            AgentError::InvalidGoal(msg) => AppError::Validation(msg),
            AgentError::InvalidTaskId(msg) => AppError::Validation(msg),
            AgentError::InvalidStatus(msg) => AppError::Validation(msg),
            AgentError::InvalidStatusTransition { from, to } => {
                AppError::Conflict(format!("cannot transition run from {from} to {to}"))
            }
            AgentError::NonMonotonicStep { expected, got } => {
                AppError::Validation(format!("step out of order: expected {expected}, got {got}"))
            }
            AgentError::StepLimitExceeded { max, attempted } => {
                AppError::Validation(format!("step limit {max} exceeded: attempted {attempted}"))
            }
            AgentError::ApprovalExpired => {
                AppError::Validation("approval request has expired".to_string())
            }
            AgentError::ApprovalAlreadyResolved => {
                AppError::Conflict("approval request has already been resolved".to_string())
            }
            AgentError::ActiveRunConflict => AppError::Conflict(
                "an active agent run is already in progress for this account".to_string(),
            ),
            AgentError::RunNotFound => AppError::NotFound("agent run not found".to_string()),
            AgentError::ApprovalNotFound => {
                AppError::NotFound("approval request not found".to_string())
            }
        }
    }
}

/// Use cases for the autonomous agent runtime.
#[derive(Clone)]
pub struct AgentService {
    pool: PgPool,
    registry: Arc<ScheduledTaskRegistry>,
}

impl AgentService {
    /// Constructs a new `AgentService` with pre-loaded built-in tasks.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            registry: Arc::new(ScheduledTaskRegistry::with_builtins()),
        }
    }

    /// Constructs an `AgentService` with a custom registry.
    pub fn with_registry(pool: PgPool, registry: Arc<ScheduledTaskRegistry>) -> Self {
        Self { pool, registry }
    }

    /// Returns a reference to the scheduled task plugin registry.
    pub fn registry(&self) -> &ScheduledTaskRegistry {
        &self.registry
    }

    /// Starts a user-triggered asynchronous goal.
    ///
    /// Validates the goal, ensures no active run currently exists for this account,
    /// persists the initial `AgentRun`, enqueues the background job `agent.execute_run`,
    /// and publishes `SystemEvent::AgentRunStarted` in one atomic transaction.
    pub async fn start_user_goal(
        &self,
        account_id: AccountId,
        origin_conversation: Option<String>,
        goal: impl Into<String>,
        max_steps: Option<i32>,
    ) -> Result<AgentRun, AppError> {
        let now = now_at_storage_precision();
        let run = AgentRun::new(
            account_id,
            "custom_goal",
            goal,
            origin_conversation.clone(),
            max_steps,
            now,
        )?;

        let mut tx = self.pool.begin().await?;

        // Invariant: Max 1 active run per account
        if let Some(_active) =
            repositories::agent::find_active_run_for_account(&mut *tx, &account_id).await?
        {
            return Err(AppError::Conflict(
                "an active agent run is already in progress for this account".to_string(),
            ));
        }

        let created_run = repositories::agent::create_run(&mut *tx, &run).await?;

        // Enqueue background durable job
        let job = crate::domain::jobs::NewJob::agent_execute(
            account_id,
            *created_run.id.as_uuid(),
            origin_conversation,
        );
        repositories::jobs::enqueue(&mut *tx, &job).await?;

        // Publish event to transactional outbox
        let event = SystemEvent::AgentRunStarted {
            owner_id: account_id,
            run_id: created_run.id.to_string(),
            task_name: created_run.task_name.clone(),
            goal: created_run.goal.clone(),
        };
        repositories::outbox::publish(&mut tx, &event).await?;

        tx.commit().await?;

        Ok(created_run)
    }

    /// Returns the currently active run for the account, if any.
    pub async fn get_active_run(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<AgentRun>, AppError> {
        let run = repositories::agent::find_active_run_for_account(&self.pool, account_id).await?;
        Ok(run)
    }

    /// Fetches a specific run by ID for the authenticated account.
    pub async fn get_run(
        &self,
        account_id: &AccountId,
        run_id: AgentRunId,
    ) -> Result<AgentRun, AppError> {
        repositories::agent::find_run_for_account(&self.pool, account_id, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))
    }

    /// Fetches a specific run by ID without checking account ownership (internal/job use only).
    pub async fn get_run_internal(&self, run_id: AgentRunId) -> Result<AgentRun, AppError> {
        repositories::agent::find_run_by_id(&self.pool, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))
    }

    /// Lists recent runs for an account.
    pub async fn list_runs(
        &self,
        account_id: &AccountId,
        limit: i64,
    ) -> Result<Vec<AgentRun>, AppError> {
        let limit = limit.clamp(1, 100);
        let runs =
            repositories::agent::list_runs_for_account(&self.pool, account_id, limit).await?;
        Ok(runs)
    }

    /// Fetches all recorded steps for a given run.
    pub async fn get_run_steps(
        &self,
        account_id: &AccountId,
        run_id: AgentRunId,
    ) -> Result<Vec<AgentStep>, AppError> {
        // Enforce ownership
        let _ = self.get_run(account_id, run_id).await?;
        let steps = repositories::agent::get_run_steps(&self.pool, run_id).await?;
        Ok(steps)
    }

    /// Records a ReAct step execution in transaction.
    pub async fn record_step(
        &self,
        run_id: AgentRunId,
        step: &AgentStep,
    ) -> Result<AgentStep, AppError> {
        let mut tx = self.pool.begin().await?;
        let recorded = repositories::agent::record_step(&mut *tx, step).await?;

        let run = repositories::agent::find_run_by_id(&mut *tx, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        let event = SystemEvent::AgentStepCompleted {
            owner_id: run.account_id,
            run_id: run_id.to_string(),
            step_number: step.step_number,
            thought: step.thought.clone(),
            tool_name: step.tool_name.clone(),
            status: step.status.to_string(),
        };
        repositories::outbox::publish(&mut tx, &event).await?;
        tx.commit().await?;

        Ok(recorded)
    }

    /// Creates an approval request and halts run execution into `WaitingApproval`.
    pub async fn require_approval(
        &self,
        run_id: AgentRunId,
        tool_name: impl Into<String>,
        tool_input: serde_json::Value,
    ) -> Result<ApprovalRequest, AppError> {
        let now = now_at_storage_precision();
        let tool_name = tool_name.into();

        let mut tx = self.pool.begin().await?;

        let mut run = repositories::agent::find_run_by_id(&mut *tx, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        run.mark_waiting_approval(now)?;
        repositories::agent::update_run_status(
            &mut *tx,
            run_id,
            RunStatus::WaitingApproval,
            run.current_step,
            run.total_tokens,
            None,
            None,
            now,
        )
        .await?;

        let approval = ApprovalRequest::new(
            run_id,
            run.account_id,
            tool_name.clone(),
            tool_input.clone(),
            now,
        );
        let created = repositories::agent::create_approval(&mut *tx, &approval).await?;

        let event = SystemEvent::AgentApprovalRequired {
            owner_id: run.account_id,
            run_id: run_id.to_string(),
            approval_id: created.id.to_string(),
            tool_name,
            tool_input,
        };
        repositories::outbox::publish(&mut tx, &event).await?;

        tx.commit().await?;

        Ok(created)
    }

    /// Approves a pending sensitive action, transitioning the run back to `Executing`
    /// and enqueuing a job to resume execution.
    pub async fn approve_action(
        &self,
        account_id: &AccountId,
        approval_id: ApprovalId,
    ) -> Result<(), AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        let mut approval = repositories::agent::find_pending_approval_for_account(
            &mut *tx,
            account_id,
            approval_id,
        )
        .await?
        .ok_or(AppError::NotFound(
            "pending approval request not found".to_string(),
        ))?;

        approval.approve(now)?;
        repositories::agent::resolve_approval(&mut *tx, approval_id, ApprovalStatus::Approved, now)
            .await?;

        let mut run = repositories::agent::find_run_by_id(&mut *tx, approval.run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        run.status = RunStatus::Executing;
        repositories::agent::update_run_status(
            &mut *tx,
            run.id,
            RunStatus::Executing,
            run.current_step,
            run.total_tokens,
            None,
            None,
            now,
        )
        .await?;

        // Re-enqueue job to resume execution immediately
        let job = crate::domain::jobs::NewJob::agent_execute(
            *account_id,
            *run.id.as_uuid(),
            run.origin_conversation,
        );
        repositories::jobs::enqueue(&mut *tx, &job).await?;

        tx.commit().await?;

        Ok(())
    }

    /// Rejects a pending sensitive action, recording the rejection observation
    /// and re-enqueueing the run to let the agent formulate an alternative plan.
    pub async fn reject_action(
        &self,
        account_id: &AccountId,
        approval_id: ApprovalId,
        reason: Option<&str>,
    ) -> Result<(), AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        let mut approval = repositories::agent::find_pending_approval_for_account(
            &mut *tx,
            account_id,
            approval_id,
        )
        .await?
        .ok_or(AppError::NotFound(
            "pending approval request not found".to_string(),
        ))?;

        approval.reject(now)?;
        repositories::agent::resolve_approval(&mut *tx, approval_id, ApprovalStatus::Rejected, now)
            .await?;

        let mut run = repositories::agent::find_run_by_id(&mut *tx, approval.run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        run.status = RunStatus::Executing;
        repositories::agent::update_run_status(
            &mut *tx,
            run.id,
            RunStatus::Executing,
            run.current_step,
            run.total_tokens,
            None,
            None,
            now,
        )
        .await?;

        // Record a step with the rejection
        let reject_reason = reason.unwrap_or("Action rejected by user");
        let step = AgentStep::new(
            run.id,
            run.current_step + 1,
            Some("Action was rejected by user, reconsidering plan".to_string()),
            Some(approval.tool_name.clone()),
            Some(approval.tool_input.clone()),
            Some(json!({ "rejected": true, "reason": reject_reason })),
            StepStatus::Blocked,
            0,
            now,
        );
        repositories::agent::record_step(&mut *tx, &step).await?;

        let job = crate::domain::jobs::NewJob::agent_execute(
            *account_id,
            *run.id.as_uuid(),
            run.origin_conversation,
        );
        repositories::jobs::enqueue(&mut *tx, &job).await?;

        tx.commit().await?;

        Ok(())
    }

    /// Cancels an active agent run.
    pub async fn cancel_run(
        &self,
        account_id: &AccountId,
        run_id: AgentRunId,
    ) -> Result<(), AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        let mut run = repositories::agent::find_run_for_account(&mut *tx, account_id, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        run.mark_cancelled(now)?;
        repositories::agent::update_run_status(
            &mut *tx,
            run_id,
            RunStatus::Cancelled,
            run.current_step,
            run.total_tokens,
            run.error.as_deref(),
            Some(now),
            now,
        )
        .await?;

        let event = SystemEvent::AgentRunCompleted {
            owner_id: *account_id,
            run_id: run_id.to_string(),
            status: RunStatus::Cancelled.to_string(),
            total_tokens: run.total_tokens,
            error: run.error.clone(),
        };
        repositories::outbox::publish(&mut tx, &event).await?;

        tx.commit().await?;

        Ok(())
    }

    /// Marks a run completed successfully.
    pub async fn complete_run(
        &self,
        run_id: AgentRunId,
        total_tokens: i32,
    ) -> Result<(), AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        let mut run = repositories::agent::find_run_by_id(&mut *tx, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        run.mark_completed(now)?;
        repositories::agent::update_run_status(
            &mut *tx,
            run_id,
            RunStatus::Completed,
            run.current_step,
            total_tokens,
            None,
            Some(now),
            now,
        )
        .await?;

        let event = SystemEvent::AgentRunCompleted {
            owner_id: run.account_id,
            run_id: run_id.to_string(),
            status: RunStatus::Completed.to_string(),
            total_tokens,
            error: None,
        };
        repositories::outbox::publish(&mut tx, &event).await?;

        tx.commit().await?;

        Ok(())
    }

    /// Marks a run failed with an error message.
    pub async fn fail_run(
        &self,
        run_id: AgentRunId,
        error: impl Into<String>,
        total_tokens: i32,
    ) -> Result<(), AppError> {
        let now = now_at_storage_precision();
        let error = error.into();
        let mut tx = self.pool.begin().await?;

        let mut run = repositories::agent::find_run_by_id(&mut *tx, run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;

        run.mark_failed(error.clone(), now)?;
        repositories::agent::update_run_status(
            &mut *tx,
            run_id,
            RunStatus::Failed,
            run.current_step,
            total_tokens,
            Some(&error),
            Some(now),
            now,
        )
        .await?;

        let event = SystemEvent::AgentRunCompleted {
            owner_id: run.account_id,
            run_id: run_id.to_string(),
            status: RunStatus::Failed.to_string(),
            total_tokens,
            error: Some(error),
        };
        repositories::outbox::publish(&mut tx, &event).await?;

        tx.commit().await?;

        Ok(())
    }

    /// Lists pending approvals for an account.
    pub async fn list_pending_approvals(
        &self,
        account_id: &AccountId,
    ) -> Result<Vec<ApprovalRequest>, AppError> {
        let approvals =
            repositories::agent::list_pending_approvals_for_account(&self.pool, account_id).await?;
        Ok(approvals)
    }

    /// Upserts a scheduled recurring task definition.
    pub async fn schedule_task(
        &self,
        account_id: &AccountId,
        task_id: TaskId,
        cron_expression: impl Into<String>,
        config: serde_json::Value,
        next_run_at: DateTime<Utc>,
    ) -> Result<ScheduledTaskDefinition, AppError> {
        let now = now_at_storage_precision();
        let task = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: *account_id,
            task_id,
            cron_expression: cron_expression.into(),
            enabled: true,
            config,
            last_run_at: None,
            next_run_at,
            created_at: now,
            updated_at: now,
        };

        let saved = repositories::agent::upsert_scheduled_task(&self.pool, &task).await?;
        Ok(saved)
    }

    /// Lists scheduled tasks for an account.
    pub async fn list_scheduled_tasks(
        &self,
        account_id: &AccountId,
    ) -> Result<Vec<ScheduledTaskDefinition>, AppError> {
        let tasks =
            repositories::agent::list_scheduled_tasks_for_account(&self.pool, account_id).await?;
        Ok(tasks)
    }

    /// Sweeps due scheduled tasks and enqueues execution runs.
    pub async fn enqueue_due_scheduled_tasks(&self) -> Result<usize, AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        let due_tasks = repositories::agent::list_due_scheduled_tasks(&mut *tx, now, 10).await?;
        let count = due_tasks.len();

        for task in due_tasks {
            // Check if account already has an active run
            if let Some(_active) =
                repositories::agent::find_active_run_for_account(&mut *tx, &task.account_id).await?
            {
                // Skip this tick to respect the 1 active run limit
                continue;
            }

            let goal = if let Some(plugin) = self.registry.get(&task.task_id) {
                plugin.formulate_goal(&task.config)
            } else {
                format!("Scheduled execution for task '{}'", task.task_id)
            };

            let run = AgentRun {
                id: AgentRunId::generate(),
                scheduled_task_id: Some(task.id),
                account_id: task.account_id,
                task_name: task.task_id.to_string(),
                goal,
                origin_conversation: None,
                status: RunStatus::Planning,
                max_steps: 15,
                current_step: 0,
                total_tokens: 0,
                error: None,
                started_at: now,
                completed_at: None,
                updated_at: now,
            };

            let created_run = repositories::agent::create_run(&mut *tx, &run).await?;

            let job = crate::domain::jobs::NewJob::agent_execute(
                task.account_id,
                *created_run.id.as_uuid(),
                None,
            );
            repositories::jobs::enqueue(&mut *tx, &job).await?;

            // Push next run ahead by 1 hour placeholder until cron parser computes next instant
            let next_run = now + chrono::Duration::hours(1);
            repositories::agent::update_task_next_run(&mut *tx, task.id, now, next_run, now)
                .await?;
        }

        tx.commit().await?;

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn start_goal_enforces_single_active_run_constraint() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_srv_1");
        crate::db::pool::seed_account(&pool, &owner).await;

        let service = AgentService::new(pool.clone());

        // First run succeeds
        let run1 = service
            .start_user_goal(
                owner,
                Some("matrix:!room:server".to_string()),
                "First goal",
                None,
            )
            .await
            .unwrap();
        assert_eq!(run1.goal, "First goal");

        // Concurrent second run fails with Conflict
        let err = service
            .start_user_goal(owner, None, "Second concurrent goal", None)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Conflict(_)));

        // Once first run completes, new run is allowed
        service.complete_run(run1.id, 250).await.unwrap();

        let run2 = service
            .start_user_goal(owner, None, "Second goal allowed now", None)
            .await
            .unwrap();
        assert_eq!(run2.goal, "Second goal allowed now");
    }

    #[tokio::test]
    async fn approval_flow_pauses_and_resumes_run() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_srv_2");
        crate::db::pool::seed_account(&pool, &owner).await;

        let service = AgentService::new(pool.clone());

        let run = service
            .start_user_goal(owner, None, "Financial rebalance", None)
            .await
            .unwrap();

        // Require approval
        let approval = service
            .require_approval(
                run.id,
                "portfolio_execute_trade",
                json!({"symbol": "ETH", "quantity": 2}),
            )
            .await
            .unwrap();

        let in_flight = service.get_run(&owner, run.id).await.unwrap();
        assert_eq!(in_flight.status, RunStatus::WaitingApproval);

        // Approve
        service.approve_action(&owner, approval.id).await.unwrap();

        let resumed = service.get_run(&owner, run.id).await.unwrap();
        assert_eq!(resumed.status, RunStatus::Executing);
    }
}
