//! Inbound webhook delivery endpoints.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::domain::webhooks::hash_token;
use crate::error::ApiError;
use crate::interface::http::auth::types::ApiState;

use super::dto::{InboundAcceptedResponse, InboundPayload, WebhookSeverity};

pub(crate) async fn queue_and_broadcast(
    state: &ApiState,
    webhook: &crate::domain::webhooks::Webhook,
    title: Option<&str>,
    raw_message: &str,
    severity: WebhookSeverity,
    room_override: Option<String>,
    format: Option<&str>,
) -> Result<String, ApiError> {
    let target_room = if let Some(ref override_room) = room_override {
        let client_guard = state.bot_client.read().await;
        let is_auth = crate::interface::channels::matrix::rooms::is_room_authorized_for_account(
            &state.services.rooms,
            client_guard.as_ref(),
            &webhook.owner_id,
            override_room,
        )
        .await;
        if !is_auth {
            return Err(ApiError::bad_request(
                "Target room override is not authorized for this webhook owner",
            ));
        }
        override_room.clone()
    } else {
        webhook.default_room_id.clone()
    };

    let domain_severity: crate::domain::webhooks::WebhookSeverity = severity.into();
    let (html, plain) = crate::domain::webhooks::renderer::render_alert_message(
        &webhook.name,
        title,
        raw_message,
        domain_severity,
        format,
    );

    let delivery_id = state
        .services
        .webhooks
        .enqueue_alert(webhook, &target_room, title, &html, &plain, domain_severity)
        .await
        .map_err(ApiError::from)?;

    Ok(delivery_id.to_string())
}

#[utoipa::path(
    post,
    tag = "webhooks",
    path = "/api/v1/webhooks/{token}",
    responses(
        (status = 202, body = InboundAcceptedResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 429, body = ApiErrorResponse)
    )
)]
pub async fn inbound_webhook(
    State(state): State<ApiState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body_bytes: Bytes,
) -> Result<(StatusCode, Json<InboundAcceptedResponse>), ApiError> {
    let token_hash = hash_token(&token);
    let webhook = state
        .services
        .webhooks
        .find_by_token_hash(&token_hash)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::unauthorized("Invalid or inactive webhook token"))?;

    if !crate::interface::http::rate_limit::check_webhook_limit(&token_hash) {
        return Err(ApiError::too_many_requests(
            "Rate limit exceeded: max 60 requests per minute",
        ));
    }

    let is_json = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));

    let (title, raw_message, severity, room_id, format) = if is_json {
        if let Ok(payload) = serde_json::from_slice::<InboundPayload>(&body_bytes) {
            let msg = payload.message.or(payload.text).unwrap_or_default();
            let sev = payload
                .level
                .as_deref()
                .map(WebhookSeverity::from_str_or_default)
                .unwrap_or(WebhookSeverity::Info);
            (
                payload.title,
                msg,
                sev,
                payload.room_id.or(payload.chat_id),
                payload.format,
            )
        } else {
            (
                None,
                String::from_utf8_lossy(&body_bytes).to_string(),
                WebhookSeverity::Info,
                None,
                None,
            )
        }
    } else {
        (
            None,
            String::from_utf8_lossy(&body_bytes).to_string(),
            WebhookSeverity::Info,
            None,
            None,
        )
    };

    if raw_message.trim().is_empty() {
        return Err(ApiError::bad_request("Message content cannot be empty"));
    }

    let outbox_id = queue_and_broadcast(
        &state,
        &webhook,
        title.as_deref(),
        &raw_message,
        severity,
        room_id,
        format.as_deref(),
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(InboundAcceptedResponse {
            status: "queued",
            outbox_id,
        }),
    ))
}
