use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::habits::models::{Habit, HabitStatsSummary, HabitWithStatus, HeatmapDay};
use crate::modules::habits::repository::HabitsRepository;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListHabitsQuery {
    pub today: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateHabitRequest {
    pub name: String,
    pub description: Option<String>,
    pub frequency: Option<String>,
    pub category: Option<String>,
    pub target_days_per_week: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateHabitRequest {
    pub name: String,
    pub description: Option<String>,
    pub frequency: Option<String>,
    pub category: Option<String>,
    pub target_days_per_week: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ToggleHabitRequest {
    pub date: Option<String>,
    pub note: Option<String>,
}

fn get_today_string(override_today: Option<&str>) -> String {
    override_today.map(|s| s.to_string()).unwrap_or_else(|| {
        chrono::Utc::now()
            .naive_utc()
            .date()
            .format("%Y-%m-%d")
            .to_string()
    })
}

#[utoipa::path(
    get,
    tag = "habits",
    path = "/api/v1/habits",
    params(ListHabitsQuery),
    responses(
        (status = 200, description = "List habits with today's status", body = Vec<HabitWithStatus>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_habits(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(params): Query<ListHabitsQuery>,
) -> Result<Json<Vec<HabitWithStatus>>, ApiError> {
    let today = get_today_string(params.today.as_deref());
    let items =
        HabitsRepository::find_all_with_status(state.db.sqlite_pool(), &user.account_id, &today)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(items))
}

#[utoipa::path(
    post,
    tag = "habits",
    path = "/api/v1/habits",
    request_body = CreateHabitRequest,
    responses(
        (status = 200, description = "Habit created", body = Habit),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_habit(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<CreateHabitRequest>,
) -> Result<Json<Habit>, ApiError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("Habit name is required."));
    }
    if name.len() > 100 {
        return Err(ApiError::bad_request(
            "Habit name must be 100 characters or fewer.",
        ));
    }

    let habit = HabitsRepository::create_habit(
        state.db.sqlite_pool(),
        &user.account_id,
        name,
        payload.description.as_deref(),
        payload.frequency.as_deref().unwrap_or("daily"),
        payload.category.as_deref().unwrap_or("wellness"),
        payload.target_days_per_week.unwrap_or(7),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::HabitCreated(habit.clone()));

    Ok(Json(habit))
}

#[utoipa::path(
    put,
    tag = "habits",
    path = "/api/v1/habits/{id}",
    params(("id" = i64, Path, description = "Habit ID")),
    request_body = UpdateHabitRequest,
    responses(
        (status = 200, description = "Habit updated", body = Habit),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Habit not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_habit(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateHabitRequest>,
) -> Result<Json<Habit>, ApiError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("Habit name cannot be empty."));
    }
    if name.len() > 100 {
        return Err(ApiError::bad_request(
            "Habit name must be 100 characters or fewer.",
        ));
    }

    let habit = HabitsRepository::update_habit(
        state.db.sqlite_pool(),
        id,
        &user.account_id,
        name,
        payload.description.as_deref(),
        payload.frequency.as_deref().unwrap_or("daily"),
        payload.category.as_deref().unwrap_or("wellness"),
        payload.target_days_per_week.unwrap_or(7),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found(format!("Habit #{} not found", id)))?;

    state
        .broadcaster
        .send(SystemEvent::HabitUpdated(habit.clone()));

    Ok(Json(habit))
}

#[utoipa::path(
    delete,
    tag = "habits",
    path = "/api/v1/habits/{id}",
    params(("id" = i64, Path, description = "Habit ID")),
    responses(
        (status = 200, description = "Habit deleted"),
        (status = 404, description = "Habit not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_habit(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let deleted = HabitsRepository::delete_habit(state.db.sqlite_pool(), id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !deleted {
        return Err(ApiError::not_found(format!("Habit #{} not found", id)));
    }

    state.broadcaster.send(SystemEvent::HabitDeleted {
        id,
        owner_id: user.account_id.clone(),
    });

    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    tag = "habits",
    path = "/api/v1/habits/{id}/toggle",
    params(("id" = i64, Path, description = "Habit ID")),
    request_body = ToggleHabitRequest,
    responses(
        (status = 200, description = "Habit log toggled for the given date"),
        (status = 404, description = "Habit not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn toggle_habit_log(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<ToggleHabitRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let habit = HabitsRepository::find_by_id(state.db.sqlite_pool(), id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found(format!("Habit #{} not found", id)))?;

    let log_date = get_today_string(payload.date.as_deref());

    let new_completed = HabitsRepository::toggle_log(
        state.db.sqlite_pool(),
        habit.id,
        &user.account_id,
        &log_date,
        payload.note.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::HabitLogToggled {
        habit_id: habit.id,
        owner_id: user.account_id.clone(),
        log_date: log_date.clone(),
        completed: new_completed,
    });

    Ok(Json(serde_json::json!({
        "habit_id": habit.id,
        "log_date": log_date,
        "completed": new_completed,
    })))
}

#[utoipa::path(
    get,
    tag = "habits",
    path = "/api/v1/habits/heatmap",
    responses(
        (status = 200, description = "Daily completion counts for the heatmap", body = Vec<HeatmapDay>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_heatmap(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<Vec<HeatmapDay>>, ApiError> {
    let days = HabitsRepository::get_heatmap(state.db.sqlite_pool(), &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(days))
}

#[utoipa::path(
    get,
    tag = "habits",
    path = "/api/v1/habits/stats",
    // `utoipa::path` defaults the operation id to the function name, and
    // `bookmarks::get_stats` already claims plain "get_stats" — an explicit id
    // keeps the generated OpenAPI operations (and so the TS client) from
    // colliding. See `portfolio::summary` for the same pattern.
    operation_id = "habits_get_stats",
    params(ListHabitsQuery),
    responses(
        (status = 200, description = "Habit completion stats summary", body = HabitStatsSummary),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_stats(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(params): Query<ListHabitsQuery>,
) -> Result<Json<HabitStatsSummary>, ApiError> {
    let today = get_today_string(params.today.as_deref());
    let stats =
        HabitsRepository::get_summary_stats(state.db.sqlite_pool(), &user.account_id, &today)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(stats))
}
