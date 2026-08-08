use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::reminder::models::{RecurrencePattern, Reminder};
use crate::modules::reminder::repository::ReminderRepository;

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct ReminderQuery {
    pub room_id: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateReminderRequest {
    pub room_id: Option<String>,
    pub message: String,
    pub seconds: u64,
    pub recurrence: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ReminderListResponse {
    pub items: Vec<Reminder>,
    pub page: i64,
    pub limit: i64,
    /// Total pending reminders matching the query.
    pub total: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/reminders",
    params(ReminderQuery),
    responses(
        (status = 200, description = "List pending reminders", body = ReminderListResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_reminders(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<ReminderQuery>,
) -> Result<Json<ReminderListResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * limit;
    let owner_id = user.owner_id();

    let (items, total) = if let Some(room) = &query.room_id {
        let items = sqlx::query_as::<_, Reminder>(
            "SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count, created_at FROM reminders WHERE room_id = ? AND sender_mxid = ? AND status = 'pending' ORDER BY trigger_at ASC LIMIT ? OFFSET ?",
        )
        .bind(room)
        .bind(&owner_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM reminders WHERE room_id = ? AND sender_mxid = ? AND status = 'pending'",
        )
        .bind(room)
        .bind(&owner_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        (items, total)
    } else {
        let items = ReminderRepository::list_user_pending(&state.db.pool, &owner_id, limit, offset)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let total = ReminderRepository::count_user_pending(&state.db.pool, &owner_id)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        (items, total)
    };

    Ok(Json(ReminderListResponse { items, page, limit, total }))
}

#[utoipa::path(
    post,
    path = "/api/v1/reminders",
    request_body = CreateReminderRequest,
    responses(
        (status = 200, description = "Reminder created", body = Reminder),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_reminder(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateReminderRequest>,
) -> Result<Json<Reminder>, ApiError> {
    if payload.message.trim().is_empty() {
        return Err(ApiError::bad_request("Message required"));
    }
    if payload.seconds == 0 {
        return Err(ApiError::bad_request("Seconds must be greater than 0"));
    }
    if payload.message.len() > 500 {
        return Err(ApiError::bad_request("Message exceeds 500 character limit"));
    }

    let trigger_at = Utc::now() + Duration::seconds(payload.seconds as i64);
    let recurrence = payload
        .recurrence
        .as_deref()
        .map(RecurrencePattern::from_str)
        .unwrap_or(RecurrencePattern::None);

    let room_id = payload.room_id.unwrap_or_else(|| "web".to_string());

    let reminder = ReminderRepository::create(
        &state.db.pool,
        &room_id,
        &user.owner_id(),
        &user.display_name(),
        &payload.message,
        trigger_at,
        recurrence,
        // Standalone reminder: not scheduled from a todo's due date.
        None,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::ReminderCreated(reminder.clone()));

    Ok(Json(reminder))
}

#[utoipa::path(
    delete,
    path = "/api/v1/reminders/{id}",
    params(("id" = i64, Path, description = "Reminder ID")),
    responses(
        (status = 204, description = "Reminder cancelled"),
        (status = 404, description = "Reminder not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_reminder(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let owner_id = user.owner_id();
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT room_id FROM reminders WHERE id = ? AND sender_mxid = ?",
    )
    .bind(id)
    .bind(&owner_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (room_id,) = match row {
        Some(r) => r,
        None => return Err(ApiError::not_found("Reminder not found or access denied")),
    };

    sqlx::query("UPDATE reminders SET status = 'cancelled' WHERE id = ?")
        .bind(id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::ReminderCancelled { id, room_id, owner_id });

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_reminder_api_crud_and_isolation() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::local("alice");
        let bob = AuthUser::local("bob");

        // Alice creates reminder
        let req = CreateReminderRequest {
            room_id: Some("!room:1".to_string()),
            message: "Alice reminder".to_string(),
            seconds: 60,
            recurrence: Some("daily".to_string()),
        };
        let created = create_reminder(State(state.clone()), alice.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(created.message, "Alice reminder");

        // Bob lists reminders -> empty
        let query = ReminderQuery {
            room_id: None,
            page: None,
            limit: None,
        };
        let bob_list = list_reminders(State(state.clone()), bob.clone(), Query(query))
            .await
            .unwrap();
        assert_eq!(bob_list.items.len(), 0);

        // Bob deletes Alice's reminder -> gets 404 access denied
        let err = delete_reminder(State(state.clone()), bob, Path(created.id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        // Alice deletes her reminder -> 204
        let status = delete_reminder(State(state), alice, Path(created.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
