use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::accounts::AccountId;
use crate::domain::agent::{ScheduledTaskDefinition, TaskId};

#[derive(Debug, sqlx::FromRow)]
struct ScheduledTaskRow {
    id: Uuid,
    account_id: Uuid,
    task_id: String,
    cron_expression: String,
    enabled: bool,
    config: serde_json::Value,
    last_run_at: Option<DateTime<Utc>>,
    next_run_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ScheduledTaskRow> for ScheduledTaskDefinition {
    fn from(r: ScheduledTaskRow) -> Self {
        Self {
            id: r.id,
            account_id: AccountId::from_uuid(r.account_id),
            task_id: TaskId::new(r.task_id).unwrap_or_else(|_| TaskId::new("task").unwrap()),
            cron_expression: r.cron_expression,
            enabled: r.enabled,
            config: r.config,
            last_run_at: r.last_run_at,
            next_run_at: r.next_run_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub async fn upsert_scheduled_task<'e>(
    conn: impl PgExecutor<'e>,
    task: &ScheduledTaskDefinition,
) -> Result<ScheduledTaskDefinition, sqlx::Error> {
    let row = sqlx::query_as!(
        ScheduledTaskRow,
        r#"
        INSERT INTO scheduled_agent_tasks (
            id, account_id, task_id, cron_expression, enabled,
            config, last_run_at, next_run_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (account_id, task_id)
        DO UPDATE SET
            cron_expression = EXCLUDED.cron_expression,
            enabled = EXCLUDED.enabled,
            config = EXCLUDED.config,
            next_run_at = EXCLUDED.next_run_at,
            updated_at = EXCLUDED.updated_at
        RETURNING
            id, account_id, task_id, cron_expression, enabled,
            config, last_run_at, next_run_at, created_at, updated_at
        "#,
        task.id,
        task.account_id.as_uuid(),
        task.task_id.as_str(),
        task.cron_expression,
        task.enabled,
        task.config,
        task.last_run_at,
        task.next_run_at,
        task.created_at,
        task.updated_at,
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn list_scheduled_tasks_for_account<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
) -> Result<Vec<ScheduledTaskDefinition>, sqlx::Error> {
    let rows = sqlx::query_as!(
        ScheduledTaskRow,
        r#"
        SELECT
            id, account_id, task_id, cron_expression, enabled,
            config, last_run_at, next_run_at, created_at, updated_at
        FROM scheduled_agent_tasks
        WHERE account_id = $1
        ORDER BY created_at ASC
        "#,
        account_id.as_uuid(),
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn find_scheduled_task<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
    task_id: &TaskId,
) -> Result<Option<ScheduledTaskDefinition>, sqlx::Error> {
    let row = sqlx::query_as!(
        ScheduledTaskRow,
        r#"
        SELECT
            id, account_id, task_id, cron_expression, enabled,
            config, last_run_at, next_run_at, created_at, updated_at
        FROM scheduled_agent_tasks
        WHERE account_id = $1 AND task_id = $2
        "#,
        account_id.as_uuid(),
        task_id.as_str(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_due_scheduled_tasks<'e>(
    conn: impl PgExecutor<'e>,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ScheduledTaskDefinition>, sqlx::Error> {
    let rows = sqlx::query_as!(
        ScheduledTaskRow,
        r#"
        SELECT
            id, account_id, task_id, cron_expression, enabled,
            config, last_run_at, next_run_at, created_at, updated_at
        FROM scheduled_agent_tasks
        WHERE enabled = TRUE AND next_run_at <= $1
        ORDER BY next_run_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#,
        now,
        limit,
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update_task_next_run<'e>(
    conn: impl PgExecutor<'e>,
    id: Uuid,
    last_run_at: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        UPDATE scheduled_agent_tasks
        SET
            last_run_at = $2,
            next_run_at = $3,
            updated_at = $4
        WHERE id = $1
        "#,
        id,
        last_run_at,
        next_run_at,
        updated_at,
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}
