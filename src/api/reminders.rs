use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::reminder::models::{RecurrencePattern, Reminder};
use crate::modules::reminder::repository::ReminderRepository;

#[derive(Deserialize)]
pub struct ReminderQuery {
    pub room_id: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateReminderRequest {
    pub room_id: String,
    pub message: String,
    pub seconds: u64,
    pub recurrence: Option<String>,
}

#[derive(Serialize)]
pub struct ReminderListResponse {
    pub items: Vec<Reminder>,
    pub page: i64,
    pub limit: i64,
}

pub async fn list_reminders(
    State(state): State<ApiState>,
    _user: AuthUser,
    Query(query): Query<ReminderQuery>,
) -> Result<Json<ReminderListResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * limit;

    let items = if let Some(room) = &query.room_id {
        sqlx::query_as::<_, Reminder>(
            "SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count, created_at FROM reminders WHERE room_id = ? AND status = 'pending' ORDER BY trigger_at ASC LIMIT ? OFFSET ?",
        )
        .bind(room)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        sqlx::query_as::<_, Reminder>(
            "SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count, created_at FROM reminders WHERE status = 'pending' ORDER BY trigger_at ASC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    };

    Ok(Json(ReminderListResponse { items, page, limit }))
}

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

    let (sender_mxid, sender_name) = if user.username.starts_with('@') {
        let name = user
            .username
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or(&user.username)
            .to_string();
        (user.username.clone(), name)
    } else {
        (format!("@web-{}:api", user.username), user.username.clone())
    };

    let reminder = ReminderRepository::create(
        &state.db.pool,
        &payload.room_id,
        &sender_mxid,
        &sender_name,
        &payload.message,
        trigger_at,
        recurrence,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::ReminderCreated(reminder.clone()));

    Ok(Json(reminder))
}

pub async fn delete_reminder(
    State(state): State<ApiState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT room_id FROM reminders WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let (room_id,) = match row {
        Some(r) => r,
        None => return Err(ApiError::not_found("Reminder not found")),
    };

    sqlx::query("UPDATE reminders SET status = 'cancelled' WHERE id = ?")
        .bind(id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::ReminderCancelled { id, room_id });

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_reminder_api_crud() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState {
            db: crate::db::Database { pool },
            broadcaster: EventBroadcaster::default(),
            jwt_secret: "secret".to_string(),
            registration_secret: None,
            homeserver: "https://matrix.org".to_string(),
            http_client: reqwest::Client::new(),
        };
        let user = AuthUser {
            username: "bob".to_string(),
        };

        // Create
        let req = CreateReminderRequest {
            room_id: "!room:1".to_string(),
            message: "Meeting".to_string(),
            seconds: 60,
            recurrence: Some("daily".to_string()),
        };
        let created = create_reminder(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(created.message, "Meeting");

        // List
        let query = ReminderQuery {
            room_id: Some("!room:1".to_string()),
            page: None,
            limit: None,
        };
        let list = list_reminders(State(state.clone()), user.clone(), Query(query))
            .await
            .unwrap();
        assert_eq!(list.items.len(), 1);

        // Delete
        let status = delete_reminder(State(state), user, Path(created.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
