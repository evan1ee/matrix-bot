use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AgentError, ABSOLUTE_MAX_STEPS, DEFAULT_MAX_STEPS, MAX_GOAL_CHARS};
use crate::domain::accounts::AccountId;

/// Unique identifier for an agent execution run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentRunId(Uuid);

impl AgentRunId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn into_inner(self) -> Uuid {
        self.0
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for AgentRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "run_{}", self.0.simple())
    }
}

impl From<Uuid> for AgentRunId {
    fn from(u: Uuid) -> Self {
        Self(u)
    }
}

/// Execution lifecycle status of an agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Planning,
    Executing,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::WaitingApproval => "waiting_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether this run is still actively occupying an account run slot.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Planning | Self::Executing | Self::WaitingApproval
        )
    }

    /// Whether this status is an immutable terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Checks if a transition from `self` to `next` is valid.
    pub fn can_transition_to(&self, next: Self) -> bool {
        match (self, next) {
            // Self-transition allowed for idempotency
            (a, b) if *a == b => true,
            // Terminal states cannot transition to anything
            (Self::Completed | Self::Failed | Self::Cancelled, _) => false,
            // Planning can move to Executing, WaitingApproval, Completed, Failed, or Cancelled
            (
                Self::Planning,
                Self::Executing
                | Self::WaitingApproval
                | Self::Completed
                | Self::Failed
                | Self::Cancelled,
            ) => true,
            // Executing can move to WaitingApproval, Completed, Failed, or Cancelled
            (
                Self::Executing,
                Self::WaitingApproval | Self::Completed | Self::Failed | Self::Cancelled,
            ) => true,
            // WaitingApproval can resume to Executing, or terminate via Completed, Failed, or Cancelled
            (
                Self::WaitingApproval,
                Self::Executing | Self::Completed | Self::Failed | Self::Cancelled,
            ) => true,
            _ => false,
        }
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for RunStatus {
    type Err = AgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "planning" => Ok(Self::Planning),
            "executing" => Ok(Self::Executing),
            "waiting_approval" => Ok(Self::WaitingApproval),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(AgentError::InvalidStatus(other.to_string())),
        }
    }
}

