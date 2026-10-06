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
    /// It carries the approval id and the call the user was shown, so the job
    /// can check it against the held step. It names no tool itself, so
    /// replaying a run for the model skips it.
    pub fn approval_marker(
        run_id: AgentRunId,
        step_number: i32,
        approval_id: &str,
        tool_name: &str,
        tool_input: &serde_json::Value,
        now: DateTime<Utc>,
    ) -> Self {
        Self::new(
            run_id,
            step_number,
            Some("Action approved by the user".to_string()),
            None,
            None,
            Some(serde_json::json!({
                "approved": true,
                "approval_id": approval_id,
                "tool_name": tool_name,
                "tool_input": tool_input,
            })),
            StepStatus::Success,
            0,
            now,
        )
    }

    /// Recorded (and committed) before an approved call runs. Seeing it as
    /// the last step on a retry means the call may already have run, so it is
    /// not run again.
    pub fn execution_started(run_id: AgentRunId, step_number: i32, now: DateTime<Utc>) -> Self {
        Self::new(
            run_id,
            step_number,
            Some("Running the action the user approved".to_string()),
            None,
            None,
            Some(serde_json::json!({ "execution_started": true })),
            StepStatus::Success,
            0,
            now,
        )
    }

    fn marks(&self, key: &str) -> bool {
        self.status == StepStatus::Success
            && self.tool_name.is_none()
            && self.tool_output.as_ref().and_then(|o| o.get(key))
                == Some(&serde_json::Value::Bool(true))
    }

    /// This step is an approval marker: the held call may now run.
    pub fn is_approved_pending_execution(&self) -> bool {
        self.marks("approved")
    }

    /// This step is an execution-started marker.
    pub fn is_execution_started(&self) -> bool {
        self.marks("execution_started")
    }

    /// The call the user approved, as stored in an approval marker.
    pub fn approved_call(&self) -> Option<(String, serde_json::Value)> {
        let output = self.tool_output.as_ref()?;
        Some((
            output.get("tool_name")?.as_str()?.to_string(),
            output.get("tool_input")?.clone(),
        ))
    }

    /// A step `reject_action` recorded.
    fn is_rejection(&self) -> bool {
        self.tool_output.as_ref().and_then(|o| o.get("rejected"))
            == Some(&serde_json::Value::Bool(true))
    }
}

/// The held call and its approval marker, while the marker is the last step.
/// `None` otherwise, so a call that already has a started or result step is
/// never run from here again.
pub fn pending_approved_call(steps: &[AgentStep]) -> Option<(&AgentStep, &AgentStep)> {
    let (marker, earlier) = steps.split_last()?;
    if !marker.is_approved_pending_execution() {
        return None;
    }
    let held = earlier.iter().rev().find(|s| {
        s.status == StepStatus::Blocked && s.tool_name.is_some() && s.tool_output.is_none()
    })?;
    Some((held, marker))
}

/// The last step is an execution-started marker with no result after it: an
/// approved call that may have run and whose result was never recorded.
pub fn interrupted_approved_execution(steps: &[AgentStep]) -> bool {
    steps.last().is_some_and(AgentStep::is_execution_started)
}

/// Steps the model initiated. Approval markers, execution-started markers,
/// the result step that follows a started marker, and rejection steps are
/// bookkeeping around a user's decision and do not use the model's budget.
pub fn model_steps(steps: &[AgentStep]) -> usize {
    steps
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            let after_start = *i > 0 && steps[*i - 1].is_execution_started();
            !(s.is_approved_pending_execution()
                || s.is_execution_started()
                || after_start
                || s.is_rejection())
        })
        .count()
}

/// The model-step budget: `max_steps`, plus one step after each approved
/// execution so the model can always report what happened, but never more
/// than twice `max_steps` in total. The cap bounds a model that keeps asking
/// for approved sensitive calls while a human keeps approving them.
pub fn step_budget(steps: &[AgentStep], max_steps: i32) -> i32 {
    let approved = steps.iter().filter(|s| s.is_execution_started()).count() as i32;
    max_steps
        .saturating_add(approved)
        .min(max_steps.saturating_mul(2))
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
        AgentStep::approval_marker(
            AgentRunId::generate(),
            n,
            "appr_1",
            "deposit_portfolio_cash",
            &serde_json::json!({"amount": "1"}),
            Utc::now(),
        )
    }

    #[test]
    fn approval_bookkeeping_steps_do_not_count_as_model_steps() {
        let run = AgentRunId::generate();
        let now = Utc::now();
        let held = blocked(1);
        let steps = vec![
            held.clone(),
            AgentStep::approval_marker(
                run,
                2,
                "appr_1",
                "deposit_portfolio_cash",
                &serde_json::json!({}),
                now,
            ),
            AgentStep::execution_started(run, 3, now),
            AgentStep::new(
                run,
                4,
                None,
                Some("deposit_portfolio_cash".into()),
                Some(serde_json::json!({})),
                Some(serde_json::json!({"ok": true})),
                StepStatus::Success,
                0,
                now,
            ),
        ];

        assert_eq!(model_steps(&steps), 1, "only the held call was the model's");
        assert_eq!(
            step_budget(&steps, 2),
            3,
            "one more step to answer after the action"
        );
        assert_eq!(step_budget(&[held], 2), 2);

        // The extra steps are capped: at most twice the configured steps in
        // total, however many approved executions there are.
        let many: Vec<AgentStep> = (0..10)
            .map(|n| AgentStep::execution_started(run, n, now))
            .collect();
        assert_eq!(step_budget(&many, 3), 6);
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
        assert_eq!(
            pending_approved_call(&steps).map(|(held, _)| held.id),
            Some(held.id)
        );

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
