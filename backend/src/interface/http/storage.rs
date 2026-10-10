//! Authenticated and signed media storage access.
//!
//! Provides `GET /api/v1/storage/*key` allowing binary downloads for owners
//! or callers holding a valid, unexpired HMAC signature.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
};
use serde::Deserialize;
use tracing::{error, warn};
use utoipa::IntoParams;

use crate::domain::storage::{StorageError, StorageKey};
use crate::infrastructure::storage::local::verify_storage_signature;
use crate::interface::http::auth::types::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
#[allow(
    unused_imports,
    reason = "named only inside #[utoipa::path] attributes, where it fixes the OpenAPI schema name; a path-qualified body would rename it"
)]
use crate::interface::http::error::ApiErrorResponse;

#[derive(Debug, Deserialize, IntoParams)]
pub struct StorageQuery {
    /// HMAC-SHA256 signature for presigned temporary access.
    pub sig: Option<String>,
    /// Expiration timestamp in seconds since Unix epoch.
    pub exp: Option<i64>,
}

/// Deduces a content-type header from the storage key's filename extension.
fn content_type_for_key(key: &StorageKey) -> &'static str {
    let filename = key.filename();
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Retrieves stored binary asset for a given canonical storage key.
///
/// Access is granted if:
/// 1. The caller is authenticated as the account owning the key (`key.account_id() == user.account_id`), OR
/// 2. An unexpired HMAC token matching the key is provided in the query string (`sig` and `exp`).
#[utoipa::path(
    get,
    path = "/api/v1/storage/{key}",
    params(
        ("key" = String, Path, description = "Canonical storage key"),
        StorageQuery
    ),
    responses(
        (status = 200, description = "Storage media binary stream"),
        (status = 400, description = "Invalid storage key", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized: not logged in and no signature", body = ApiErrorResponse),
        (status = 403, description = "Forbidden: cross-account access or expired signature", body = ApiErrorResponse),
        (status = 404, description = "Storage object not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_storage(
    State(state): State<ApiState>,
    user: Option<AuthUser>,
    Path(key_str): Path<String>,
    Query(query): Query<StorageQuery>,
) -> Result<Response, ApiError> {
    let clean_key = key_str.trim_start_matches('/');
    let key = StorageKey::parse(clean_key).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let mut authorized = false;

    // 1. Check if authenticated user owns this asset
    if let Some(ref auth_user) = user {
        if auth_user.account_id == key.account_id() {
            authorized = true;
        }
    }

    // 2. Check if a valid HMAC signature was provided
    if !authorized {
        if let (Some(sig), Some(exp)) = (&query.sig, query.exp) {
            if verify_storage_signature(&state.storage_signing_secret, &key, exp, sig) {
                authorized = true;
            } else {
                warn!(%key, "Invalid or expired storage signature presented");
                return Err(ApiError::forbidden("Invalid or expired storage signature"));
            }
        }
    }

    if !authorized {
        if user.is_some() {
            return Err(ApiError::forbidden("Access denied to storage asset"));
        } else {
            return Err(ApiError::unauthorized("Authentication required"));
        }
    }

    // Fetch binary from configured storage backend
    let data = state
        .services
        .storage
        .get(&key)
        .await
        .map_err(|e| match e {
            StorageError::NotFound(_) => ApiError::not_found("Storage asset not found"),
            StorageError::AccessDenied => ApiError::forbidden("Access denied"),
            StorageError::InvalidKey(msg) => ApiError::bad_request(msg),
            StorageError::Backend(err) => {
                error!(%err, "Storage backend error");
                ApiError::internal("Failed to retrieve storage object")
            }
            StorageError::Unsupported(msg) => ApiError::bad_request(msg),
        })?;

    let content_type = content_type_for_key(&key);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .body(Body::from(data))
        .map_err(|e| ApiError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::storage::StorageCategory;
    use crate::infrastructure::storage::local::{sign_storage_key, LocalStorageService};
    use crate::infrastructure::storage::StorageService;
    use axum::extract::Query;
    use bytes::Bytes;
    use std::sync::Arc;

    async fn setup_test_state() -> (ApiState, LocalStorageService, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let signing_secret = "test-secret-12345".to_string();
        let storage =
            LocalStorageService::new(temp_dir.path(), None, signing_secret.clone()).unwrap();

        let pool = crate::db::pool::test_pool().await;
        let services =
            crate::application::AppServices::new(&pool).with_storage(Arc::new(storage.clone()));

        let mut state = ApiState::new(
            crate::db::Database { pool },
            None,
            "http://localhost".to_string(),
            reqwest::Client::new(),
        );
        state.services = services;
        state.storage_signing_secret = signing_secret;

        (state, storage, temp_dir)
    }

    #[tokio::test]
    async fn owner_can_read_own_storage_asset() {
        let (state, storage, _dir) = setup_test_state().await;
        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "png");

        storage
            .put(&key, Bytes::from_static(b"receipt png bytes"), "image/png")
            .await
            .unwrap();

        let user = Some(AuthUser::for_account(owner));
        let res = get_storage(
            State(state),
            user,
            Path(key.as_str().to_string()),
            Query(StorageQuery {
                sig: None,
                exp: None,
            }),
        )
        .await;

        assert!(res.is_ok());
        let response = res.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
    }

    #[tokio::test]
    async fn cross_account_access_forbidden() {
        let (state, storage, _dir) = setup_test_state().await;
        let owner = AccountId::from_seed("acct_alice");
        let attacker = AccountId::from_seed("acct_bob");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "png");

        storage
            .put(&key, Bytes::from_static(b"secret data"), "image/png")
            .await
            .unwrap();

        let user = Some(AuthUser::for_account(attacker));
        let res = get_storage(
            State(state),
            user,
            Path(key.as_str().to_string()),
            Query(StorageQuery {
                sig: None,
                exp: None,
            }),
        )
        .await;

        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unauthenticated_access_without_signature_unauthorized() {
        let (state, storage, _dir) = setup_test_state().await;
        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");

        storage
            .put(&key, Bytes::from_static(b"image data"), "image/jpeg")
            .await
            .unwrap();

        let res = get_storage(
            State(state),
            None,
            Path(key.as_str().to_string()),
            Query(StorageQuery {
                sig: None,
                exp: None,
            }),
        )
        .await;

        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unauthenticated_access_with_valid_signature_succeeds() {
        let (state, storage, _dir) = setup_test_state().await;
        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");

        storage
            .put(&key, Bytes::from_static(b"valid photo"), "image/jpeg")
            .await
            .unwrap();

        let exp = chrono::Utc::now().timestamp() + 300;
        let sig = sign_storage_key(&state.storage_signing_secret, key.as_str(), exp);

        let res = get_storage(
            State(state),
            None,
            Path(key.as_str().to_string()),
            Query(StorageQuery {
                sig: Some(sig),
                exp: Some(exp),
            }),
        )
        .await;

        assert!(res.is_ok());
        let response = res.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
    }

    #[tokio::test]
    async fn expired_signature_forbidden() {
        let (state, storage, _dir) = setup_test_state().await;
        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");

        storage
            .put(&key, Bytes::from_static(b"photo"), "image/jpeg")
            .await
            .unwrap();

        let exp = chrono::Utc::now().timestamp() - 10; // expired
        let sig = sign_storage_key(&state.storage_signing_secret, key.as_str(), exp);

        let res = get_storage(
            State(state),
            None,
            Path(key.as_str().to_string()),
            Query(StorageQuery {
                sig: Some(sig),
                exp: Some(exp),
            }),
        )
        .await;

        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.status, StatusCode::FORBIDDEN);
    }
}
