use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

/// Pattern for recurring reminders (none, daily, weekly, monthly, or yearly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RecurrencePattern {
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl RecurrencePattern {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            RecurrencePattern::None => "none",
            RecurrencePattern::Daily => "daily",
            RecurrencePattern::Weekly => "weekly",
            RecurrencePattern::Monthly => "monthly",
            RecurrencePattern::Yearly => "yearly",
        }
    }

    #[allow(dead_code)]
    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "daily" | "day" => RecurrencePattern::Daily,
            "weekly" | "week" => RecurrencePattern::Weekly,
            "monthly" | "month" => RecurrencePattern::Monthly,
            "yearly" | "year" => RecurrencePattern::Yearly,
            _ => RecurrencePattern::None,
        }
    }
}

/// Status of a reminder in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ReminderStatus {
    Pending,
    Triggered,
    Cancelled,
    Failed,
}

impl ReminderStatus {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderStatus::Pending => "pending",
            ReminderStatus::Triggered => "triggered",
            ReminderStatus::Cancelled => "cancelled",
            ReminderStatus::Failed => "failed",
        }
    }

    #[allow(dead_code)]
    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "triggered" => ReminderStatus::Triggered,
            "cancelled" => ReminderStatus::Cancelled,
            "failed" => ReminderStatus::Failed,
            _ => ReminderStatus::Pending,
        }
    }
}

/// A reminder stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Reminder {
    pub id: i64,
    pub room_id: String,
    pub owner_id: String,
    pub sender_name: String,
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub recurrence: String,
    pub status: String,
    pub retry_count: i64,
    /// Which module scheduled this reminder, if any — see [`ReminderSource`].
    pub source_kind: Option<String>,
    /// The id of the row in that module which scheduled this reminder, so
    /// changing or retiring that row can reschedule or cancel it.
    pub source_id: Option<i64>,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::reminders::Reminder> for Reminder {
    fn from(r: crate::domain::reminders::Reminder) -> Self {
        Self {
            id: r.id,
            room_id: r.room_id,
            owner_id: r.owner_id.into_inner(),
            sender_name: r.sender_name,
            message: r.message,
            trigger_at: r.trigger_at,
            recurrence: r.recurrence.as_str().to_string(),
            status: r.status.as_str().to_string(),
            retry_count: r.retry_count,
            source_kind: r.source_kind.map(|s| s.as_str().to_string()),
            source_id: r.source_id,
            created_at: r.created_at,
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

    let domain_items = state
        .services
        .reminders
        .list(
            &user.account_id,
            query.status.as_deref(),
            query.room_id.as_deref(),
            limit,
            offset,
        )
        .await?;

    let total = state
        .services
        .reminders
        .count(
            &user.account_id,
            query.status.as_deref(),
            query.room_id.as_deref(),
        )
        .await?;

    let items = domain_items
        .into_iter()
        .map(Reminder::from)
        .collect::<Vec<_>>();
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
        .map(crate::domain::reminders::RecurrencePattern::from_str_or_default)
        .unwrap_or(crate::domain::reminders::RecurrencePattern::None);

    let room_id = state
        .services
        .rooms
        .resolve_target_room(&user.account_id, payload.room_id.as_deref())
        .await;

    let input = crate::domain::reminders::NewReminder {
        message: payload.message,
        trigger_at,
        room_id,
        owner_name: user.display_name,
        recurrence,
        source: None,
    };

    let reminder = state
        .services
        .reminders
        .create(&user.account_id, input)
        .await?;

    Ok(Json(reminder.into()))
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
    if payload.message.len() > crate::domain::reminders::MAX_MESSAGE_LEN {
        return Err(ApiError::bad_request(format!(
            "Message exceeds {} character limit",
            crate::domain::reminders::MAX_MESSAGE_LEN
        )));
    }

    let trigger_at = Utc::now() + Duration::seconds(payload.seconds as i64);
    let recurrence = payload
        .recurrence
        .as_deref()
        .map(crate::domain::reminders::RecurrencePattern::from_str_or_default)
        .unwrap_or(crate::domain::reminders::RecurrencePattern::None);

    let updated = state
        .services
        .reminders
        .update_pending(
            &user.account_id,
            id,
            &payload.message,
            trigger_at,
            recurrence,
        )
        .await?;

    Ok(Json(updated.into()))
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
    state
        .services
        .reminders
        .delete_or_cancel(&user.account_id, id)
        .await?;

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
    state
        .services
        .reminders
        .clear_history(&user.account_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    #[tokio::test]
    async fn test_reminder_api_crud_and_isolation() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
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
