//! Receipt files attached to ledger entries: upload (with best-effort OCR),
//! short-lived view links and detaching.
//!
//! Upload stores the file under the caller's `receipts` storage prefix and
//! returns its key; the key is then sent with `POST /finance/transactions`,
//! where the service refuses keys that are not the caller's own receipts.

use std::time::Duration;

use axum::{
    extract::{Multipart, Path, State},
    Json,
};
use bytes::Bytes;
use serde::Serialize;
use tracing::warn;
use utoipa::ToSchema;

use crate::domain::finance::receipt::sniff_receipt_type;
use crate::domain::storage::{StorageCategory, StorageKey};
use crate::interface::http::auth::types::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
#[allow(
    unused_imports,
    reason = "named only inside #[utoipa::path] attributes, where it fixes the OpenAPI schema name; a path-qualified body would rename it"
)]
use crate::interface::http::error::ApiErrorResponse;
use crate::interface::http::finance::Transaction;
use crate::interface::http::ocr::{run_ocr, ReceiptExtractionDto};

/// Largest accepted receipt file.
pub const MAX_RECEIPT_BYTES: usize = 10 * 1024 * 1024;
/// Request-body ceiling for the upload route: the file plus multipart framing.
pub const UPLOAD_BODY_LIMIT_BYTES: usize = MAX_RECEIPT_BYTES + 64 * 1024;
const VIEW_URL_TTL_SECS: u64 = 300;

#[derive(Debug, Serialize, ToSchema)]
pub struct ReceiptUploadResponse {
    /// Pass this as `receipt_storage_key` when creating the transaction.
    pub receipt_storage_key: String,
    /// `None` for PDFs and when OCR is disabled or failed; the file is kept anyway.
    pub extracted: Option<ReceiptExtractionDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReceiptUrlResponse {
    pub url: String,
    pub expires_in_seconds: i64,
}

/// Stores an uploaded receipt and, for images, reads it with OCR.
#[utoipa::path(
    post,
    path = "/api/v1/finance/receipts/upload",
    tag = "Finance",
    responses(
        (status = 200, description = "Receipt stored", body = ReceiptUploadResponse),
        (status = 400, description = "Missing, oversized or unsupported file", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn upload_receipt(
    user: AuthUser,
    State(state): State<ApiState>,
    mut multipart: Multipart,
) -> Result<Json<ReceiptUploadResponse>, ApiError> {
    let mut file: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read multipart payload: {e}")))?
    {
        if matches!(field.name(), Some("file") | Some("image") | None) {
            file = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Failed to read file: {e}")))?,
            );
            break;
        }
    }

    let bytes = file
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::bad_request("No receipt file provided"))?;
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(ApiError::bad_request("Receipt must be 10 MiB or smaller"));
    }
    let (ext, mime) = sniff_receipt_type(&bytes)
        .ok_or_else(|| ApiError::bad_request("Receipt must be a JPEG, PNG, WebP or PDF file"))?;

    let key = StorageKey::generate(&user.account_id, StorageCategory::Receipts, ext);
    state
        .services
        .storage
        .put(&key, bytes.clone(), mime)
        .await
        .map_err(|e| ApiError::internal(format!("receipt storage failed: {e}")))?;

    // The file is already kept; a missing or failing OCR only costs the prefill.
    let extracted = if mime.starts_with("image/") {
        match run_ocr(&state, bytes.to_vec()).await {
            Ok(scan) => {
                Some(crate::domain::finance::receipt::extract_receipt_fields(&scan.text).into())
            }
            Err(err) => {
                warn!(status = %err.status, "receipt OCR skipped after upload");
                None
            }
        }
    } else {
        None
    };

    Ok(Json(ReceiptUploadResponse {
        receipt_storage_key: key.into_inner(),
        extracted,
    }))
}

/// Returns a link to the transaction's receipt that stops working after five minutes.
#[utoipa::path(
    get,
    path = "/api/v1/finance/transactions/{id}/receipt-url",
    tag = "Finance",
    params(("id" = i64, Path, description = "Transaction id")),
    responses(
        (status = 200, description = "Presigned view link", body = ReceiptUrlResponse),
        (status = 404, description = "No such transaction, or it has no receipt", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn receipt_url(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ReceiptUrlResponse>, ApiError> {
    let key = state
        .services
        .finance
        .get_transaction(&user.account_id, id)
        .await?
        .and_then(|t| t.receipt_storage_key)
        .ok_or_else(|| ApiError::not_found("No receipt for this entry"))?;

    let url = state
        .services
        .storage
        .get_presigned_url(&key, Duration::from_secs(VIEW_URL_TTL_SECS))
        .await
        .map_err(|e| ApiError::internal(format!("presign failed: {e}")))?;

    Ok(Json(ReceiptUrlResponse {
        url,
        expires_in_seconds: VIEW_URL_TTL_SECS as i64,
    }))
}

/// Unlinks the receipt from the transaction. The stored file is left to the storage retention sweep.
#[utoipa::path(
    delete,
    path = "/api/v1/finance/transactions/{id}/receipt",
    tag = "Finance",
    params(("id" = i64, Path, description = "Transaction id")),
    responses(
        (status = 200, description = "Receipt unlinked", body = Transaction),
        (status = 404, description = "No such transaction, or it has no receipt", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn detach_receipt(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Transaction>, ApiError> {
    let tx = state
        .services
        .finance
        .detach_receipt(&user.account_id, id)
        .await?;
    Ok(Json(tx.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::finance::{Direction, NewTransaction};
    use axum::http::StatusCode;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_rcpt_alice", "acct_rcpt_bob"]).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    async fn expense(state: &ApiState, user: &AuthUser, with_receipt: bool) -> i64 {
        let key = with_receipt
            .then(|| StorageKey::generate(&user.account_id, StorageCategory::Receipts, "png"));
        state
            .services
            .finance
            .create_transaction(
                &user.account_id,
                NewTransaction {
                    direction: Direction::Expense,
                    amount_minor: 500,
                    currency: "EUR",
                    currency_exponent: 2,
                    category: None,
                    note: None,
                    receipt_storage_key: key,
                    room_id: "web",
                    sender_name: "t",
                    occurred_at: chrono::Utc::now(),
                    source: None,
                    create_missing_category: false,
                },
            )
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn receipt_url_is_presigned_for_the_owner() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_rcpt_alice");
        let id = expense(&state, &alice, true).await;

        let resp = receipt_url(State(state), alice, Path(id)).await.unwrap();

        assert_eq!(resp.expires_in_seconds, 300);
        assert!(resp.url.contains("sig="), "{}", resp.url);
    }

    #[tokio::test]
    async fn receipt_url_404_for_other_owner_and_without_receipt() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_rcpt_alice");
        let bob = AuthUser::named_seed("bob", "acct_rcpt_bob");
        let with = expense(&state, &alice, true).await;
        let bare = expense(&state, &alice, false).await;

        let err = receipt_url(State(state.clone()), bob, Path(with))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        let err = receipt_url(State(state), alice, Path(bare))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn detach_receipt_clears_the_dto_field() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_rcpt_alice");
        let id = expense(&state, &alice, true).await;

        let tx = detach_receipt(State(state.clone()), alice.clone(), Path(id))
            .await
            .unwrap();
        assert!(tx.receipt_storage_key.is_none());

        let err = detach_receipt(State(state), alice, Path(id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }
}
