use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::agent::{AgentRunId, AgentStep, AgentStepId, StepStatus};

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
