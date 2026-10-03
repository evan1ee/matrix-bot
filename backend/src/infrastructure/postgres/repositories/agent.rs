//! PostgreSQL repository for autonomous agent execution runs, granular steps,
//! approvals, and scheduled tasks.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::accounts::AccountId;
use crate::domain::agent::{
    AgentRun, AgentRunId, AgentStep, AgentStepId, ApprovalId, ApprovalRequest, ApprovalStatus,
    RunStatus, ScheduledTaskDefinition, StepStatus, TaskId,
};

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

#[derive(Debug, sqlx::FromRow)]
struct AgentStepRow {
    id: Uuid,
    run_id: Uuid,
    step_number: i32,
    thought: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_output: Option<serde_json::Value>,
    status: String,
    tokens_used: i32,
    created_at: DateTime<Utc>,
}

impl From<AgentStepRow> for AgentStep {
    fn from(r: AgentStepRow) -> Self {
        Self {
            id: AgentStepId::new(r.id),
            run_id: AgentRunId::new(r.run_id),
            step_number: r.step_number,
            thought: r.thought,
            tool_name: r.tool_name,
            tool_input: r.tool_input,
            tool_output: r.tool_output,
            status: StepStatus::from_str(&r.status).unwrap_or(StepStatus::Success),
            tokens_used: r.tokens_used,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct AgentApprovalRow {
    id: Uuid,
    run_id: Uuid,
    account_id: Uuid,
    tool_name: String,
    tool_input: serde_json::Value,
    status: String,
    expires_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<AgentApprovalRow> for ApprovalRequest {
    fn from(r: AgentApprovalRow) -> Self {
        Self {
            id: ApprovalId::new(r.id),
            run_id: AgentRunId::new(r.run_id),
            account_id: AccountId::from_uuid(r.account_id),
            tool_name: r.tool_name,
            tool_input: r.tool_input,
            status: ApprovalStatus::from_str(&r.status).unwrap_or(ApprovalStatus::Pending),
            expires_at: r.expires_at,
            resolved_at: r.resolved_at,
            created_at: r.created_at,
        }
    }
}

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

// ----------------------------------------------------------------------------
// Agent Runs
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Agent Steps
// ----------------------------------------------------------------------------

pub async fn record_step<'e>(
    conn: impl PgExecutor<'e>,
    step: &AgentStep,
) -> Result<AgentStep, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentStepRow,
        r#"
        INSERT INTO agent_steps (
            id, run_id, step_number, thought, tool_name,
            tool_input, tool_output, status, tokens_used, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id, run_id, step_number, thought, tool_name,
            tool_input, tool_output, status, tokens_used, created_at
        "#,
        step.id.as_uuid(),
        step.run_id.as_uuid(),
        step.step_number,
        step.thought,
        step.tool_name,
        step.tool_input,
        step.tool_output,
        step.status.as_str(),
        step.tokens_used,
        step.created_at,
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn get_run_steps<'e>(
    conn: impl PgExecutor<'e>,
    run_id: AgentRunId,
) -> Result<Vec<AgentStep>, sqlx::Error> {
    let rows = sqlx::query_as!(
        AgentStepRow,
        r#"
        SELECT
            id, run_id, step_number, thought, tool_name,
            tool_input, tool_output, status, tokens_used, created_at
        FROM agent_steps
        WHERE run_id = $1
        ORDER BY step_number ASC
        "#,
        run_id.as_uuid(),
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

// ----------------------------------------------------------------------------
// Approvals
// ----------------------------------------------------------------------------

pub async fn create_approval<'e>(
    conn: impl PgExecutor<'e>,
    approval: &ApprovalRequest,
) -> Result<ApprovalRequest, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentApprovalRow,
        r#"
        INSERT INTO agent_approvals (
            id, run_id, account_id, tool_name, tool_input,
            status, expires_at, resolved_at, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING
            id, run_id, account_id, tool_name, tool_input,
            status, expires_at, resolved_at, created_at
        "#,
        approval.id.as_uuid(),
        approval.run_id.as_uuid(),
        approval.account_id.as_uuid(),
        approval.tool_name,
        approval.tool_input,
        approval.status.as_str(),
        approval.expires_at,
        approval.resolved_at,
        approval.created_at,
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find_pending_approval<'e>(
    conn: impl PgExecutor<'e>,
    approval_id: ApprovalId,
) -> Result<Option<ApprovalRequest>, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentApprovalRow,
        r#"
        SELECT
            id, run_id, account_id, tool_name, tool_input,
            status, expires_at, resolved_at, created_at
        FROM agent_approvals
        WHERE id = $1 AND status = 'pending'
        "#,
        approval_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_pending_approval_for_account<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
    approval_id: ApprovalId,
) -> Result<Option<ApprovalRequest>, sqlx::Error> {
    let row = sqlx::query_as!(
        AgentApprovalRow,
        r#"
        SELECT
            id, run_id, account_id, tool_name, tool_input,
            status, expires_at, resolved_at, created_at
        FROM agent_approvals
        WHERE id = $1 AND account_id = $2 AND status = 'pending'
        "#,
        approval_id.as_uuid(),
        account_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_pending_approvals_for_account<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
) -> Result<Vec<ApprovalRequest>, sqlx::Error> {
    let rows = sqlx::query_as!(
        AgentApprovalRow,
        r#"
        SELECT
            id, run_id, account_id, tool_name, tool_input,
            status, expires_at, resolved_at, created_at
        FROM agent_approvals
        WHERE account_id = $1 AND status = 'pending'
        ORDER BY created_at ASC
        "#,
        account_id.as_uuid(),
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn resolve_approval<'e>(
    conn: impl PgExecutor<'e>,
    approval_id: ApprovalId,
    status: ApprovalStatus,
    resolved_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        UPDATE agent_approvals
        SET
            status = $2,
            resolved_at = $3
        WHERE id = $1 AND status = 'pending'
        "#,
        approval_id.as_uuid(),
        status.as_str(),
        resolved_at,
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}

// ----------------------------------------------------------------------------
// Scheduled Tasks
// ----------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::time::now_at_storage_precision;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn agent_run_and_step_persistence_lifecycle() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_repo_run");
        crate::db::pool::seed_account(&pool, &owner).await;
        let now = now_at_storage_precision();

        let run = AgentRun::new(
            owner,
            "research_task",
            "Research Rust async runtime",
            Some("matrix:!room:server".to_string()),
            Some(10),
            now,
        )
        .unwrap();

        let created = create_run(&pool, &run).await.unwrap();
        assert_eq!(created.id, run.id);
        assert_eq!(created.status, RunStatus::Planning);

        let active = find_active_run_for_account(&pool, &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.id, run.id);

        // Record a step
        let step = AgentStep::new(
            run.id,
            1,
            Some("Need to search documentation".to_string()),
            Some("search_web".to_string()),
            Some(serde_json::json!({"query": "tokio runtime"})),
            Some(serde_json::json!({"result": "Found docs"})),
            StepStatus::Success,
            120,
            now,
        );
        let recorded = record_step(&pool, &step).await.unwrap();
        assert_eq!(recorded.step_number, 1);
        assert_eq!(recorded.tokens_used, 120);

        let steps = get_run_steps(&pool, run.id).await.unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].thought.as_deref(),
            Some("Need to search documentation")
        );

        // Update run status
        let updated = update_run_status(
            &pool,
            run.id,
            RunStatus::Completed,
            1,
            120,
            None,
            Some(now),
            now,
        )
        .await
        .unwrap();
        assert!(updated);

        // Run is no longer active
        let no_active = find_active_run_for_account(&pool, &owner).await.unwrap();
        assert!(no_active.is_none());
    }

    #[tokio::test]
    async fn approval_request_round_trip_and_resolution() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_repo_appr");
        crate::db::pool::seed_account(&pool, &owner).await;
        let now = now_at_storage_precision();

        let run = AgentRun::new(owner, "task", "Goal", None, None, now).unwrap();
        create_run(&pool, &run).await.unwrap();

        let approval = ApprovalRequest::new(
            run.id,
            owner,
            "portfolio_execute_trade",
            serde_json::json!({"symbol": "BTC", "amount": 1}),
            now,
        );
        let created = create_approval(&pool, &approval).await.unwrap();
        assert_eq!(created.status, ApprovalStatus::Pending);

        let found = find_pending_approval_for_account(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.tool_name, "portfolio_execute_trade");

        // Resolve
        let resolved = resolve_approval(&pool, created.id, ApprovalStatus::Approved, now)
            .await
            .unwrap();
        assert!(resolved);

        // No longer pending
        assert!(find_pending_approval(&pool, created.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn scheduled_task_due_sweep_and_owner_isolation() {
        let pool = setup_test_db().await;
        let owner1 = AccountId::from_seed("acct_agent_sched_1");
        let owner2 = AccountId::from_seed("acct_agent_sched_2");
        crate::db::pool::seed_account(&pool, &owner1).await;
        crate::db::pool::seed_account(&pool, &owner2).await;
        let now = now_at_storage_precision();

        let task1 = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: owner1,
            task_id: TaskId::new("weekly_digest").unwrap(),
            cron_expression: "0 21 * * SUN".to_string(),
            enabled: true,
            config: serde_json::json!({}),
            last_run_at: None,
            next_run_at: now - chrono::Duration::minutes(5), // due
            created_at: now,
            updated_at: now,
        };
        upsert_scheduled_task(&pool, &task1).await.unwrap();

        let task2 = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: owner2,
            task_id: TaskId::new("financial_audit").unwrap(),
            cron_expression: "0 8 1 * *".to_string(),
            enabled: false, // disabled
            config: serde_json::json!({}),
            last_run_at: None,
            next_run_at: now - chrono::Duration::minutes(10),
            created_at: now,
            updated_at: now,
        };
        upsert_scheduled_task(&pool, &task2).await.unwrap();

        // Due sweep only returns enabled due tasks
        let mut tx = pool.begin().await.unwrap();
        let due = list_due_scheduled_tasks(&mut *tx, now, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].task_id.as_str(), "weekly_digest");
        tx.rollback().await.unwrap();

        // Owner isolation: list tasks for owner1 returns only task1
        let list1 = list_scheduled_tasks_for_account(&pool, &owner1)
            .await
            .unwrap();
        assert_eq!(list1.len(), 1);
        assert_eq!(list1[0].account_id, owner1);

        let list2 = list_scheduled_tasks_for_account(&pool, &owner2)
            .await
            .unwrap();
        assert_eq!(list2.len(), 1);
        assert_eq!(list2[0].account_id, owner2);
    }
}
