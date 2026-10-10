//! Administrative REST endpoints for encrypted secret credentials.
//!
//! Enforces write-only semantics: read endpoints strictly return metadata only.
//! Plaintext secrets, ciphertexts, and nonces are NEVER exposed in response payloads.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::secrets::{SecretId, SecretMetadata, SecretProvider, SecretValue};
use crate::interface::http::auth::{AdminSession, AdminUser, ApiState};
use crate::interface::http::error::ApiError;

/// Public metadata for a stored secret credential. Plaintext and ciphertext are omitted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SecretMetadataDto {
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    pub provider: String,
    pub name: String,
    pub status: String,
    pub key_version: i32,
    pub masked_preview: Option<String>,
    pub last_tested_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_test_status: Option<String>,
    pub last_test_error: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<SecretMetadata> for SecretMetadataDto {
    fn from(m: SecretMetadata) -> Self {
        Self {
            id: m.id.uuid(),
            provider: m.provider.as_str().to_string(),
            name: m.name,
            status: m.status.as_str().to_string(),
            key_version: m.key_version,
            masked_preview: m.masked_preview,
            last_tested_at: m.last_tested_at,
            last_test_status: m.last_test_status,
            last_test_error: m.last_test_error,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Request payload for creating a new encrypted credential.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSecretRequest {
    pub provider: String,
    pub name: String,
    pub secret: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request payload for replacing an existing credential's secret value.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ReplaceSecretRequest {
    pub secret: String,
}

/// Response returned when a secret is deleted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteSecretResponse {
    pub status: String,
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
}

/// Outcome of a credential probe test.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SecretTestResultDto {
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    pub status: String,
    pub error: Option<String>,
}

#[utoipa::path(
    get,
    tag = "admin",
    path = "/api/v1/admin/secrets",
    responses(
        (status = 200, body = Vec<SecretMetadataDto>),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_secrets(
    State(state): State<ApiState>,
    _admin: AdminSession,
) -> Result<Json<Vec<SecretMetadataDto>>, ApiError> {
    let metadata_list = state
        .services
        .secrets
        .list_metadata()
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        metadata_list
            .into_iter()
            .map(SecretMetadataDto::from)
            .collect(),
    ))
}

#[utoipa::path(
    post,
    tag = "admin",
    path = "/api/v1/admin/secrets",
    request_body = CreateSecretRequest,
    responses(
        (status = 200, body = SecretMetadataDto),
        (status = 400, body = ApiErrorResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_secret(
    State(state): State<ApiState>,
    admin: AdminUser,
    Json(payload): Json<CreateSecretRequest>,
) -> Result<Json<SecretMetadataDto>, ApiError> {
    let provider = payload
        .provider
        .parse::<SecretProvider>()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let secret_val = SecretValue::new(payload.secret);
    let secret_id = state
        .services
        .secrets
        .store_secret(
            &admin.account_id,
            provider,
            &payload.name,
            secret_val,
            payload.metadata,
        )
        .await
        .map_err(ApiError::from)?;

    let meta = state
        .services
        .secrets
        .get_metadata_by_id(secret_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SecretMetadataDto::from(meta)))
}

#[utoipa::path(
    put,
    tag = "admin",
    path = "/api/v1/admin/secrets/{id}",
    params(("id" = String, Path, description = "Secret ID")),
    request_body = ReplaceSecretRequest,
    responses(
        (status = 200, body = SecretMetadataDto),
        (status = 400, body = ApiErrorResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn replace_secret(
    State(state): State<ApiState>,
    admin: AdminUser,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<ReplaceSecretRequest>,
) -> Result<Json<SecretMetadataDto>, ApiError> {
    let secret_id = SecretId::from_uuid(id);
    let secret_val = SecretValue::new(payload.secret);

    state
        .services
        .secrets
        .replace_secret(&admin.account_id, secret_id, secret_val)
        .await
        .map_err(ApiError::from)?;

    let meta = state
        .services
        .secrets
        .get_metadata_by_id(secret_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(SecretMetadataDto::from(meta)))
}

#[utoipa::path(
    delete,
    tag = "admin",
    path = "/api/v1/admin/secrets/{id}",
    params(("id" = String, Path, description = "Secret ID")),
    responses(
        (status = 200, body = DeleteSecretResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_secret(
    State(state): State<ApiState>,
    admin: AdminUser,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<DeleteSecretResponse>, ApiError> {
    let secret_id = SecretId::from_uuid(id);

    state
        .services
        .secrets
        .delete_secret(&admin.account_id, secret_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DeleteSecretResponse {
        status: "deleted".to_string(),
        id,
    }))
}

#[utoipa::path(
    post,
    tag = "admin",
    path = "/api/v1/admin/secrets/{id}/test",
    params(("id" = String, Path, description = "Secret ID")),
    responses(
        (status = 200, body = SecretTestResultDto),
        (status = 401, body = ApiErrorResponse),
        (status = 403, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn test_secret(
    State(state): State<ApiState>,
    admin: AdminUser,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<SecretTestResultDto>, ApiError> {
    let secret_id = SecretId::from_uuid(id);

    let res = state
        .services
        .secrets
        .test_credential(&admin.account_id, secret_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(SecretTestResultDto {
        id: res.id,
        status: res.status,
        error: res.error,
    }))
}
