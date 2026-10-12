//! Application service for the autonomous agent subsystem.
//!
//! Owns transaction boundaries for starting agent runs, executing ReAct steps,
//! resolving human-in-the-loop approvals, managing task schedules, and publishing
//! transactional outbox events.

pub mod approvals;
pub mod lifecycle;
pub mod tasks;

use std::sync::Arc;

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::agent::{AgentError, ScheduledTaskRegistry};

impl From<AgentError> for AppError {
    fn from(err: AgentError) -> Self {
        match err {
            AgentError::InvalidGoal(msg) => AppError::Validation(msg),
            AgentError::InvalidTaskId(msg) => AppError::Validation(msg),
            AgentError::InvalidCronExpression(msg) => AppError::Validation(msg),
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
    pub(crate) pool: PgPool,
    pub(crate) registry: Arc<ScheduledTaskRegistry>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::domain::accounts::AccountId;
    use crate::domain::agent::{AgentRunId, RunStatus};

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

    async fn started_run(seed: &str) -> (AgentService, AccountId, AgentRunId) {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let service = AgentService::new(pool);
        let run = service
            .start_user_goal(owner, None, "Race the writers", None)
            .await
            .unwrap();
        (service, owner, run.id)
    }

    #[tokio::test]
    async fn a_cancelled_run_stays_cancelled_for_every_status_writer() {
        let (service, owner, run_id) = started_run("agent-cancel-first").await;
        service.cancel_run(&owner, run_id).await.unwrap();

        assert!(service.complete_run(run_id, 1).await.is_err());
        assert!(service.fail_run(run_id, "late", 1).await.is_err());
        assert!(service
            .require_approval(run_id, "delete_todo", json!({"id": 1}))
            .await
            .is_err());

        let run = service.get_run(&owner, run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Cancelled);
    }

    /// Whichever of the two commits first wins and the other is refused: both
    /// succeeding would mean one overwrote the other's status.
    fn exactly_one_won(a: &Result<(), AppError>, b: &Result<(), AppError>) -> bool {
        a.is_ok() != b.is_ok()
    }

    #[tokio::test]
    async fn overlapping_cancel_and_complete_have_one_winner() {
        let (service, owner, run_id) = started_run("agent-race-complete").await;

        let (cancelled, completed) = tokio::join!(
            service.cancel_run(&owner, run_id),
            service.complete_run(run_id, 1),
        );

        assert!(
            exactly_one_won(&cancelled, &completed),
            "{cancelled:?} {completed:?}"
        );
        let status = service.get_run(&owner, run_id).await.unwrap().status;
        assert_eq!(
            status,
            if cancelled.is_ok() {
                RunStatus::Cancelled
            } else {
                RunStatus::Completed
            }
        );
    }

    #[tokio::test]
    async fn overlapping_cancel_and_fail_have_one_winner() {
        let (service, owner, run_id) = started_run("agent-race-fail").await;

        let (cancelled, failed) = tokio::join!(
            service.cancel_run(&owner, run_id),
            service.fail_run(run_id, "boom", 1),
        );

        assert!(
            exactly_one_won(&cancelled, &failed),
            "{cancelled:?} {failed:?}"
        );
        let status = service.get_run(&owner, run_id).await.unwrap().status;
        assert_eq!(
            status,
            if cancelled.is_ok() {
                RunStatus::Cancelled
            } else {
                RunStatus::Failed
            }
        );
    }

    #[tokio::test]
    async fn overlapping_cancel_and_require_approval_have_one_winner() {
        let (service, owner, run_id) = started_run("agent-race-approval").await;

        let (cancelled, required) = tokio::join!(service.cancel_run(&owner, run_id), async {
            service
                .require_approval(run_id, "delete_todo", json!({"id": 1}))
                .await
                .map(|_| ())
        },);

        // Requiring approval first and cancelling after is a valid order, so
        // both may succeed; what must never happen is a cancelled run coming
        // back as waiting for approval.
        assert!(cancelled.is_ok(), "{cancelled:?} {required:?}");
        let status = service.get_run(&owner, run_id).await.unwrap().status;
        assert_eq!(status, RunStatus::Cancelled, "{required:?}");
    }
}
