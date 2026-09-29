//! Webhook account management API endpoints.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};

use crate::error::ApiError;
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::types::{ApiState, AuthUser};

use super::dto::{
    build_example_curls, CreateWebhookInput, CreatedWebhookResponse, InboundAcceptedResponse,
    UpdateWebhookInput, Webhook, WebhookLogsQuery, WebhookLogsResponse, WebhookSeverity,
};
use super::inbound::queue_and_broadcast;

#[utoipa::path(
    get,
    tag = "webhooks",
    path = "/api/v1/account/webhooks",
    responses((status = 200, body = Vec<Webhook>)),
    security(("session_cookie" = []))
)]
pub async fn list_webhooks(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<Webhook>>, ApiError> {
    let hooks = state
        .services
        .webhooks
        .list(&user.account_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(hooks.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    tag = "webhooks",
    path = "/api/v1/account/webhooks",
    responses((status = 201, body = CreatedWebhookResponse)),
    security(("session_cookie" = []))
)]
pub async fn create_webhook(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(input): Json<CreateWebhookInput>,
) -> Result<(StatusCode, Json<CreatedWebhookResponse>), ApiError> {
    if input.name.trim().is_empty() || input.name.len() > 100 {
        return Err(ApiError::bad_request(
            "Webhook name must be between 1 and 100 characters",
        ));
    }
    if input.default_room_id.trim().is_empty() {
        return Err(ApiError::bad_request("Target room ID cannot be empty"));
    }

    let client_guard = state.bot_client.read().await;
    let is_auth = crate::interface::chat::rooms::is_room_authorized_for_account(
        &state.services.rooms,
        client_guard.as_ref(),
        &user.account_id,
        input.default_room_id.trim(),
    )
    .await;
    if !is_auth {
        return Err(ApiError::bad_request(
            "You are not joined to the specified target room or the room is inaccessible",
        ));
    }

    let (webhook, raw_token) = state
        .services
        .webhooks
        .create(
            &user.account_id,
            input.name.trim(),
            input.default_room_id.trim(),
        )
        .await
        .map_err(ApiError::from)?;

    let (json_curl, tg_curl) = build_example_curls(&raw_token);

    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhookResponse {
            webhook: webhook.into(),
            raw_token,
            example_curl_json: json_curl,
            example_curl_telegram: tg_curl,
        }),
    ))
}

#[utoipa::path(
    patch,
    tag = "webhooks",
    path = "/api/v1/account/webhooks/{id}",
    responses((status = 200, body = Webhook)),
    security(("session_cookie" = []))
)]
pub async fn update_webhook(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<UpdateWebhookInput>,
) -> Result<Json<Webhook>, ApiError> {
    if let Some(ref new_room) = input.default_room_id {
        let client_guard = state.bot_client.read().await;
        let is_auth = crate::interface::chat::rooms::is_room_authorized_for_account(
            &state.services.rooms,
            client_guard.as_ref(),
            &user.account_id,
            new_room.trim(),
        )
        .await;
        if !is_auth {
            return Err(ApiError::bad_request(
                "You are not joined to the specified target room or the room is inaccessible",
            ));
        }
    }

    let updated = state
        .services
        .webhooks
        .update(
            &user.account_id,
            &id,
            input.name.as_deref(),
            input.default_room_id.as_deref(),
            input.is_active,
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(updated.into()))
}

#[utoipa::path(
    post,
    tag = "webhooks",
    path = "/api/v1/account/webhooks/{id}/rotate",
    responses((status = 200, body = CreatedWebhookResponse)),
    security(("session_cookie" = []))
)]
pub async fn rotate_webhook_token(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Path(id): Path<String>,
) -> Result<Json<CreatedWebhookResponse>, ApiError> {
    let (webhook, raw_token) = state
        .services
        .webhooks
        .rotate_token(&user.account_id, &id)
        .await
        .map_err(ApiError::from)?;

    let (json_curl, tg_curl) = build_example_curls(&raw_token);

    Ok(Json(CreatedWebhookResponse {
        webhook: webhook.into(),
        raw_token,
        example_curl_json: json_curl,
        example_curl_telegram: tg_curl,
    }))
}

