//! Telegram Bot API emulation endpoints for webhooks.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::Utc;

use crate::error::ApiError;
use crate::interface::http::auth::types::ApiState;
use crate::modules::webhook::{
    models::{TelegramMessageResult, TelegramQuery, TelegramResponse, WebhookSeverity},
    repository::{hash_token, WebhookRepository},
};

/// Mounted outside `/api/v1` (see `api/mod.rs`), mimicking the path shape of
/// Telegram's own Bot API (`https://api.telegram.org/bot<token>/sendMessage`)
/// so existing Telegram integrations can point at this host unmodified. It is
/// unauthenticated in the session-cookie sense — the token in the URL path is
/// the credential — so it carries no `security(...)` requirement.
#[utoipa::path(
    post,
    tag = "webhooks",
    path = "/bot{token}/sendMessage",
    params(("token" = String, Path, description = "Webhook token")),
    request_body(content = TelegramQuery, description = "Accepts this shape as JSON, as `application/x-www-form-urlencoded`, or as a bare text body"),
    responses(
        (status = 200, description = "Message accepted", body = TelegramResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Invalid or inactive webhook token", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    )
)]
pub async fn telegram_send_message_post(
    State(state): State<ApiState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Result<Json<TelegramResponse>, ApiError> {
    let token_hash = hash_token(&token);
    let webhook = WebhookRepository::find_by_token_hash(&state.db.pool, &token_hash)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::unauthorized("Invalid or inactive webhook token"))?;

    if !crate::interface::http::rate_limit::check_webhook_limit(&token_hash) {
        return Err(ApiError::too_many_requests(
            "Rate limit exceeded: max 60 requests per minute",
        ));
    }

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let (text, chat_id, parse_mode) = if content_type.contains("application/json") {
        let q: TelegramQuery = serde_json::from_slice(&body_bytes)
            .map_err(|_| ApiError::bad_request("Malformed JSON body"))?;
        (q.text, q.chat_id, q.parse_mode)
    } else if content_type.contains("application/x-www-form-urlencoded") {
        let mut t = None;
        let mut c = None;
        let mut p = None;
        for (k, v) in url::form_urlencoded::parse(&body_bytes) {
            match k.as_ref() {
                "text" => t = Some(v.into_owned()),
                "chat_id" => c = Some(v.into_owned()),
                "parse_mode" => p = Some(v.into_owned()),
                _ => {}
            }
        }
        (t, c, p)
    } else {
        (
            Some(String::from_utf8_lossy(&body_bytes).to_string()),
            None,
            None,
        )
    };

    let message_text = text.ok_or_else(|| ApiError::bad_request("Missing text parameter"))?;
    super::webhooks::queue_and_broadcast(
        &state,
        &webhook,
        None,
        &message_text,
        WebhookSeverity::Info,
        chat_id,
        parse_mode.as_deref(),
    )
    .await?;

    Ok(Json(TelegramResponse {
        ok: true,
        result: TelegramMessageResult {
            message_id: 1001,
            date: Utc::now().timestamp(),
            text: message_text,
        },
    }))
}

/// Mounted outside `/api/v1` — see `telegram_send_message_post`.
#[utoipa::path(
    get,
    tag = "webhooks",
    path = "/bot{token}/sendMessage",
    params(("token" = String, Path, description = "Webhook token"), TelegramQuery),
    responses(
        (status = 200, description = "Message accepted", body = TelegramResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Invalid or inactive webhook token", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    )
)]
pub async fn telegram_send_message_get(
    State(state): State<ApiState>,
    Path(token): Path<String>,
    Query(q): Query<TelegramQuery>,
) -> Result<Json<TelegramResponse>, ApiError> {
    let token_hash = hash_token(&token);
    let webhook = WebhookRepository::find_by_token_hash(&state.db.pool, &token_hash)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::unauthorized("Invalid or inactive webhook token"))?;

    if !crate::interface::http::rate_limit::check_webhook_limit(&token_hash) {
        return Err(ApiError::too_many_requests("Rate limit exceeded"));
    }

    let message_text = q
        .text
        .ok_or_else(|| ApiError::bad_request("Missing text parameter"))?;
    super::webhooks::queue_and_broadcast(
        &state,
        &webhook,
        None,
        &message_text,
        WebhookSeverity::Info,
        q.chat_id,
        q.parse_mode.as_deref(),
    )
    .await?;

    Ok(Json(TelegramResponse {
        ok: true,
        result: TelegramMessageResult {
            message_id: 1001,
            date: Utc::now().timestamp(),
            text: message_text,
        },
    }))
}
