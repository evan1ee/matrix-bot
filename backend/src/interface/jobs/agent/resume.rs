//! Resuming a run after the user decided on a held action: at most once,
//! from the approval the user saw, before any model step.

use super::AgentExecuteRunHandler;
use crate::domain::accounts::AccountId;
use crate::domain::agent::{
    interrupted_approved_execution, pending_approved_call, AgentRun, AgentStep, StepStatus,
};
use crate::domain::jobs::AgentExecutePayload;
use crate::jobs::JobError;
use crate::llm::agent::{StepContext, StepOutcome};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::utils::html::html_escape;
use crate::utils::time::now_at_storage_precision;

/// What `resume_approved` did, so the run loop knows how to go on.
pub(super) enum Resume {
    /// No approved call is waiting: evaluate the next model step.
    NotApplicable,
    /// An approved call ran and its result is recorded: read the steps again.
    Continue,
    /// The run was failed (or is already over): stop.
    Stop,
}

impl AgentExecuteRunHandler {
    /// Fails the run with a recorded step and tells the origin conversation.
    pub(super) async fn stop_run(
        &self,
        run: &AgentRun,
        payload: &AgentExecutePayload,
        step_number: i32,
        error: &str,
        total_tokens: i32,
    ) -> Result<(), JobError> {
        let step = AgentStep::new(
            run.id,
            step_number,
            None,
            None,
            None,
            Some(serde_json::json!({ "error": error })),
            StepStatus::Failed,
            0,
            now_at_storage_precision(),
        );
        self.services
            .agent
            .record_step(run.id, &step)
            .await
            .map_err(JobError::from)?;
        self.services
            .agent
            .fail_run(run.id, error, total_tokens)
            .await
            .map_err(JobError::from)?;
        self.deliver_message(
            payload.origin_conversation.as_deref(),
            &format!("❌ Agent Goal Failed:\n\n{error}"),
            &format!(
                "<p>❌ <b>Agent Goal Failed</b></p><p>{}</p>",
                html_escape(error)
            ),
        )
        .await;
        Ok(())
    }

    /// Runs the call the user approved, at most once.
    ///
    /// Order matters: a "started" step is committed BEFORE the tool runs, and
    /// the result step is recorded after. A retry that finds the started step
    /// last cannot tell whether the tool ran, so it fails the run for the user
    /// to check instead of running the tool again. The call comes from the
    /// approval the user saw (carried by the marker) and must equal the held
    /// step; the model is not asked and no sensitivity check applies.
    pub(super) async fn resume_approved(
        &self,
        owner: &AccountId,
        run: &AgentRun,
        payload: &AgentExecutePayload,
        steps: &[AgentStep],
        config: &ResolvedAiConfig,
        total_tokens: i32,
    ) -> Result<Resume, JobError> {
        let next = steps.len() as i32 + 1;
        if interrupted_approved_execution(steps) {
            let error = "An approved action may have run, but its result was not recorded. \
                         Please check manually whether it took effect before repeating it.";
            self.stop_run(run, payload, next, error, total_tokens)
                .await?;
            return Ok(Resume::Stop);
        }
        let Some((held, marker)) = pending_approved_call(steps) else {
            return Ok(Resume::NotApplicable);
        };
        let held_call = (
            held.tool_name.clone().unwrap_or_default(),
            held.tool_input
                .clone()
                .unwrap_or_else(|| serde_json::json!({})),
        );
        if marker.approved_call().as_ref() != Some(&held_call) {
            let error = "The approved action does not match the held call; nothing was run.";
            self.stop_run(run, payload, next, error, total_tokens)
                .await?;
            return Ok(Resume::Stop);
        }

        let allowed_tools = self.services.agent.tool_policy(run);
        if !crate::llm::agent::sandbox::is_tool_allowed(&held_call.0, allowed_tools.as_deref()) {
            let error = format!(
                "Tool '{}' is not permitted by task whitelist; execution stopped.",
                held_call.0
            );
            self.stop_run(run, payload, next, &error, total_tokens)
                .await?;
            return Ok(Resume::Stop);
        }

        // One transaction locks the run, checks it is still active (a cancel
        // since this job was queued must stop the held action) and commits the
        // started step, so there is no gap between the check and the record.
        let started = AgentStep::execution_started(run.id, next, now_at_storage_precision());
        let began = self
            .services
            .agent
            .begin_approved_execution(run.id, &started)
            .await
            .map_err(JobError::from)?;
        if !began {
            return Ok(Resume::Stop);
        }

        let step_ctx = StepContext {
            account_id: owner,
            room_id: payload
                .origin_conversation
                .as_deref()
                .unwrap_or("agent:internal"),
            goal: &run.goal,
            steps,
            allowed_tools: allowed_tools.as_deref(),
            custom_sensitive_tools: None,
            max_steps: run.max_steps,
            config,
        };
        let outcome = self
            .engine
            .execute_approved(
                &self.services,
                &self.pool,
                &step_ctx,
                held_call.0,
                held_call.1,
            )
            .await;
        match outcome {
            StepOutcome::Executed {
                thought,
                tool_name,
                tool_input,
                tool_output,
                tokens_used,
            } => {
                let step = AgentStep::new(
                    run.id,
                    next + 1,
                    thought,
                    Some(tool_name),
                    Some(tool_input),
                    Some(tool_output),
                    StepStatus::Success,
                    tokens_used,
                    now_at_storage_precision(),
                );
                self.services
                    .agent
                    .record_step(run.id, &step)
                    .await
                    .map_err(JobError::from)?;
                Ok(Resume::Continue)
            }
            other => {
                let error = match other {
                    StepOutcome::Failed { error, .. } => error,
                    _ => "The approved action could not be run".to_string(),
                };
                self.stop_run(run, payload, next + 1, &error, total_tokens)
                    .await?;
                Ok(Resume::Stop)
            }
        }
    }
}
