use crate::db::DbPool;
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::life::models::{
    normalize_mood, LifeDayLog, LifeHorizonDay, LifeProfile, LifeProfileStats, LifeYearView,
    UpdateTimeAllocationInput, UpsertLifeDayLogInput, UpsertLifeProfileInput,
};
use crate::modules::life::repository::LifeRepository;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeleteDayResponse {
    pub success: bool,
    pub date: String,
}

fn get_today_string() -> String {
    chrono::Utc::now()
        .naive_utc()
        .date()
        .format("%Y-%m-%d")
        .to_string()
}

#[utoipa::path(
    get,
    tag = "life",
    path = "/api/v1/life/profile",
    responses(
        (status = 200, description = "Life profile and computed stats", body = LifeProfileStats),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_profile_and_stats(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<LifeProfileStats>, ApiError> {
    let today = get_today_string();
    let stats = LifeRepository::get_stats(&state.db.pool, &user.account_id, &today)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(stats))
}

/// Also mounted as `POST /api/v1/life/profile` for the same handler — utoipa
/// documents one method per function, so this covers the `PUT` form.
#[utoipa::path(
    put,
    tag = "life",
    path = "/api/v1/life/profile",
    request_body = UpsertLifeProfileInput,
    responses(
        (status = 200, description = "Profile created or updated", body = LifeProfile),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn upsert_profile(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<UpsertLifeProfileInput>,
) -> Result<Json<LifeProfile>, ApiError> {
    let birth = payload.birth_date.trim();
    if NaiveDate::parse_from_str(birth, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(
            "Invalid birth date format. Use YYYY-MM-DD.",
        ));
    }

    let lifespan = payload.expected_lifespan_years.unwrap_or(80).clamp(40, 130);

    let profile = LifeRepository::upsert_profile(&state.db.pool, &user.account_id, birth, lifespan)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::LifeProfileUpdated {
        owner_id: user.account_id.clone(),
        profile: profile.clone(),
    });

    Ok(Json(profile))
}

#[utoipa::path(
    get,
    tag = "life",
    path = "/api/v1/life/year/{year}",
    params(("year" = i64, Path, description = "Calendar year")),
    responses(
        (status = 200, description = "All logged days for the year", body = LifeYearView),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_year_view(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(year): Path<i64>,
) -> Result<Json<LifeYearView>, ApiError> {
    if !(1900..=2200).contains(&year) {
        return Err(ApiError::bad_request("Year must be between 1900 and 2200."));
    }

    let view = LifeRepository::get_year_view(&state.db.pool, &user.account_id, year)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(view))
}

