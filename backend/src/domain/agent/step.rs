use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::run::AgentRunId;
use super::AgentError;

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
