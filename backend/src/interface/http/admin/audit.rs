//! Administrative REST endpoints for configuration audit logging.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::interface::http::auth::{AdminSession, ApiState};
use crate::interface::http::error::ApiError;

/// Query parameters for paginating administrative audit logs.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct AuditLogQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Serialized row of an administrative configuration change.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigAuditRowDto {
    pub id: i64,
    pub actor_id: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_key: String,
    pub outcome: String,
    pub detail: Option<serde_json::Value>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Paginated audit log response page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditLogPageDto {
    pub entries: Vec<ConfigAuditRowDto>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[utoipa::path(
    get,
    tag = "admin",
    path = "/api/v1/admin/audit-logs",
    params(AuditLogQuery),
    responses(
        (status = 200, body = AuditLogPageDto),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_audit_logs(
    State(state): State<ApiState>,
    _admin: AdminSession,
    Query(query): Query<AuditLogQuery>,
) -> Result<Json<AuditLogPageDto>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);

    let (entries, total) = state
        .services
        .app_config
        .list_audit_logs(limit, offset)
        .await
        .map_err(ApiError::from)?;

    let dtos = entries
        .into_iter()
        .map(|e| ConfigAuditRowDto {
            id: e.id.unwrap_or_default(),
            actor_id: e.actor_id.map(|a| a.to_string()),
            action: e.action.as_str().to_string(),
            target_type: e.target_type,
            target_key: e.target_key,
            outcome: e.outcome.as_str().to_string(),
            detail: e.detail,
            created_at: e.created_at,
        })
        .collect();

    Ok(Json(AuditLogPageDto {
        entries: dtos,
        total,
        limit,
        offset,
    }))
}
