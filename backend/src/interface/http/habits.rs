use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::habits::HabitUpdate;
use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListHabitsQuery {
    pub today: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct Habit {
    pub id: i64,
    pub owner_id: String,
    pub name: String,
    pub description: Option<String>,
    pub frequency: String,
    pub category: String,
    pub target_days_per_week: i64,
    pub archived: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl From<crate::domain::habits::Habit> for Habit {
    fn from(h: crate::domain::habits::Habit) -> Self {
        Self {
            id: h.id,
            owner_id: h.owner_id.to_string(),
            name: h.name,
            description: h.description,
            frequency: h.frequency,
            category: h.category,
            target_days_per_week: h.target_days_per_week,
            archived: h.archived,
            created_at: h.created_at,
            updated_at: h.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct DayStatus {
    pub date: String,
    pub completed: bool,
}

impl From<crate::domain::habits::DayStatus> for DayStatus {
    fn from(d: crate::domain::habits::DayStatus) -> Self {
        Self {
            date: d.date,
            completed: d.completed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct HabitWithStatus {
    #[serde(flatten)]
    pub habit: Habit,
    pub completed_today: bool,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub total_completions: i64,
    pub last_7_days: Vec<DayStatus>,
}

impl From<crate::domain::habits::HabitWithStatus> for HabitWithStatus {
    fn from(h: crate::domain::habits::HabitWithStatus) -> Self {
        Self {
            habit: h.habit.into(),
            completed_today: h.completed_today,
            current_streak: h.current_streak,
            longest_streak: h.longest_streak,
            total_completions: h.total_completions,
            last_7_days: h.last_7_days.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct HeatmapDay {
    pub date: String,
    pub count: i64,
}

impl From<crate::domain::habits::HeatmapDay> for HeatmapDay {
    fn from(d: crate::domain::habits::HeatmapDay) -> Self {
        Self {
            date: d.date,
            count: d.count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct HabitStatsSummary {
    pub total_active_habits: i64,
    pub completed_today: i64,
    pub completion_rate_today_percent: i64,
    pub best_streak: i64,
    pub total_all_time_completions: i64,
}

impl From<crate::domain::habits::HabitStatsSummary> for HabitStatsSummary {
    fn from(s: crate::domain::habits::HabitStatsSummary) -> Self {
        Self {
            total_active_habits: s.total_active_habits,
            completed_today: s.completed_today,
            completion_rate_today_percent: s.completion_rate_today_percent,
            best_streak: s.best_streak,
            total_all_time_completions: s.total_all_time_completions,
        }
    }
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
    let items = state
        .services
        .habits
        .list_with_status(&user.account_id, &today)
        .await?;

    let dtos = items.into_iter().map(HabitWithStatus::from).collect();
    Ok(Json(dtos))
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
    let habit = state
        .services
        .habits
        .create_habit(
            &user.account_id,
            &payload.name,
            payload.description.as_deref(),
            payload.frequency.as_deref(),
            payload.category.as_deref(),
            payload.target_days_per_week,
        )
        .await?;

    Ok(Json(habit.into()))
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
    let habit = state
        .services
        .habits
        .update_habit(
            &user.account_id,
            id,
            HabitUpdate {
                name: &payload.name,
                description: payload.description.as_deref(),
                frequency: payload.frequency.as_deref(),
                category: payload.category.as_deref(),
                target_days_per_week: payload.target_days_per_week,
            },
        )
        .await?;

    Ok(Json(habit.into()))
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
    state
        .services
        .habits
        .delete_habit(&user.account_id, id)
        .await?;
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
    let log_date = get_today_string(payload.date.as_deref());

    let new_completed = state
        .services
        .habits
        .toggle_log(&user.account_id, id, &log_date, payload.note.as_deref())
        .await?;

    Ok(Json(serde_json::json!({
        "habit_id": id,
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
    let days = state.services.habits.get_heatmap(&user.account_id).await?;
    let dtos = days.into_iter().map(HeatmapDay::from).collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    get,
    tag = "habits",
    path = "/api/v1/habits/stats",
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
    let stats = state
        .services
        .habits
        .get_summary_stats(&user.account_id, &today)
        .await?;

    Ok(Json(stats.into()))
}