#[utoipa::path(
    post,
    tag = "webhooks",
    path = "/api/v1/account/webhooks/{id}/test",
    responses((status = 202, body = InboundAcceptedResponse)),
    security(("session_cookie" = []))
)]
pub async fn test_webhook(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<InboundAcceptedResponse>), ApiError> {
    let webhook = state
        .services
        .webhooks
        .get(&user.account_id, &id)
        .await
        .map_err(ApiError::from)?;

    let outbox_id = queue_and_broadcast(
        &state,
        &webhook,
        Some("Test Webhook Alert"),
        "This is a synthetic test alert dispatched from the Web UI.",
        WebhookSeverity::Info,
        None,
        Some("markdown"),
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

#[utoipa::path(
    delete,
    tag = "webhooks",
    path = "/api/v1/account/webhooks/{id}",
    responses((status = 204)),
    security(("session_cookie" = []))
)]
pub async fn delete_webhook(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .services
        .webhooks
        .delete(&user.account_id, &id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    tag = "webhooks",
    path = "/api/v1/account/webhooks/{id}/logs",
    params(
        ("id" = String, Path, description = "Webhook ID"),
        ("limit" = Option<i64>, Query, description = "Max entries to return (default 50)"),
        ("offset" = Option<i64>, Query, description = "Offset for pagination (default 0)"),
    ),
    responses((status = 200, body = WebhookLogsResponse)),
    security(("session_cookie" = []))
)]
pub async fn get_webhook_logs(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<WebhookLogsQuery>,
) -> Result<Json<WebhookLogsResponse>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);

    let (items, total) = state
        .services
        .webhooks
        .list_logs(&user.account_id, &id, limit, offset)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(WebhookLogsResponse {
        items: items.into_iter().map(Into::into).collect(),
        total,
        limit,
        offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use crate::domain::accounts::AccountId;
    use crate::events::EventBroadcaster;
    use crate::interface::http::webhooks::dto::{InboundPayload, TelegramQuery};
    use crate::interface::http::webhooks::inbound::inbound_webhook;
    use crate::interface::http::webhooks_telegram::telegram_send_message_get;
    use axum::body::Bytes;
    use axum::http::HeaderMap;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::new(32),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_webhook_api_flow() {
        let state = test_state().await;
        let acct_str = AccountRepository::create(&state.db.pool, "alice", STATUS_ACTIVE)
            .await
            .unwrap();
        let acct = AccountId::from(acct_str);
        let user = AuthUser::for_account(acct);
        let verified = VerifiedUser(AuthUser::for_account(acct));

        // 0. Ensure unauthorized room creation is rejected
        let unauth = create_webhook(
            State(state.clone()),
            user.clone(),
            Json(CreateWebhookInput {
                name: "Intruder Alert".to_string(),
                default_room_id: "!unauthorized:matrix.org".to_string(),
            }),
        )
        .await;
        assert!(unauth.is_err());

        // Pre-populate account_rooms for authorized room via RoomsService (no raw SQL)
        state
            .services
            .rooms
            .replace_cached(
                &acct,
                &[crate::domain::rooms::JoinedRoom::new(
                    "!ops:matrix.org",
                    Some("Ops Room"),
                    false,
                )],
            )
            .await
            .unwrap();

        // 1. Create Webhook (authorized)
        let (status, Json(created)) = create_webhook(
            State(state.clone()),
            user.clone(),
            Json(CreateWebhookInput {
                name: "Server Alerts".to_string(),
                default_room_id: "!ops:matrix.org".to_string(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created.webhook.name, "Server Alerts");
        assert!(created.raw_token.starts_with("whk_"));

        // 2. Inbound JSON alert
        let payload = serde_json::to_vec(&InboundPayload {
            title: Some("CPU Alert".to_string()),
            message: Some("High CPU load **99%**".to_string()),
            text: None,
            level: Some("critical".to_string()),
            room_id: None,
            chat_id: None,
            format: Some("markdown".to_string()),
        })
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );

        let (inbound_status, Json(res)) = inbound_webhook(
            State(state.clone()),
            Path(created.raw_token.clone()),
            headers,
            Bytes::from(payload),
        )
        .await
        .unwrap();

        assert_eq!(inbound_status, StatusCode::ACCEPTED);
        assert_eq!(res.status, "queued");

        // 3. Telegram API emulation GET
        let tg_res = telegram_send_message_get(
            State(state.clone()),
            Path(created.raw_token.clone()),
            Query(TelegramQuery {
                chat_id: None,
                text: Some("Telegram alert message".to_string()),
                parse_mode: None,
            }),
        )
        .await
        .unwrap();

        assert!(tg_res.ok);

        // 4. Test Webhook trigger
        let (test_status, _) = test_webhook(
            State(state.clone()),
            user.clone(),
            Path(created.webhook.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(test_status, StatusCode::ACCEPTED);

        // 5. Delete Webhook
        let del_status = delete_webhook(
            State(state.clone()),
            verified,
            Path(created.webhook.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(del_status, StatusCode::NO_CONTENT);
    }
}
