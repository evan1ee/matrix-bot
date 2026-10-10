//! HTTP handlers for autonomous agent runs, interactive approvals, and scheduled tasks.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::domain::agent::{AgentRunId, ApprovalId, TaskId};
use crate::interface::http::auth::{ApiState, AuthUser};
#[allow(
    unused_imports,
    reason = "named only inside #[utoipa::path] attributes, where it fixes the OpenAPI schema name; a path-qualified body would rename it"
)]
use crate::interface::http::error::{ApiError, ApiErrorResponse};
use crate::utils::time::now_at_storage_precision;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AgentRunResponse {
    pub id: String,
    pub scheduled_task_id: Option<String>,
    pub account_id: String,
    pub task_name: String,
    pub goal: String,
    pub origin_conversation: Option<String>,
    pub status: String,
    pub max_steps: i32,
    pub current_step: i32,
    pub total_tokens: i32,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

impl From<crate::domain::agent::AgentRun> for AgentRunResponse {
    fn from(r: crate::domain::agent::AgentRun) -> Self {
        Self {
            id: r.id.to_string(),
            scheduled_task_id: r.scheduled_task_id.map(|u| u.to_string()),
            account_id: r.account_id.to_string(),
            task_name: r.task_name,
            goal: r.goal,
            origin_conversation: r.origin_conversation,
            status: r.status.to_string(),
            max_steps: r.max_steps,
            current_step: r.current_step,
            total_tokens: r.total_tokens,
            error: r.error,
            started_at: r.started_at.to_rfc3339(),
            completed_at: r.completed_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AgentStepResponse {
    pub id: String,
    pub run_id: String,
    pub step_number: i32,
    pub thought: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_output: Option<serde_json::Value>,
    pub status: String,
    pub tokens_used: i32,
    pub created_at: String,
}

impl From<crate::domain::agent::AgentStep> for AgentStepResponse {
    fn from(s: crate::domain::agent::AgentStep) -> Self {
        Self {
            id: s.id.to_string(),
            run_id: s.run_id.to_string(),
            step_number: s.step_number,
            thought: s.thought,
            tool_name: s.tool_name,
            tool_input: s.tool_input,
            tool_output: s.tool_output,
            status: s.status.to_string(),
            tokens_used: s.tokens_used,
            created_at: s.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AgentRunDetailResponse {
    pub run: AgentRunResponse,
    pub steps: Vec<AgentStepResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ApprovalRequestResponse {
    pub id: String,
    pub run_id: String,
    pub account_id: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub status: String,
    pub expires_at: String,
    pub resolved_at: Option<String>,
    pub created_at: String,
}

impl From<crate::domain::agent::ApprovalRequest> for ApprovalRequestResponse {
    fn from(a: crate::domain::agent::ApprovalRequest) -> Self {
        Self {
            id: a.id.to_string(),
            run_id: a.run_id.to_string(),
            account_id: a.account_id.to_string(),
            tool_name: a.tool_name,
            tool_input: a.tool_input,
            status: a.status.to_string(),
            expires_at: a.expires_at.to_rfc3339(),
            resolved_at: a.resolved_at.map(|t| t.to_rfc3339()),
            created_at: a.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ScheduledTaskResponse {
    pub id: String,
    pub account_id: String,
    pub task_id: String,
    pub cron_expression: String,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub last_run_at: Option<String>,
    pub next_run_at: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::domain::agent::ScheduledTaskDefinition> for ScheduledTaskResponse {
    fn from(t: crate::domain::agent::ScheduledTaskDefinition) -> Self {
        Self {
            id: t.id.to_string(),
            account_id: t.account_id.to_string(),
            task_id: t.task_id.to_string(),
            cron_expression: t.cron_expression,
            enabled: t.enabled,
            config: t.config,
            last_run_at: t.last_run_at.map(|d| d.to_rfc3339()),
            next_run_at: t.next_run_at.to_rfc3339(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartAgentRunRequest {
    pub goal: String,
    pub origin_conversation: Option<String>,
    pub max_steps: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema, Default)]
pub struct ListAgentRunsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct RejectActionRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScheduleTaskRequest {
    pub cron_expression: String,
    pub config: Option<serde_json::Value>,
}

#[utoipa::path(
    post,
    path = "/api/v1/agent/runs",
    request_body = StartAgentRunRequest,
    responses(
        (status = 200, description = "Agent run started", body = AgentRunResponse),
        (status = 400, description = "Validation error", body = ApiErrorResponse),
        (status = 409, description = "Active run already running", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn start_run(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(body): Json<StartAgentRunRequest>,
) -> Result<Json<AgentRunResponse>, ApiError> {
    let run = state
        .services
        .agent
        .start_user_goal(
            user.account_id,
            body.origin_conversation,
            body.goal,
            body.max_steps,
        )
        .await?;

    Ok(Json(AgentRunResponse::from(run)))
}

#[utoipa::path(
    get,
    path = "/api/v1/agent/runs",
    params(ListAgentRunsQuery),
    responses(
        (status = 200, description = "List recent agent runs", body = Vec<AgentRunResponse>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_runs(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<ListAgentRunsQuery>,
) -> Result<Json<Vec<AgentRunResponse>>, ApiError> {
    let limit = query.limit.unwrap_or(20);
    let runs = state
        .services
        .agent
        .list_runs(&user.account_id, limit)
        .await?;

    let dtos = runs.into_iter().map(AgentRunResponse::from).collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    get,
    path = "/api/v1/agent/runs/{id}",
    responses(
        (status = 200, description = "Agent run details with steps", body = AgentRunDetailResponse),
        (status = 404, description = "Run not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_run(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentRunDetailResponse>, ApiError> {
    let run_id = AgentRunId::new(id);
    let run = state
        .services
        .agent
        .get_run(&user.account_id, run_id)
        .await?;
    let steps = state
        .services
        .agent
        .get_run_steps(&user.account_id, run_id)
        .await?;

    let step_dtos = steps.into_iter().map(AgentStepResponse::from).collect();
    Ok(Json(AgentRunDetailResponse {
        run: AgentRunResponse::from(run),
        steps: step_dtos,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/agent/runs/{id}/cancel",
    responses(
        (status = 200, description = "Agent run cancelled"),
        (status = 404, description = "Run not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn cancel_run(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let run_id = AgentRunId::new(id);
    state
        .services
        .agent
        .cancel_run(&user.account_id, run_id)
        .await?;
    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/api/v1/agent/approvals",
    responses(
        (status = 200, description = "List pending approval requests", body = Vec<ApprovalRequestResponse>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_pending_approvals(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<ApprovalRequestResponse>>, ApiError> {
    let approvals = state
        .services
        .agent
        .list_pending_approvals(&user.account_id)
        .await?;

    let dtos = approvals
        .into_iter()
        .map(ApprovalRequestResponse::from)
        .collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    post,
    path = "/api/v1/agent/approvals/{id}/approve",
    responses(
        (status = 200, description = "Action approved and execution resumed"),
        (status = 404, description = "Approval not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn approve_action(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let approval_id = ApprovalId::new(id);
    state
        .services
        .agent
        .approve_action(&user.account_id, approval_id)
        .await?;
    Ok(StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/api/v1/agent/approvals/{id}/reject",
    request_body = RejectActionRequest,
    responses(
        (status = 200, description = "Action rejected and recorded"),
        (status = 404, description = "Approval not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn reject_action(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RejectActionRequest>,
) -> Result<StatusCode, ApiError> {
    let approval_id = ApprovalId::new(id);
    state
        .services
        .agent
        .reject_action(&user.account_id, approval_id, body.reason.as_deref())
        .await?;
    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/api/v1/agent/tasks",
    responses(
        (status = 200, description = "List scheduled agent tasks", body = Vec<ScheduledTaskResponse>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_tasks(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<ScheduledTaskResponse>>, ApiError> {
    let tasks = state
        .services
        .agent
        .list_scheduled_tasks(&user.account_id)
        .await?;

    let dtos = tasks.into_iter().map(ScheduledTaskResponse::from).collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    put,
    path = "/api/v1/agent/tasks/{id}",
    request_body = ScheduleTaskRequest,
    responses(
        (status = 200, description = "Scheduled task upserted", body = ScheduledTaskResponse),
        (status = 400, description = "Invalid task ID or cron expression", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn schedule_task(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ScheduleTaskRequest>,
) -> Result<Json<ScheduledTaskResponse>, ApiError> {
    let task_id = TaskId::new(id)
        .map_err(|e| ApiError::from(crate::application::AppError::Validation(e.to_string())))?;
    let now = now_at_storage_precision();
    let next_run = now + chrono::Duration::hours(1); // placeholder until cron compute
    let config = body.config.unwrap_or(serde_json::json!({}));

    let task = state
        .services
        .agent
        .schedule_task(
            &user.account_id,
            task_id,
            body.cron_expression,
            config,
            next_run,
        )
        .await?;

    Ok(Json(ScheduledTaskResponse::from(task)))
}