/// An autonomous agent execution run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: AgentRunId,
    pub scheduled_task_id: Option<Uuid>,
    pub account_id: AccountId,
    pub task_name: String,
    pub goal: String,
    pub origin_conversation: Option<String>,
    pub status: RunStatus,
    pub max_steps: i32,
    pub current_step: i32,
    pub total_tokens: i32,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl AgentRun {
    /// Creates a new `AgentRun` validating invariants.
    pub fn new(
        account_id: AccountId,
        task_name: impl Into<String>,
        goal: impl Into<String>,
        origin_conversation: Option<String>,
        max_steps: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<Self, AgentError> {
        let task_name = task_name.into().trim().to_string();
        let goal = goal.into().trim().to_string();

        if goal.is_empty() {
            return Err(AgentError::InvalidGoal("goal cannot be empty".to_string()));
        }
        if goal.chars().count() > MAX_GOAL_CHARS {
            return Err(AgentError::InvalidGoal(format!(
                "goal exceeds maximum allowed length of {} characters",
                MAX_GOAL_CHARS
            )));
        }

        let max_steps = match max_steps {
            Some(n) if n > 0 => n.min(ABSOLUTE_MAX_STEPS),
            _ => DEFAULT_MAX_STEPS,
        };

        Ok(Self {
            id: AgentRunId::generate(),
            scheduled_task_id: None,
            account_id,
            task_name: if task_name.is_empty() {
                "custom_goal".to_string()
            } else {
                task_name
            },
            goal,
            origin_conversation,
            status: RunStatus::Planning,
            max_steps,
            current_step: 0,
            total_tokens: 0,
            error: None,
            started_at: now,
            completed_at: None,
            updated_at: now,
        })
    }

    /// Advances the run by one step, checking monotonicity, ceiling, and updating token counts.
    pub fn advance_step(
        &mut self,
        step_number: i32,
        tokens_used: i32,
        now: DateTime<Utc>,
    ) -> Result<(), AgentError> {
        if self.status.is_terminal() {
            return Err(AgentError::InvalidStatusTransition {
                from: self.status,
                to: RunStatus::Executing,
            });
        }
        let expected = self.current_step + 1;
        if step_number != expected {
            return Err(AgentError::NonMonotonicStep {
                expected,
                got: step_number,
            });
        }
        if step_number > self.max_steps {
            return Err(AgentError::StepLimitExceeded {
                max: self.max_steps,
                attempted: step_number,
            });
        }

        self.current_step = step_number;
        self.total_tokens = self.total_tokens.saturating_add(tokens_used.max(0));
        self.status = RunStatus::Executing;
        self.updated_at = now;
        Ok(())
    }

    /// Transitions the run into `Executing`.
    pub fn mark_executing(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if !self.status.can_transition_to(RunStatus::Executing) {
            return Err(AgentError::InvalidStatusTransition {
                from: self.status,
                to: RunStatus::Executing,
            });
        }
        self.status = RunStatus::Executing;
        self.updated_at = now;
        Ok(())
    }

    /// Transitions the run into `WaitingApproval`.
    pub fn mark_waiting_approval(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if !self.status.can_transition_to(RunStatus::WaitingApproval) {
            return Err(AgentError::InvalidStatusTransition {
                from: self.status,
                to: RunStatus::WaitingApproval,
            });
        }
        self.status = RunStatus::WaitingApproval;
        self.updated_at = now;
        Ok(())
    }

    /// Marks the run completed successfully.
    pub fn mark_completed(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if !self.status.can_transition_to(RunStatus::Completed) {
            return Err(AgentError::InvalidStatusTransition {
                from: self.status,
                to: RunStatus::Completed,
            });
        }
        self.status = RunStatus::Completed;
        self.completed_at = Some(now);
        self.updated_at = now;
        Ok(())
    }

    /// Marks the run failed with an error message.
    pub fn mark_failed(
        &mut self,
        error: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), AgentError> {
        if !self.status.can_transition_to(RunStatus::Failed) {
            return Err(AgentError::InvalidStatusTransition {
                from: self.status,
                to: RunStatus::Failed,
            });
        }
        self.status = RunStatus::Failed;
        self.error = Some(error.into());
        self.completed_at = Some(now);
        self.updated_at = now;
        Ok(())
    }

    /// Cancels the run.
    pub fn mark_cancelled(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if !self.status.can_transition_to(RunStatus::Cancelled) {
            return Err(AgentError::InvalidStatusTransition {
                from: self.status,
                to: RunStatus::Cancelled,
            });
        }
        self.status = RunStatus::Cancelled;
        self.completed_at = Some(now);
        self.updated_at = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_run_creation_and_goal_clamping() {
        let now = Utc::now();
        let account = AccountId::from_seed("test-agent");

        let err_empty = AgentRun::new(account, "task", "", None, None, now);
        assert!(matches!(err_empty, Err(AgentError::InvalidGoal(_))));

        let long_goal = "x".repeat(MAX_GOAL_CHARS + 1);
        let err_long = AgentRun::new(account, "task", long_goal, None, None, now);
        assert!(matches!(err_long, Err(AgentError::InvalidGoal(_))));

        let valid = AgentRun::new(account, "task", "Valid goal", None, Some(50), now).unwrap();
        assert_eq!(valid.status, RunStatus::Planning);
        assert_eq!(valid.max_steps, ABSOLUTE_MAX_STEPS); // clamped from 50 to 25
        assert_eq!(valid.current_step, 0);
    }

    #[test]
    fn run_monotonic_step_advancement() {
        let now = Utc::now();
        let account = AccountId::from_seed("test-agent");
        let mut run = AgentRun::new(account, "task", "Do work", None, Some(5), now).unwrap();

        // Must start with step 1
        assert!(run.advance_step(2, 100, now).is_err());
        assert!(run.advance_step(1, 100, now).is_ok());
        assert_eq!(run.current_step, 1);
        assert_eq!(run.total_tokens, 100);
        assert_eq!(run.status, RunStatus::Executing);

        // Next must be step 2
        assert!(run.advance_step(3, 50, now).is_err());
        assert!(run.advance_step(2, 50, now).is_ok());
        assert_eq!(run.current_step, 2);
        assert_eq!(run.total_tokens, 150);

        // Cannot exceed ceiling (max_steps = 5)
        run.advance_step(3, 10, now).unwrap();
        run.advance_step(4, 10, now).unwrap();
        run.advance_step(5, 10, now).unwrap();
        let err_ceil = run.advance_step(6, 10, now);
        assert!(matches!(
            err_ceil,
            Err(AgentError::StepLimitExceeded { .. })
        ));
    }

    #[test]
    fn run_status_state_transitions() {
        let now = Utc::now();
        let account = AccountId::from_seed("test-agent");
        let mut run = AgentRun::new(account, "task", "Goal", None, None, now).unwrap();

        assert_eq!(run.status, RunStatus::Planning);
        run.mark_executing(now).unwrap();
        assert_eq!(run.status, RunStatus::Executing);

        run.mark_waiting_approval(now).unwrap();
        assert_eq!(run.status, RunStatus::WaitingApproval);
        assert!(run.status.is_active());

        // Resume from WaitingApproval
        run.advance_step(1, 20, now).unwrap();
        assert_eq!(run.status, RunStatus::Executing);

        run.mark_completed(now).unwrap();
        assert_eq!(run.status, RunStatus::Completed);
        assert!(run.status.is_terminal());

        // Terminal states cannot advance or transition
        assert!(run.advance_step(2, 10, now).is_err());
        assert!(run.mark_executing(now).is_err());
        assert!(run.mark_failed("error", now).is_err());
        assert!(run.mark_cancelled(now).is_err());
    }
}
