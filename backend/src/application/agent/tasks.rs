use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::agent::{AgentRun, AgentRunId, RunStatus, ScheduledTaskDefinition, TaskId};
use crate::infrastructure::postgres::repositories;
use crate::utils::time::now_at_storage_precision;

use super::AgentService;

impl AgentService {
    /// Upserts a scheduled recurring task definition.
    pub async fn schedule_task(
        &self,
        account_id: &AccountId,
        task_id: TaskId,
        cron_expression: impl Into<String>,
        config: serde_json::Value,
        next_run_at: DateTime<Utc>,
    ) -> Result<ScheduledTaskDefinition, AppError> {
        let now = now_at_storage_precision();
        let task = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: *account_id,
            task_id,
            cron_expression: cron_expression.into(),
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

    /// Lists scheduled tasks for an account.
    pub async fn list_scheduled_tasks(
        &self,
        account_id: &AccountId,
    ) -> Result<Vec<ScheduledTaskDefinition>, AppError> {
        let tasks =
            repositories::agent::list_scheduled_tasks_for_account(&self.pool, account_id).await?;
        Ok(tasks)
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

            let created_run = repositories::agent::create_run(&mut *tx, &run).await?;

            let job = crate::domain::jobs::NewJob::agent_execute(
                task.account_id,
                *created_run.id.as_uuid(),
                None,
            );
            repositories::jobs::enqueue(&mut *tx, &job).await?;

            // Push next run ahead by 1 hour placeholder until cron parser computes next instant
            let next_run = now + chrono::Duration::hours(1);
            repositories::agent::update_task_next_run(&mut *tx, task.id, now, next_run, now)
                .await?;
        }

        tx.commit().await?;

        Ok(count)
    }
}
