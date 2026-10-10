//! Administrative REST endpoints for runtime configuration settings.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::application::AppError;
use crate::domain::config::{SettingKey, SettingRegistry, SettingValue};
use crate::interface::http::auth::{AdminUser, ApiState};
use crate::interface::http::error::ApiError;

/// Data transfer object for a registered configuration setting with value and metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SettingItemDto {
    pub key: String,
    pub value: serde_json::Value,
    pub default_value: serde_json::Value,
    pub value_type: String,
    pub description: String,
    pub is_custom: bool,
    pub hot_reloadable: bool,
    pub category: String,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<crate::application::config::SettingItem> for SettingItemDto {
    fn from(item: crate::application::config::SettingItem) -> Self {
        let value = serde_json::to_value(&item.value).unwrap_or(serde_json::Value::Null);
        let default_value =
            serde_json::to_value(&item.default_value).unwrap_or(serde_json::Value::Null);
        Self {
            key: item.key,
            value,
            default_value,
            value_type: item.value_type.as_str().to_string(),
            description: item.description,
            is_custom: item.is_custom,
            hot_reloadable: item.hot_reloadable,
            category: item.category,
            updated_at: item.updated_at,
        }
    }
}

/// Request payload to update a setting value.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSettingRequest {
    pub value: serde_json::Value,
}

/// Response returned when a setting is reset to default.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResetSettingResponse {
    pub status: String,
    pub key: String,
}

#[utoipa::path(
    get,
    tag = "admin",
    path = "/api/v1/admin/settings",
    responses(
        (status = 200, body = Vec<SettingItemDto>),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_settings(
    State(state): State<ApiState>,
    _admin: AdminUser,
) -> Result<Json<Vec<SettingItemDto>>, ApiError> {
    let items = state
        .services
        .app_config
        .list_settings()
        .await
        .map_err(ApiError::from)?;
    Ok(Json(items.into_iter().map(SettingItemDto::from).collect()))
}

#[utoipa::path(
    get,
    tag = "admin",
    path = "/api/v1/admin/settings/{key}",
    params(("key" = String, Path, description = "Configuration setting key")),
    responses(
        (status = 200, body = SettingItemDto),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_setting(
    State(state): State<ApiState>,
    _admin: AdminUser,
    Path(key): Path<String>,
) -> Result<Json<SettingItemDto>, ApiError> {
    let parsed_key = key
        .parse::<SettingKey>()
        .map_err(AppError::from)
        .map_err(ApiError::from)?;
    let item = state
        .services
        .app_config
        .get_setting_item(parsed_key)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SettingItemDto::from(item)))
}

#[utoipa::path(
    put,
    tag = "admin",
    path = "/api/v1/admin/settings/{key}",
    params(("key" = String, Path, description = "Configuration setting key")),
    request_body = UpdateSettingRequest,
    responses(
        (status = 200, body = SettingItemDto),
        (status = 400, body = ApiErrorResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_setting(
    State(state): State<ApiState>,
    admin: AdminUser,
    Path(key): Path<String>,
    Json(payload): Json<UpdateSettingRequest>,
) -> Result<Json<SettingItemDto>, ApiError> {
    let parsed_key = key
        .parse::<SettingKey>()
        .map_err(AppError::from)
        .map_err(ApiError::from)?;

    // Parse untyped JSON value into typed domain SettingValue
    let setting_value = serde_json::from_value::<SettingValue>(payload.value)
        .map_err(|e| ApiError::bad_request(format!("Invalid setting value shape: {e}")))?;

    // Validate domain rules
    SettingRegistry::validate(parsed_key, &setting_value)
        .map_err(AppError::from)
        .map_err(ApiError::from)?;

    state
        .services
        .app_config
        .update_setting(&admin.account_id, parsed_key, setting_value)
        .await
        .map_err(ApiError::from)?;

    let updated_item = state
        .services
        .app_config
        .get_setting_item(parsed_key)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SettingItemDto::from(updated_item)))
}

#[utoipa::path(
    delete,
    tag = "admin",
    path = "/api/v1/admin/settings/{key}",
    params(("key" = String, Path, description = "Configuration setting key")),
    responses(
        (status = 200, body = ResetSettingResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn reset_setting(
    State(state): State<ApiState>,
    admin: AdminUser,
    Path(key): Path<String>,
) -> Result<Json<ResetSettingResponse>, ApiError> {
    let parsed_key = key
        .parse::<SettingKey>()
        .map_err(AppError::from)
        .map_err(ApiError::from)?;

    state
        .services
        .app_config
        .reset_setting(&admin.account_id, parsed_key)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(ResetSettingResponse {
        status: "reset".to_string(),
        key,
    }))
}
