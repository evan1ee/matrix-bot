use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::agent::{
    AgentError, AgentRun, AgentRunId, CronSchedule, RunStatus, ScheduledTaskDefinition, TaskId,
};
use crate::domain::events::SystemEvent;
use crate::infrastructure::postgres::repositories;
use crate::utils::time::now_at_storage_precision;

use super::AgentService;

fn task_not_found(task_id: &TaskId) -> AppError {
    AppError::NotFound(format!("scheduled task '{task_id}' not found"))
}

impl AgentService {
    /// Upserts a scheduled recurring task definition after validating `cron_expression`.
    pub async fn schedule_task(
        &self,
        account_id: &AccountId,
        task_id: TaskId,
        cron_expression: impl Into<String>,
        config: serde_json::Value,
        next_run_at: DateTime<Utc>,
    ) -> Result<ScheduledTaskDefinition, AppError> {
        let cron_expression = cron_expression.into();
        let _schedule = CronSchedule::parse(&cron_expression)?;
        let now = now_at_storage_precision();
        let task = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: *account_id,
            task_id,
            cron_expression,
            enabled: true,
            config,
            last_run_at: None,
            next_run_at,
            created_at: now,
            updated_at: now,
        };

        let saved = repositories::agent::upsert_scheduled_task(&self.pool, &task).await?;
        Ok(saved)
    }

    /// Pauses or resumes a scheduled task. Resuming recomputes `next_run_at`
    /// strictly after now, so ticks missed while paused never replay as a burst.
    pub async fn set_task_enabled(
        &self,
        account_id: &AccountId,
        task_id: &TaskId,
        enabled: bool,
    ) -> Result<ScheduledTaskDefinition, AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;
        let existing = repositories::agent::find_scheduled_task(&mut *tx, account_id, task_id)
            .await?
            .ok_or_else(|| task_not_found(task_id))?;
        let next_run_at = if enabled {
            Some(CronSchedule::parse(&existing.cron_expression)?.next_after(now)?)
        } else {
            None
        };
        let updated = repositories::agent::set_scheduled_task_state(
            &mut *tx,
            account_id,
            task_id,
            enabled,
            next_run_at,
            now,
        )
        .await?
        .ok_or_else(|| task_not_found(task_id))?;
        repositories::outbox::publish(
            &mut tx,
            &SystemEvent::AgentTaskUpdated {
                owner_id: *account_id,
                task_id: task_id.to_string(),
                enabled,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Starts an out-of-schedule run of a task. The schedule itself is left
    /// alone, and a paused task can still be run by hand.
    pub async fn trigger_task_now(
        &self,
        account_id: &AccountId,
        task_id: &TaskId,
    ) -> Result<AgentRun, AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;
        let task = repositories::agent::find_scheduled_task(&mut *tx, account_id, task_id)
            .await?
            .ok_or_else(|| task_not_found(task_id))?;
        if repositories::agent::find_active_run_for_account(&mut *tx, account_id)
            .await?
            .is_some()
        {
            return Err(AgentError::ActiveRunConflict.into());
        }
        let run = self.create_scheduled_run(&mut tx, &task, now).await?;
        repositories::outbox::publish(
            &mut tx,
            &SystemEvent::AgentRunStarted {
                owner_id: *account_id,
                run_id: run.id.to_string(),
                task_name: run.task_name.clone(),
                goal: run.goal.clone(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(run)
    }

    /// Deletes a scheduled task. Past runs stay; their link to the task is cleared.
    pub async fn delete_task(
        &self,
        account_id: &AccountId,
        task_id: &TaskId,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        if !repositories::agent::delete_scheduled_task(&mut *tx, account_id, task_id).await? {
            return Err(task_not_found(task_id));
        }
        repositories::outbox::publish(
            &mut tx,
            &SystemEvent::AgentTaskDeleted {
                owner_id: *account_id,
                task_id: task_id.to_string(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Creates the run for a scheduled task and enqueues its execution job.
    async fn create_scheduled_run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        task: &ScheduledTaskDefinition,
        now: DateTime<Utc>,
    ) -> Result<AgentRun, AppError> {
        let goal = if let Some(plugin) = self.registry.get(&task.task_id) {
            plugin.formulate_goal(&task.config)
        } else {
            format!("Scheduled execution for task '{}'", task.task_id)
        };

        let run = AgentRun {
            id: AgentRunId::generate(),
            scheduled_task_id: Some(task.id),
            account_id: task.account_id,
            task_name: task.task_id.to_string(),
            goal,
            origin_conversation: None,
            status: RunStatus::Planning,
            max_steps: 15,
            current_step: 0,
            total_tokens: 0,
            error: None,
            started_at: now,
            completed_at: None,
            updated_at: now,
        };

        let created_run = repositories::agent::create_run(&mut **tx, &run).await?;
        let job = crate::domain::jobs::NewJob::agent_execute(
            task.account_id,
            *created_run.id.as_uuid(),
            None,
        );
        repositories::jobs::enqueue(&mut **tx, &job).await?;
        Ok(created_run)
    }

    /// Lists scheduled tasks for an account.
    pub async fn list_scheduled_tasks(
        &self,
        account_id: &AccountId,
    ) -> Result<Vec<ScheduledTaskDefinition>, AppError> {
        let tasks =
            repositories::agent::list_scheduled_tasks_for_account(&self.pool, account_id).await?;
        Ok(tasks)
    }

    /// The tool whitelist the run executes under (see
    /// `ScheduledTaskRegistry::tool_policy`).
    pub fn tool_policy(&self, run: &AgentRun) -> Option<Vec<String>> {
        self.registry.tool_policy(run)
    }

    /// Sweeps due scheduled tasks and enqueues execution runs.
    pub async fn enqueue_due_scheduled_tasks(&self) -> Result<usize, AppError> {
        let now = now_at_storage_precision();
        let mut tx = self.pool.begin().await?;

        let due_tasks = repositories::agent::list_due_scheduled_tasks(&mut *tx, now, 10).await?;
        let count = due_tasks.len();

        for task in due_tasks {
            // Check if account already has an active run
            if let Some(_active) =
                repositories::agent::find_active_run_for_account(&mut *tx, &task.account_id).await?
            {
                // Skip this tick to respect the 1 active run limit
                continue;
            }

            self.create_scheduled_run(&mut tx, &task, now).await?;

            let next_run = CronSchedule::parse(&task.cron_expression)
                .and_then(|s| s.next_after(now))
                .unwrap_or_else(|_| now + chrono::Duration::hours(1));
            repositories::agent::update_task_next_run(&mut *tx, task.id, now, next_run, now)
                .await?;
        }

        tx.commit().await?;

        Ok(count)
    }
}
