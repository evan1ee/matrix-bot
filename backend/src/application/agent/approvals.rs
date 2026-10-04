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
