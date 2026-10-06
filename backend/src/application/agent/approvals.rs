use serde_json::json;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::agent::{
    AgentRunId, AgentStep, ApprovalId, ApprovalRequest, ApprovalStatus, RunStatus, StepStatus,
};
use crate::domain::events::SystemEvent;
use crate::infrastructure::postgres::repositories;
use crate::utils::time::now_at_storage_precision;

use super::AgentService;

fn ensure_waiting_for_approval(run: &crate::domain::agent::AgentRun) -> Result<(), AppError> {
    if run.status == RunStatus::WaitingApproval {
        Ok(())
    } else {
        Err(AppError::Conflict(format!(
            "the agent run is {}, not waiting for approval",
            run.status
        )))
    }
}

/// Resolves a pending approval, and fails if it was no longer pending: with
/// the run locked this means another decision won, so this one must not go on.
async fn resolve_once(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    approval_id: ApprovalId,
    status: ApprovalStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    if repositories::agent::resolve_approval(&mut **tx, approval_id, status, now).await? {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "the approval request was already resolved".to_string(),
        ))
    }
}

/// Announces a step recorded in this transaction, as `record_step` does.
async fn publish_step(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_id: AccountId,
    step: &AgentStep,
) -> Result<(), AppError> {
    let event = SystemEvent::AgentStepCompleted {
        owner_id,
        run_id: step.run_id.to_string(),
        step_number: step.step_number,
        thought: step.thought.clone(),
        tool_name: step.tool_name.clone(),
        status: step.status.to_string(),
    };
    repositories::outbox::publish(tx, &event).await?;
    Ok(())
}

/// The number for the next step of a run: the job handler numbers its steps
/// by how many exist, and `run.current_step` is not advanced as steps are
/// recorded, so it cannot be used here.
async fn next_step_number(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    run_id: AgentRunId,
) -> Result<i32, AppError> {
    let steps = repositories::agent::get_run_steps(&mut **tx, run_id).await?;
    Ok(steps.len() as i32 + 1)
}

impl AgentService {
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

        // Lock the run first: approve, reject and cancel take turns on it, so
        // the status checked below is the status that gets written.
        let mut run = repositories::agent::lock_run(&mut *tx, approval.run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;
        // A cancelled or finished run must not come back to life, and its
        // held action must not run: refuse (the transaction rolls back).
        ensure_waiting_for_approval(&run)?;

        approval.approve(now)?;
        resolve_once(&mut tx, approval_id, ApprovalStatus::Approved, now).await?;

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

        // The marker, committed with the approval, is what tells the job to
        // run the held call: the model is not asked to decide it again.
        let step_number = next_step_number(&mut tx, run.id).await?;
        let marker = AgentStep::approval_marker(
            run.id,
            step_number,
            &approval_id.to_string(),
            &approval.tool_name,
            &approval.tool_input,
            now,
        );
        repositories::agent::record_step(&mut *tx, &marker).await?;
        publish_step(&mut tx, run.account_id, &marker).await?;

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

        let mut run = repositories::agent::lock_run(&mut *tx, approval.run_id)
            .await?
            .ok_or(AppError::NotFound("agent run not found".to_string()))?;
        ensure_waiting_for_approval(&run)?;

        approval.reject(now)?;
        resolve_once(&mut tx, approval_id, ApprovalStatus::Rejected, now).await?;

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
        let step_number = next_step_number(&mut tx, run.id).await?;
        let step = AgentStep::new(
            run.id,
            step_number,
            Some("Action was rejected by user, reconsidering plan".to_string()),
            Some(approval.tool_name.clone()),
            Some(approval.tool_input.clone()),
            Some(json!({ "rejected": true, "reason": reject_reason })),
            StepStatus::Blocked,
            0,
            now,
        );
        repositories::agent::record_step(&mut *tx, &step).await?;
        publish_step(&mut tx, run.account_id, &step).await?;

        let job = crate::domain::jobs::NewJob::agent_execute(
            *account_id,
            *run.id.as_uuid(),
            run.origin_conversation,
        );
        repositories::jobs::enqueue(&mut *tx, &job).await?;

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
}
