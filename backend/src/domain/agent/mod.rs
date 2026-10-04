pub mod approval;
pub mod run;
pub mod step;
pub mod task;

pub use approval::{ApprovalId, ApprovalRequest, ApprovalStatus, APPROVAL_TTL_HOURS};
pub use run::{AgentRun, AgentRunId, RunStatus};
pub use step::{AgentStep, AgentStepId, StepStatus};
pub use task::{ScheduledAgentTask, ScheduledTaskDefinition, ScheduledTaskRegistry, TaskId};

/// Maximum allowable characters in an agent goal.
pub const MAX_GOAL_CHARS: usize = 1000;

/// Default maximum ReAct steps per run.
pub const DEFAULT_MAX_STEPS: i32 = 15;

/// Absolute maximum allowed steps per run across the system.
pub const ABSOLUTE_MAX_STEPS: i32 = 25;

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
