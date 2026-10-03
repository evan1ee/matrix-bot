pub mod task;
pub use task::{ScheduledAgentTask, ScheduledTaskRegistry};

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::accounts::AccountId;

/// Maximum allowable characters in an agent goal.
pub const MAX_GOAL_CHARS: usize = 1000;

/// Default maximum ReAct steps per run.
pub const DEFAULT_MAX_STEPS: i32 = 15;

/// Absolute maximum allowed steps per run across the system.
pub const ABSOLUTE_MAX_STEPS: i32 = 25;

/// Default time-to-live for a sensitive action human-in-the-loop approval.
pub const APPROVAL_TTL_HOURS: i64 = 1;

/// Validated identifier for a scheduled or built-in task plugin (e.g. `weekly_bookmark_digest`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    /// Creates a validated `TaskId`.
    pub fn new(s: impl AsRef<str>) -> Result<Self, AgentError> {
        let s = s.as_ref().trim();
        if s.len() < 3 || s.len() > 64 {
            return Err(AgentError::InvalidTaskId(
                "task_id must be between 3 and 64 characters".to_string(),
            ));
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(AgentError::InvalidTaskId(
                "task_id must consist only of lowercase alphanumeric characters, '_', or '-'"
                    .to_string(),
            ));
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TaskId {
    type Err = AgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

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

/// Unique identifier for a granular ReAct step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentStepId(Uuid);

impl AgentStepId {
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

impl fmt::Display for AgentStepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "step_{}", self.0.simple())
    }
}

impl From<Uuid> for AgentStepId {
    fn from(u: Uuid) -> Self {
        Self(u)
    }
}

/// Unique identifier for an interactive human approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalId(Uuid);

impl ApprovalId {
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

impl fmt::Display for ApprovalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "appr_{}", self.0.simple())
    }
}

impl From<Uuid> for ApprovalId {
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

/// Outcome status of an individual ReAct step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Success,
    Failed,
    Blocked,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for StepStatus {
    type Err = AgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            other => Err(AgentError::InvalidStatus(other.to_string())),
        }
    }
}

/// Resolution status of an interactive approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn is_resolved(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

impl fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ApprovalStatus {
    type Err = AgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "expired" => Ok(Self::Expired),
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

/// An individual ReAct execution step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStep {
    pub id: AgentStepId,
    pub run_id: AgentRunId,
    pub step_number: i32,
    pub thought: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_output: Option<serde_json::Value>,
    pub status: StepStatus,
    pub tokens_used: i32,
    pub created_at: DateTime<Utc>,
}

impl AgentStep {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: AgentRunId,
        step_number: i32,
        thought: Option<String>,
        tool_name: Option<String>,
        tool_input: Option<serde_json::Value>,
        tool_output: Option<serde_json::Value>,
        status: StepStatus,
        tokens_used: i32,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: AgentStepId::generate(),
            run_id,
            step_number,
            thought,
            tool_name,
            tool_input,
            tool_output,
            status,
            tokens_used: tokens_used.max(0),
            created_at: now,
        }
    }
}

/// An interactive human approval request for sensitive tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    pub run_id: AgentRunId,
    pub account_id: AccountId,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub status: ApprovalStatus,
    pub expires_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ApprovalRequest {
    pub fn new(
        run_id: AgentRunId,
        account_id: AccountId,
        tool_name: impl Into<String>,
        tool_input: serde_json::Value,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: ApprovalId::generate(),
            run_id,
            account_id,
            tool_name: tool_name.into(),
            tool_input,
            status: ApprovalStatus::Pending,
            expires_at: now + Duration::hours(APPROVAL_TTL_HOURS),
            resolved_at: None,
            created_at: now,
        }
    }

    /// Whether this approval request is past its expiry.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.status == ApprovalStatus::Pending && now >= self.expires_at
    }

    /// Approves the request.
    pub fn approve(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if self.is_expired(now) {
            self.status = ApprovalStatus::Expired;
            return Err(AgentError::ApprovalExpired);
        }
        if self.status.is_resolved() {
            return Err(AgentError::ApprovalAlreadyResolved);
        }
        self.status = ApprovalStatus::Approved;
        self.resolved_at = Some(now);
        Ok(())
    }

    /// Rejects the request.
    pub fn reject(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if self.is_expired(now) {
            self.status = ApprovalStatus::Expired;
            return Err(AgentError::ApprovalExpired);
        }
        if self.status.is_resolved() {
            return Err(AgentError::ApprovalAlreadyResolved);
        }
        self.status = ApprovalStatus::Rejected;
        self.resolved_at = Some(now);
        Ok(())
    }
}

/// Definition for a scheduled recurring agent task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTaskDefinition {
    pub id: Uuid,
    pub account_id: AccountId,
    pub task_id: TaskId,
    pub cron_expression: String,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Pure domain errors for the Agent subsystem.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentError {
    #[error("invalid goal: {0}")]
    InvalidGoal(String),

    #[error("invalid task id: {0}")]
    InvalidTaskId(String),

    #[error("invalid status value: {0}")]
    InvalidStatus(String),

    #[error("invalid status transition from {from} to {to}")]
    InvalidStatusTransition { from: RunStatus, to: RunStatus },

    #[error("non-monotonic step: expected step {expected}, got {got}")]
    NonMonotonicStep { expected: i32, got: i32 },

    #[error("step limit exceeded: maximum allowed steps is {max}, attempted step {attempted}")]
    StepLimitExceeded { max: i32, attempted: i32 },

    #[error("approval request has expired")]
    ApprovalExpired,

    #[error("approval request has already been resolved")]
    ApprovalAlreadyResolved,

    #[error("an active agent run is already in progress for this account")]
    ActiveRunConflict,

    #[error("agent run not found")]
    RunNotFound,

    #[error("approval request not found")]
    ApprovalNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_validation_rules() {
        assert!(TaskId::new("weekly_digest").is_ok());
        assert!(TaskId::new("financial-audit-1").is_ok());
        assert!(TaskId::new("ab").is_err()); // too short
        assert!(TaskId::new("Uppercase").is_err()); // no uppercase
        assert!(TaskId::new("has space").is_err());
        assert!(TaskId::new("special!").is_err());
    }

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

    #[test]
    fn approval_request_lifecycle_and_expiry() {
        let now = Utc::now();
        let account = AccountId::from_seed("test-agent");
        let run_id = AgentRunId::generate();
        let mut req = ApprovalRequest::new(
            run_id,
            account,
            "portfolio_execute_trade",
            serde_json::json!({"symbol": "AAPL", "quantity": 10}),
            now,
        );

        assert_eq!(req.status, ApprovalStatus::Pending);
        assert!(!req.is_expired(now));

        // Expired after 1 hour + 1 second
        let future = now + Duration::hours(1) + Duration::seconds(1);
        assert!(req.is_expired(future));

        // Approval fails if expired
        let err_exp = req.approve(future);
        assert_eq!(err_exp, Err(AgentError::ApprovalExpired));
        assert_eq!(req.status, ApprovalStatus::Expired);

        // Fresh request approves cleanly
        let mut fresh = ApprovalRequest::new(
            run_id,
            account,
            "portfolio_execute_trade",
            serde_json::json!({}),
            now,
        );
        assert!(fresh.approve(now + Duration::minutes(10)).is_ok());
        assert_eq!(fresh.status, ApprovalStatus::Approved);
        assert!(fresh.resolved_at.is_some());

        // Cannot double resolve
        assert_eq!(
            fresh.reject(now + Duration::minutes(15)),
            Err(AgentError::ApprovalAlreadyResolved)
        );
    }
}
