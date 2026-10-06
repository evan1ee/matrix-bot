use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::accounts::AccountId;
use crate::domain::agent::{AgentRun, AgentRunId, RunStatus};

#[derive(Debug, sqlx::FromRow)]
struct AgentRunRow {
    id: Uuid,
    scheduled_task_id: Option<Uuid>,
    account_id: Uuid,
    task_name: String,
    goal: String,
    origin_conversation: Option<String>,
    status: String,
    max_steps: i32,
    current_step: i32,
    total_tokens: i32,
    error: Option<String>,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl From<AgentRunRow> for AgentRun {
    fn from(r: AgentRunRow) -> Self {
        Self {
            id: AgentRunId::new(r.id),
            scheduled_task_id: r.scheduled_task_id,
            account_id: AccountId::from_uuid(r.account_id),
            task_name: r.task_name,
            goal: r.goal,
            origin_conversation: r.origin_conversation,
            status: RunStatus::from_str(&r.status).unwrap_or(RunStatus::Planning),
            max_steps: r.max_steps,
            current_step: r.current_step,
            total_tokens: r.total_tokens,
            error: r.error,
            started_at: r.started_at,
            completed_at: r.completed_at,
            updated_at: r.updated_at,
        }
    }
}

pub async fn create_run<'e>(
    conn: impl PgExecutor<'e>,
    run: &AgentRun,
) -> Result<AgentRun, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentRunRow,
        r#"
        INSERT INTO agent_runs (
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        RETURNING
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        "#,
        run.id.as_uuid(),
        run.scheduled_task_id,
        run.account_id.as_uuid(),
        run.task_name,
        run.goal,
        run.origin_conversation,
        run.status.as_str(),
        run.max_steps,
        run.current_step,
        run.total_tokens,
        run.error,
        run.started_at,
        run.completed_at,
        run.updated_at,
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find_run_by_id<'e>(
    conn: impl PgExecutor<'e>,
    run_id: AgentRunId,
) -> Result<Option<AgentRun>, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentRunRow,
        r#"
        SELECT
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        FROM agent_runs
        WHERE id = $1
        "#,
        run_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

/// Like [`find_run_by_id`], but takes the row lock for the rest of the
/// transaction. Approve, reject and cancel read the run's status and then
/// change it; locking first makes them take turns, so a decision made on a
/// stale status cannot overwrite a newer one.
pub async fn lock_run<'e>(
    conn: impl PgExecutor<'e>,
    run_id: AgentRunId,
) -> Result<Option<AgentRun>, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentRunRow,
        r#"
        SELECT
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        FROM agent_runs
        WHERE id = $1
        FOR UPDATE
        "#,
        run_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_run_for_account<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
    run_id: AgentRunId,
) -> Result<Option<AgentRun>, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentRunRow,
        r#"
        SELECT
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        FROM agent_runs
        WHERE id = $1 AND account_id = $2
        "#,
        run_id.as_uuid(),
        account_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_active_run_for_account<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
) -> Result<Option<AgentRun>, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentRunRow,
        r#"
        SELECT
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        FROM agent_runs
        WHERE account_id = $1 AND status IN ('planning', 'executing', 'waiting_approval')
        ORDER BY started_at DESC
        LIMIT 1
        "#,
        account_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_runs_for_account<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
    limit: i64,
) -> Result<Vec<AgentRun>, sqlx::Error> {
    let rows = sqlx::query_as!(
        AgentRunRow,
        r#"
        SELECT
            id, scheduled_task_id, account_id, task_name, goal,
            origin_conversation, status, max_steps, current_step,
            total_tokens, error, started_at, completed_at, updated_at
        FROM agent_runs
        WHERE account_id = $1
        ORDER BY started_at DESC
        LIMIT $2
        "#,
        account_id.as_uuid(),
        limit,
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_run_status<'e>(
    conn: impl PgExecutor<'e>,
    run_id: AgentRunId,
    status: RunStatus,
    current_step: i32,
    total_tokens: i32,
    error: Option<&str>,
    completed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        UPDATE agent_runs
        SET
            status = $2,
            current_step = $3,
            total_tokens = $4,
            error = $5,
            completed_at = $6,
            updated_at = $7
        WHERE id = $1
        "#,
        run_id.as_uuid(),
        status.as_str(),
        current_step,
        total_tokens,
        error,
        completed_at,
        updated_at,
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}
