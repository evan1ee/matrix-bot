use axum::{
    extract::{Path, State},
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::life::models::{
    normalize_mood, LifeDayLog, LifeHorizonDay, LifeProfile, LifeProfileStats, LifeYearView,
    UpsertLifeDayLogInput, UpsertLifeProfileInput,
};
use crate::modules::life::repository::LifeRepository;

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteDayResponse {
    pub success: bool,
    pub date: String,
}

fn get_today_string() -> String {
    chrono::Utc::now().naive_utc().date().format("%Y-%m-%d").to_string()
}

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

pub async fn upsert_profile(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<UpsertLifeProfileInput>,
) -> Result<Json<LifeProfile>, ApiError> {
    let birth = payload.birth_date.trim();
    if NaiveDate::parse_from_str(birth, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request("Invalid birth date format. Use YYYY-MM-DD."));
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

pub async fn get_horizon(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<Vec<LifeHorizonDay>>, ApiError> {
    let horizon = LifeRepository::get_horizon_days(&state.db.pool, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(horizon))
}

pub async fn upsert_day(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<UpsertLifeDayLogInput>,
) -> Result<Json<LifeDayLog>, ApiError> {
    let date_str = payload.log_date.trim();
    if NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request("Invalid log date format. Use YYYY-MM-DD."));
    }

    let mood = normalize_mood(&payload.mood)
        .ok_or_else(|| ApiError::bad_request("Invalid mood. Choose from amazing, good, neutral, low, rough."))?;

    if let Some(ref note) = payload.note {
        if note.len() > 500 {
            return Err(ApiError::bad_request("Note exceeds 500 characters limit."));
        }
    }

    let activities_json = match payload.activities {
        Some(ref acts) => serde_json::to_string(acts).ok(),
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
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::LifeDayLogged(log.clone()));

    Ok(Json(log))
}

pub async fn get_day(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(date): Path<String>,
) -> Result<Json<Option<LifeDayLog>>, ApiError> {
    if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request("Invalid date format. Use YYYY-MM-DD."));
    }

    let log = LifeRepository::get_day_log(&state.db.pool, &user.account_id, &date)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(log))
}

pub async fn delete_day(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(date): Path<String>,
) -> Result<Json<DeleteDayResponse>, ApiError> {
    if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request("Invalid date format. Use YYYY-MM-DD."));
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
    use sqlx::SqlitePool;

    async fn create_test_state() -> (ApiState, SqlitePool) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
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
            }),
        )
        .await
        .unwrap();
        assert_eq!(day.mood, "amazing");

        // 4. Get Year View
        let year_view = get_year_view(user.clone(), State(state.clone()), Path(2026))
            .await
            .unwrap();
        assert_eq!(year_view.total_logged_days, 1);

        // 5. Delete Day
        let del = delete_day(user.clone(), State(state.clone()), Path("2026-09-04".to_string()))
            .await
            .unwrap();
        assert!(del.success);
    }
}
