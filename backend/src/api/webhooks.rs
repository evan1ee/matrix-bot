use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::api::auth::extractor::VerifiedUser;
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::webhook::{
    models::*,
    renderer::render_alert_message,
    repository::{hash_token, WebhookRepository},
    scheduler::notify_outbox,
};

fn build_example_curls(raw_token: &str) -> (String, String) {
    let json = format!(
        r#"curl -X POST https://<host>/api/v1/webhooks/{raw_token} -H "Content-Type: application/json" -d '{{"title":"Alert","message":"Server is down","level":"critical"}}'"#
    );
    let tg =
        format!(r#"curl -X POST https://<host>/bot{raw_token}/sendMessage -d "text=Server alert""#);
    (json, tg)
}

pub(crate) async fn queue_and_broadcast(
    state: &ApiState,
    webhook: &Webhook,
    title: Option<&str>,
    raw_message: &str,
    severity: WebhookSeverity,
    room_override: Option<String>,
    format: Option<&str>,
) -> Result<String, ApiError> {
    let target_room = if let Some(ref override_room) = room_override {
        let client_guard = state.bot_client.read().await;
        let is_auth = crate::modules::rooms::is_room_authorized_for_account(
            &state.db.pool,
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

    let (html, plain) = render_alert_message(&webhook.name, title, raw_message, severity, format);

    let outbox_item = WebhookRepository::enqueue_outbox(
        &state.db.pool,
        &webhook.id,
        &webhook.owner_id,
        &target_room,
        severity.as_str(),
        title,
        &html,
        &plain,
    )
    .await
    .map_err(ApiError::from)?;

    notify_outbox();

    state.broadcaster.send(SystemEvent::WebhookTriggered {
        id: webhook.id.clone(),
        owner_id: webhook.owner_id.clone(),
        outbox_id: outbox_item.id.clone(),
        room_id: target_room,
        severity: severity.as_str().to_string(),
    });

    Ok(outbox_item.id)
}

// ----------------------------------------------------------- Inbound Alerts --

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
    let webhook = WebhookRepository::find_by_token_hash(&state.db.pool, &token_hash)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::unauthorized("Invalid or inactive webhook token"))?;

    if !crate::api::rate_limit::check_webhook_limit(&token_hash) {
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

pub use crate::api::webhooks_telegram::{telegram_send_message_get, telegram_send_message_post};

// ----------------------------------------------------- Account Management --

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
    let hooks = WebhookRepository::list_by_owner(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(hooks))
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
    let is_auth = crate::modules::rooms::is_room_authorized_for_account(
        &state.db.pool,
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

    let (webhook, raw_token) = WebhookRepository::create(
        &state.db.pool,
        &user.account_id,
        input.name.trim(),
        input.default_room_id.trim(),
    )
    .await
    .map_err(ApiError::from)?;

    state
        .broadcaster
        .send(SystemEvent::WebhookCreated(webhook.clone()));
    let (json_curl, tg_curl) = build_example_curls(&raw_token);

    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhookResponse {
            webhook,
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
        let is_auth = crate::modules::rooms::is_room_authorized_for_account(
            &state.db.pool,
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
    let updated = WebhookRepository::update(
        &state.db.pool,
        &id,
        &user.account_id,
        input.name.as_deref(),
        input.default_room_id.as_deref(),
        input.is_active,
    )
    .await
    .map_err(ApiError::from)?
    .ok_or_else(|| ApiError::not_found("Webhook not found"))?;

    state
        .broadcaster
        .send(SystemEvent::WebhookUpdated(updated.clone()));
    Ok(Json(updated))
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
    let (webhook, raw_token) =
        WebhookRepository::rotate_token(&state.db.pool, &id, &user.account_id)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::not_found("Webhook not found"))?;

    state
        .broadcaster
        .send(SystemEvent::WebhookUpdated(webhook.clone()));
    let (json_curl, tg_curl) = build_example_curls(&raw_token);

    Ok(Json(CreatedWebhookResponse {
        webhook,
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
    let webhook = WebhookRepository::find_by_id(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("Webhook not found"))?;

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
    let deleted = WebhookRepository::delete(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    if !deleted {
        return Err(ApiError::not_found("Webhook not found"));
    }

    state.broadcaster.send(SystemEvent::WebhookDeleted {
        id,
        owner_id: user.account_id.clone(),
    });

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
    let _webhook = WebhookRepository::find_by_id(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("Webhook not found"))?;

    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);

    let items = WebhookRepository::list_outbox_by_webhook(
        &state.db.pool,
        &id,
        &user.account_id,
        limit,
        offset,
    )
    .await
    .map_err(ApiError::from)?;
    let total = WebhookRepository::count_outbox_by_webhook(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(WebhookLogsResponse {
        items,
        total,
        limit,
        offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::new(32),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_webhook_api_flow() {
        let state = test_state().await;
        let acct = AccountRepository::create(&state.db.pool, "alice", STATUS_ACTIVE)
            .await
            .unwrap();
        let user = AuthUser::for_account(&acct);
        let verified = VerifiedUser(AuthUser::for_account(&acct));

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

        // Pre-populate account_rooms for authorized room
        crate::db_execute!(
            &state.db.pool,
            "INSERT INTO account_rooms (account_id, room_id, name, is_direct) VALUES (?, ?, ?, FALSE)",
            &acct,
            "!ops:matrix.org",
            "Ops Room"
        )
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
