use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::{IntoParams, ToSchema};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::reminder::models::{RecurrencePattern, Reminder};
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::reminder::service::{self, NewReminder, ReminderError};

impl From<ReminderError> for ApiError {
    fn from(err: ReminderError) -> Self {
        match err {
            ReminderError::NotFound => ApiError::not_found(err.to_string()),
            ReminderError::Db(inner) => ApiError::internal(inner.to_string()),
            other => ApiError::bad_request(other.to_string()),
        }
    }
}

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct ReminderQuery {
    pub room_id: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    /// Narrows the listing to one status or view: 'pending', 'history' (non-pending),
    /// 'triggered', 'cancelled', 'failed', or 'all'. Defaults to 'pending'.
    pub status: Option<String>,
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
    /// Total reminders matching the query.
    pub total: i64,
    /// Whether more records exist beyond the current page.
    pub has_more: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/reminders",
    params(ReminderQuery),
    responses(
        (status = 200, description = "List reminders with optional status filter", body = ReminderListResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
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

    let items = ReminderRepository::list_user_reminders(
        &state.db.pool,
        &owner_id,
        query.status.as_deref(),
        query.room_id.as_deref(),
        limit,
        offset,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let total = ReminderRepository::count_user_reminders(
        &state.db.pool,
        &owner_id,
        query.status.as_deref(),
        query.room_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let has_more = offset + (items.len() as i64) < total;

    Ok(Json(ReminderListResponse {
        items,
        page,
        limit,
        total,
        has_more,
    }))
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
    security(("session_cookie" = []))
)]
pub async fn create_reminder(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateReminderRequest>,
) -> Result<Json<Reminder>, ApiError> {
    let trigger_at = Utc::now() + Duration::seconds(payload.seconds as i64);
    let recurrence = payload
        .recurrence
        .as_deref()
        .map(RecurrencePattern::from_str_or_default)
        .unwrap_or(RecurrencePattern::None);

    let room_id = crate::db::rooms::resolve_target_room(
        &state.db.pool,
        &user.owner_id(),
        payload.room_id.as_deref(),
    )
    .await;

    let reminder = service::create(
        &state.db.pool,
        Some(&state.broadcaster),
        &user.owner_id(),
        NewReminder {
            message: &payload.message,
            trigger_at,
            room_id: &room_id,
            owner_name: &user.display_name,
            recurrence,
        },
    )
    .await?;

    Ok(Json(reminder))
}

/// Mirrors `CreateReminderRequest` rather than patching individual fields: the
/// fire time is expressed as a delay from now, exactly as at creation, so an
/// edit and a create mean the same thing by the same arithmetic.
#[derive(Deserialize, ToSchema)]
pub struct UpdateReminderRequest {
    pub message: String,
    pub seconds: u64,
    pub recurrence: Option<String>,
}

#[utoipa::path(
    patch,
    path = "/api/v1/reminders/{id}",
    params(("id" = i64, Path, description = "Reminder ID")),
    request_body = UpdateReminderRequest,
    responses(
        (status = 200, description = "Reminder updated", body = Reminder),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Reminder not found, or no longer pending", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_reminder(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateReminderRequest>,
) -> Result<Json<Reminder>, ApiError> {
    if payload.message.trim().is_empty() {
        return Err(ApiError::bad_request("Message required"));
    }
    if payload.seconds == 0 {
        return Err(ApiError::bad_request("Seconds must be greater than 0"));
    }
    if payload.message.len() > service::MAX_MESSAGE_LEN {
        return Err(ApiError::bad_request(format!(
            "Message exceeds {} character limit",
            service::MAX_MESSAGE_LEN
        )));
    }

    let trigger_at = Utc::now() + Duration::seconds(payload.seconds as i64);
    let recurrence = payload
        .recurrence
        .as_deref()
        .map(RecurrencePattern::from_str_or_default)
        .unwrap_or(RecurrencePattern::None);

    let updated = ReminderRepository::update_pending(
        &state.db.pool,
        &user.owner_id(),
        id,
        &payload.message,
        trigger_at,
        recurrence,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Reminder not found, or is no longer pending"))?;

    state
        .broadcaster
        .send(SystemEvent::ReminderUpdated(updated.clone()));

    Ok(Json(updated))
}

#[utoipa::path(
    delete,
    path = "/api/v1/reminders/{id}",
    params(("id" = i64, Path, description = "Reminder ID")),
    responses(
        (status = 204, description = "Reminder cancelled or deleted"),
        (status = 404, description = "Reminder not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_reminder(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let owner_id = user.owner_id();
    let row: Option<(String, String)> = crate::db_fetch_optional!(
        &state.db.pool,
        _,
        "SELECT room_id, status FROM reminders WHERE id = ? AND owner_id = ?",
        id,
        &owner_id
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (room_id, status) = match row {
        Some(r) => r,
        None => return Err(ApiError::not_found("Reminder not found or access denied")),
    };

    if status == "pending" {
        service::cancel(&state.db.pool, Some(&state.broadcaster), &owner_id, id).await?;
    } else {
        crate::db_execute!(&state.db.pool, "DELETE FROM reminders WHERE id = ?", id)
            .map_err(|e| ApiError::internal(e.to_string()))?;

        state.broadcaster.send(SystemEvent::ReminderDeleted {
            id,
            room_id,
            owner_id,
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/v1/reminders/history",
    responses(
        (status = 204, description = "Reminder history cleared"),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn clear_reminder_history(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<StatusCode, ApiError> {
    let owner_id = user.owner_id();
    service::clear_history(&state.db.pool, Some(&state.broadcaster), &owner_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;

    #[tokio::test]
    async fn test_reminder_api_crud_and_isolation() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::named("alice", "acct_alice");
        let bob = AuthUser::named("bob", "acct_bob");

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
            status: None,
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

    #[tokio::test]
    async fn test_update_reminder_is_owner_scoped_and_validated() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::named("alice", "acct_alice");
        let bob = AuthUser::named("bob", "acct_bob");

        let created = create_reminder(
            State(state.clone()),
            alice.clone(),
            Json(CreateReminderRequest {
                room_id: Some("!room:1".to_string()),
                message: "Water the plants".to_string(),
                seconds: 60,
                recurrence: None,
            }),
        )
        .await
        .unwrap();

        // Bob cannot edit Alice's reminder.
        let err = update_reminder(
            State(state.clone()),
            bob,
            Path(created.id),
            Json(UpdateReminderRequest {
                message: "Hijacked".to_string(),
                seconds: 60,
                recurrence: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        // An empty message is rejected before anything is written.
        let err = update_reminder(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(UpdateReminderRequest {
                message: "   ".to_string(),
                seconds: 60,
                recurrence: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        // Alice's own edit lands, including the recurrence change.
        let updated = update_reminder(
            State(state.clone()),
            alice,
            Path(created.id),
            Json(UpdateReminderRequest {
                message: "Water the plants twice".to_string(),
                seconds: 7200,
                recurrence: Some("weekly".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.message, "Water the plants twice");
        assert_eq!(updated.recurrence, "weekly");
        assert!(
            updated.trigger_at > created.trigger_at,
            "the new delay should push the fire time out"
        );
    }

    #[tokio::test]
    async fn test_reminder_history_api() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::named("alice", "acct_alice");

        // Create 2 reminders
        let r1 = create_reminder(
            State(state.clone()),
            alice.clone(),
            Json(CreateReminderRequest {
                room_id: Some("!room:1".to_string()),
                message: "Reminder 1".to_string(),
                seconds: 60,
                recurrence: None,
            }),
        )
        .await
        .unwrap();

        let r2 = create_reminder(
            State(state.clone()),
            alice.clone(),
            Json(CreateReminderRequest {
                room_id: Some("!room:1".to_string()),
                message: "Reminder 2".to_string(),
                seconds: 120,
                recurrence: None,
            }),
        )
        .await
        .unwrap();

        // Cancel r1 -> moves to history
        delete_reminder(State(state.clone()), alice.clone(), Path(r1.id))
            .await
            .unwrap();

        // Query pending -> only r2
        let pending = list_reminders(
            State(state.clone()),
            alice.clone(),
            Query(ReminderQuery {
                room_id: None,
                page: Some(1),
                limit: Some(10),
                status: Some("pending".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(pending.items.len(), 1);
        assert_eq!(pending.items[0].id, r2.id);
        assert_eq!(pending.total, 1);
        assert!(!pending.has_more);

        // Query history -> r1
        let history = list_reminders(
            State(state.clone()),
            alice.clone(),
            Query(ReminderQuery {
                room_id: None,
                page: Some(1),
                limit: Some(10),
                status: Some("history".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].id, r1.id);
        assert_eq!(history.items[0].status, "cancelled");

        // Delete r1 again -> permanently deletes from history
        let status = delete_reminder(State(state.clone()), alice.clone(), Path(r1.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);

        let history_after_del = list_reminders(
            State(state.clone()),
            alice.clone(),
            Query(ReminderQuery {
                room_id: None,
                page: Some(1),
                limit: Some(10),
                status: Some("history".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(history_after_del.items.len(), 0);

        // Cancel r2 and test clear_reminder_history
        delete_reminder(State(state.clone()), alice.clone(), Path(r2.id))
            .await
            .unwrap();
        let cleared = clear_reminder_history(State(state.clone()), alice.clone())
            .await
            .unwrap();
        assert_eq!(cleared, StatusCode::NO_CONTENT);

        let history_cleared = list_reminders(
            State(state),
            alice,
            Query(ReminderQuery {
                room_id: None,
                page: Some(1),
                limit: Some(10),
                status: Some("history".to_string()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(history_cleared.items.len(), 0);
    }
}
