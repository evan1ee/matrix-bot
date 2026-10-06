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

    /// The step `approve_action` records when the user approves a held call.
    /// It names no tool, so replaying a run for the model skips it: the held
    /// call is the earlier blocked step, and its result step follows.
    pub fn approval_marker(run_id: AgentRunId, step_number: i32, now: DateTime<Utc>) -> Self {
        Self::new(
            run_id,
            step_number,
            Some("Action approved by the user".to_string()),
            None,
            None,
            Some(serde_json::json!({ "approved": true })),
            StepStatus::Success,
            0,
            now,
        )
    }

    /// This step is an approval marker: the held call may now run.
    pub fn is_approved_pending_execution(&self) -> bool {
        self.status == StepStatus::Success
            && self.tool_name.is_none()
            && self.tool_output.as_ref().and_then(|o| o.get("approved"))
                == Some(&serde_json::Value::Bool(true))
    }
}

/// The held call to run now: the blocked step with no output that precedes a
/// final approval marker. `None` while the last step is anything else, so a
/// call that already has its result step is never run again.
pub fn pending_approved_call(steps: &[AgentStep]) -> Option<&AgentStep> {
    let (marker, earlier) = steps.split_last()?;
    if !marker.is_approved_pending_execution() {
        return None;
    }
    earlier.iter().rev().find(|s| {
        s.status == StepStatus::Blocked && s.tool_name.is_some() && s.tool_output.is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(n: i32) -> AgentStep {
        AgentStep::new(
            AgentRunId::generate(),
            n,
            None,
            Some("deposit_portfolio_cash".to_string()),
            Some(serde_json::json!({"amount": "1"})),
            None,
            StepStatus::Blocked,
            0,
            Utc::now(),
        )
    }

    fn marker(n: i32) -> AgentStep {
        AgentStep::approval_marker(AgentRunId::generate(), n, Utc::now())
    }

    #[test]
    fn only_an_approval_marker_is_pending_execution() {
        assert!(marker(2).is_approved_pending_execution());
        assert!(!blocked(1).is_approved_pending_execution());
        let mut final_answer = marker(3);
        final_answer.tool_output = Some(serde_json::json!({"final_answer": "approved"}));
        assert!(!final_answer.is_approved_pending_execution());
    }

    #[test]
    fn the_held_call_is_found_only_while_the_marker_is_last() {
        let held = blocked(1);
        let steps = vec![held.clone(), marker(2)];
        assert_eq!(pending_approved_call(&steps).map(|s| s.id), Some(held.id));

        let mut done = held.clone();
        done.step_number = 3;
        done.status = StepStatus::Success;
        done.tool_output = Some(serde_json::json!({"ok": true}));
        let with_result = vec![held.clone(), marker(2), done];
        assert!(pending_approved_call(&with_result).is_none());
        assert!(pending_approved_call(&[held]).is_none());
        assert!(
            pending_approved_call(&[marker(1)]).is_none(),
            "no held call to run"
        );
    }
}
