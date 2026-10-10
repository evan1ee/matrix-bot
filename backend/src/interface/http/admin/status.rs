//! Administrative REST endpoints for configuration and secret status overview.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::interface::http::auth::{AdminSession, ApiState};
use crate::interface::http::error::ApiError;

/// Consolidated status overview of runtime configuration and secret vault.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigurationStatusDto {
    pub settings_count: usize,
    pub custom_settings_count: usize,
    pub encryption_configured: bool,
    pub secrets_count: usize,
    pub active_secrets_count: usize,
    pub last_reload_at: chrono::DateTime<chrono::Utc>,
}

#[utoipa::path(
    get,
    tag = "admin",
    path = "/api/v1/admin/config-status",
    responses(
        (status = 200, body = ConfigurationStatusDto),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn config_status(
    State(state): State<ApiState>,
    _admin: AdminSession,
) -> Result<Json<ConfigurationStatusDto>, ApiError> {
    let status = state
        .services
        .app_config
        .get_status(state.services.secrets.is_configured())
        .await
        .map_err(ApiError::from)?;

    Ok(Json(ConfigurationStatusDto {
        settings_count: status.settings_count,
        custom_settings_count: status.custom_settings_count,
        encryption_configured: status.encryption_configured,
        secrets_count: status.secrets_count,
        active_secrets_count: status.active_secrets_count,
        last_reload_at: status.last_reload_at,
    }))
}
