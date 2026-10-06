use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::agent::{AgentRun, AgentRunId, AgentStep, RunStatus};
use crate::domain::events::SystemEvent;
use crate::infrastructure::postgres::repositories;
use crate::utils::time::now_at_storage_precision;

use super::AgentService;

impl AgentService {
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

    /// Cancels an active agent run.
    pub async fn cancel_run(
        &self,
        account_id: &AccountId,
        run_id: AgentRunId,
    ) -> Result<(), AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        // Lock before reading the status, so a concurrent approve or reject
        // finishes first and cancel sees (and overwrites) its result.
        let mut run = repositories::agent::lock_run(&mut *tx, run_id)
            .await?
            .filter(|run| run.account_id == *account_id)
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

        let mut run = repositories::agent::lock_run(&mut *tx, run_id)
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

        let mut run = repositories::agent::lock_run(&mut *tx, run_id)
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
}