#[utoipa::path(
    get,
    tag = "life",
    path = "/api/v1/life/horizon",
    responses(
        (status = 200, description = "Upcoming days on the life horizon", body = Vec<LifeHorizonDay>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_horizon(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<Vec<LifeHorizonDay>>, ApiError> {
    let horizon = LifeRepository::get_horizon_days(&state.db.pool, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(horizon))
}

#[utoipa::path(
    post,
    tag = "life",
    path = "/api/v1/life/days",
    request_body = UpsertLifeDayLogInput,
    responses(
        (status = 200, description = "Day logged", body = LifeDayLog),
        (status = 400, description = "Bad request, or day already logged", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn upsert_day(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<UpsertLifeDayLogInput>,
) -> Result<Json<LifeDayLog>, ApiError> {
    let date_str = payload.log_date.trim();
    if NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(
            "Invalid log date format. Use YYYY-MM-DD.",
        ));
    }

    let mood = normalize_mood(&payload.mood).ok_or_else(|| {
        ApiError::bad_request("Invalid mood. Choose from amazing, good, neutral, low, rough.")
    })?;

    let existing = LifeRepository::get_day_log(&state.db.pool, &user.account_id, date_str)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(ref ex) = existing {
        if !ex.mood.is_empty() {
            return Err(ApiError::bad_request(
                "This day has already been logged and cannot be edited. It can only be cleared.",
            ));
        }
    }

    if let Some(ref note) = payload.note {
        if note.len() > 500 {
            return Err(ApiError::bad_request("Note exceeds 500 characters limit."));
        }
    }

    let activities_json = match payload.activities {
        Some(ref acts) => {
            if acts.len() > 50 {
                return Err(ApiError::bad_request("Activities cannot exceed 50 items."));
            }
            for act in acts {
                if act.len() > 100 {
                    return Err(ApiError::bad_request(
                        "Activity item exceeds 100 characters.",
                    ));
                }
            }
            serde_json::to_string(acts).ok()
        }
        None => None,
    };

    let time_allocation_json = match payload.time_allocation {
        Some(ref blocks) => {
            if blocks.len() > 288 {
                return Err(ApiError::bad_request(
                    "Time allocation cannot exceed 288 blocks.",
                ));
            }
            serde_json::to_string(blocks).ok()
        }
        None => None,
    };

    let log = LifeRepository::upsert_day_log(
        &state.db.pool,
        &user.account_id,
        date_str,
        mood,
        payload.weather.as_deref(),
        activities_json.as_deref(),
        payload.note.as_deref(),
        time_allocation_json.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::LifeDayLogged(log.clone()));

    Ok(Json(log))
}

#[utoipa::path(
    put,
    tag = "life",
    path = "/api/v1/life/days/{date}/time",
    params(("date" = String, Path, description = "Day, as YYYY-MM-DD")),
    request_body = UpdateTimeAllocationInput,
    responses(
        (status = 200, description = "Time allocation updated", body = LifeDayLog),
        (status = 400, description = "Bad request, or day is sealed", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_time_allocation(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(date): Path<String>,
    Json(payload): Json<UpdateTimeAllocationInput>,
) -> Result<Json<LifeDayLog>, ApiError> {
    if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(
            "Invalid date format. Use YYYY-MM-DD.",
        ));
    }

    if payload.time_allocation.len() > 288 {
        return Err(ApiError::bad_request(
            "Time allocation cannot exceed 288 blocks.",
        ));
    }

    let today = get_today_string();
    let existing = LifeRepository::get_day_log(&state.db.pool, &user.account_id, &date)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(ref ex) = existing {
        if !ex.mood.is_empty() && date < today {
            return Err(ApiError::bad_request(
                "Past sealed days cannot be edited. Only today can be updated intraday.",
            ));
        }
    }

    let time_allocation_json = serde_json::to_string(&payload.time_allocation)
        .map_err(|e| ApiError::bad_request(format!("Invalid time allocation format: {e}")))?;

    let log = LifeRepository::update_time_allocation(
        &state.db.pool,
        &user.account_id,
        &date,
        &time_allocation_json,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::LifeDayLogged(log.clone()));

    Ok(Json(log))
}

#[utoipa::path(
    get,
    tag = "life",
    path = "/api/v1/life/days/{date}",
    params(("date" = String, Path, description = "Day, as YYYY-MM-DD")),
    responses(
        (status = 200, description = "The day's log, if any", body = Option<LifeDayLog>),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_day(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(date): Path<String>,
) -> Result<Json<Option<LifeDayLog>>, ApiError> {
    if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(
            "Invalid date format. Use YYYY-MM-DD.",
        ));
    }

    let log = LifeRepository::get_day_log(&state.db.pool, &user.account_id, &date)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(log))
}

#[utoipa::path(
    delete,
    tag = "life",
    path = "/api/v1/life/days/{date}",
    params(("date" = String, Path, description = "Day, as YYYY-MM-DD")),
    responses(
        (status = 200, description = "Day cleared, or was already empty", body = DeleteDayResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_day(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(date): Path<String>,
) -> Result<Json<DeleteDayResponse>, ApiError> {
    if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(
            "Invalid date format. Use YYYY-MM-DD.",
        ));
    }

    let deleted = LifeRepository::delete_day_log(&state.db.pool, &user.account_id, &date)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if deleted {
        state.broadcaster.send(SystemEvent::LifeDayCleared {
            owner_id: user.account_id.clone(),
            date: date.clone(),
        });
    }

    Ok(Json(DeleteDayResponse {
        success: deleted,
        date,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema_life::run_life_schema;
    use crate::db::Database;
    use crate::events::EventBroadcaster;

    async fn create_test_state() -> (ApiState, DbPool) {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_life_schema(&pool).await.unwrap();
        let db = Database { pool: pool.clone() };
        let broadcaster = EventBroadcaster::new(10);
        let client = reqwest::Client::new();
        let state = ApiState::new(
            db,
            broadcaster,
            Some("secret".to_string()),
            "https://matrix.org".to_string(),
            client,
        );
        (state, pool)
    }

    #[tokio::test]
    async fn test_profile_and_day_api_flow() {
        let (state, _) = create_test_state().await;
        let user = AuthUser {
            account_id: "user_api_1".to_string(),
            display_name: "User Api 1".to_string(),
            session_id_hash: "hash_1".to_string(),
            last_verified_at: chrono::Utc::now(),
            restricted_until: None,
        };

        // 1. Initial stats
        let stats = get_profile_and_stats(user.clone(), State(state.clone()))
            .await
            .unwrap();
        assert!(stats.profile.is_none());

        // 2. Set profile
        let profile = upsert_profile(
            user.clone(),
            State(state.clone()),
            Json(UpsertLifeProfileInput {
                birth_date: "1998-04-20".to_string(),
                expected_lifespan_years: Some(85),
            }),
        )
        .await
        .unwrap();
        assert_eq!(profile.birth_date, "1998-04-20");

        // 3. Upsert Day
        let day = upsert_day(
            user.clone(),
            State(state.clone()),
            Json(UpsertLifeDayLogInput {
                log_date: "2026-09-04".to_string(),
                mood: "amazing".to_string(),
                weather: Some("sunny".to_string()),
                activities: Some(vec!["workout".to_string(), "reading".to_string()]),
                note: Some("Great progress today!".to_string()),
                time_allocation: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(day.mood, "amazing");

        // 3b. Attempting to edit an already logged day is rejected
        let edit_err = upsert_day(
            user.clone(),
            State(state.clone()),
            Json(UpsertLifeDayLogInput {
                log_date: "2026-09-04".to_string(),
                mood: "good".to_string(),
                weather: None,
                activities: None,
                note: None,
                time_allocation: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(edit_err.status, axum::http::StatusCode::BAD_REQUEST);
        assert!(edit_err.message.contains("cannot be edited"));

        // 3c. Update time allocation intraday
        let mut blocks = vec![None; 96];
        blocks[0] = Some("sleep".to_string());
        blocks[1] = Some("sleep".to_string());
        blocks[36] = Some("deep_work".to_string());
        blocks[37] = Some("deep_work".to_string());
        let updated_time = update_time_allocation(
            user.clone(),
            State(state.clone()),
            Path(get_today_string()),
            Json(UpdateTimeAllocationInput {
                time_allocation: blocks,
            }),
        )
        .await
        .unwrap();
        assert!(updated_time.time_allocation.is_some());

        // 4. Get Year View
        let year_view = get_year_view(user.clone(), State(state.clone()), Path(2026))
            .await
            .unwrap();
        assert_eq!(year_view.total_logged_days, 2);

        // 4b. Verify Stats includes deep work and sleep hours
        let stats = get_profile_and_stats(user.clone(), State(state.clone()))
            .await
            .unwrap();
        assert_eq!(stats.total_deep_work_hours, 0.5);
        assert_eq!(stats.average_sleep_hours, 0.5);

        // 5. Delete Day
        let del = delete_day(
            user.clone(),
            State(state.clone()),
            Path("2026-09-04".to_string()),
        )
        .await
        .unwrap();
        assert!(del.success);
    }
}
