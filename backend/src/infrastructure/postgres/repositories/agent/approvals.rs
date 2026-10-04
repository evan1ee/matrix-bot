use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::accounts::AccountId;
use crate::domain::agent::{AgentRunId, ApprovalId, ApprovalRequest, ApprovalStatus};

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
