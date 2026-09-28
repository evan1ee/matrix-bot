use axum::{
    extract::{Path, State},
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct LifeProfile {
    pub owner_id: String,
    pub birth_date: String,
    pub expected_lifespan_years: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::domain::life::LifeProfile> for LifeProfile {
    fn from(p: crate::domain::life::LifeProfile) -> Self {
        Self {
            owner_id: p.owner_id.to_string(),
            birth_date: p.birth_date,
            expected_lifespan_years: p.expected_lifespan_years,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct LifeDayLog {
    pub id: i64,
    pub owner_id: String,
    pub log_date: String,
    pub mood: String,
    pub weather: Option<String>,
    pub activities: Option<String>,
    pub note: Option<String>,
    pub time_allocation: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::domain::life::LifeDayLog> for LifeDayLog {
    fn from(l: crate::domain::life::LifeDayLog) -> Self {
        Self {
            id: l.id,
            owner_id: l.owner_id.to_string(),
            log_date: l.log_date,
            mood: l.mood,
            weather: l.weather,
            activities: l.activities,
            note: l.note,
            time_allocation: l.time_allocation,
            created_at: l.created_at,
            updated_at: l.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertLifeProfileInput {
    pub birth_date: String,
    pub expected_lifespan_years: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertLifeDayLogInput {
    #[serde(alias = "date")]
    pub log_date: String,
    pub mood: String,
    pub weather: Option<String>,
    pub activities: Option<Vec<String>>,
    pub note: Option<String>,
    pub time_allocation: Option<Vec<Option<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateTimeAllocationInput {
    pub time_allocation: Vec<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct LifeHorizonDay {
    pub date: String,
    pub mood: String,
    pub time_allocation: Option<String>,
    pub has_activity: bool,
    pub activity_count: i64,
}

impl From<crate::domain::life::LifeHorizonDay> for LifeHorizonDay {
    fn from(d: crate::domain::life::LifeHorizonDay) -> Self {
        Self {
            date: d.date,
            mood: d.mood,
            time_allocation: d.time_allocation,
            has_activity: d.has_activity,
            activity_count: d.activity_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct LifeProfileStats {
    pub profile: Option<LifeProfile>,
    pub age_days: i64,
    pub total_expected_days: i64,
    pub days_lived_percent: f64,
    pub total_logged_days: i64,
    pub dominant_mood: Option<String>,
    pub total_deep_work_hours: f64,
    pub average_sleep_hours: f64,
    pub today_date: String,
}

impl From<crate::domain::life::LifeProfileStats> for LifeProfileStats {
    fn from(s: crate::domain::life::LifeProfileStats) -> Self {
        Self {
            profile: s.profile.map(LifeProfile::from),
            age_days: s.age_days,
            total_expected_days: s.total_expected_days,
            days_lived_percent: s.days_lived_percent,
            total_logged_days: s.total_logged_days,
            dominant_mood: s.dominant_mood,
            total_deep_work_hours: s.total_deep_work_hours,
            average_sleep_hours: s.average_sleep_hours,
            today_date: s.today_date,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct LifeYearView {
    pub year: i64,
    pub logs: Vec<LifeDayLog>,
    pub total_logged_days: i64,
}

impl From<crate::domain::life::LifeYearView> for LifeYearView {
    fn from(v: crate::domain::life::LifeYearView) -> Self {
        Self {
            year: v.year,
            logs: v.logs.into_iter().map(LifeDayLog::from).collect(),
            total_logged_days: v.total_logged_days,
        }
    }
}

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
    let stats = state
        .services
        .life
        .get_stats(&user.account_id, &today)
        .await?;

    Ok(Json(LifeProfileStats::from(stats)))
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

    let profile = state
        .services
        .life
        .upsert_profile(&user.account_id, birth, lifespan)
        .await?;

    Ok(Json(LifeProfile::from(profile)))
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

    let view = state
        .services
        .life
        .get_year_view(&user.account_id, year)
        .await?;

    Ok(Json(LifeYearView::from(view)))
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
    let horizon = state
        .services
        .life
        .get_horizon_days(&user.account_id)
        .await?;

    Ok(Json(
        horizon.into_iter().map(LifeHorizonDay::from).collect(),
    ))
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

    let existing = state
        .services
        .life
        .get_day_log(&user.account_id, date_str)
        .await?;
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

    let log = state
        .services
        .life
        .upsert_day_log(
            &user.account_id,
            date_str,
            &payload.mood,
            payload.weather.as_deref(),
            activities_json.as_deref(),
            payload.note.as_deref(),
            time_allocation_json.as_deref(),
        )
        .await?;

    Ok(Json(LifeDayLog::from(log)))
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
    let existing = state
        .services
        .life
        .get_day_log(&user.account_id, &date)
        .await?;

    if let Some(ref ex) = existing {
        if !ex.mood.is_empty() && date < today {
            return Err(ApiError::bad_request(
                "Past sealed days cannot be edited. Only today can be updated intraday.",
            ));
        }
    }

    let time_allocation_json = serde_json::to_string(&payload.time_allocation)
        .map_err(|e| ApiError::bad_request(format!("Invalid time allocation format: {e}")))?;

    let log = state
        .services
        .life
        .update_time_allocation(&user.account_id, &date, &time_allocation_json)
        .await?;

    Ok(Json(LifeDayLog::from(log)))
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

    let log = state
        .services
        .life
        .get_day_log(&user.account_id, &date)
        .await?;

    Ok(Json(log.map(LifeDayLog::from)))
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

    let deleted = state
        .services
        .life
        .delete_day_log(&user.account_id, &date)
        .await?;

    Ok(Json(DeleteDayResponse {
        success: deleted,
        date,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::DbPool;
    use crate::events::EventBroadcaster;

    async fn create_test_state() -> (ApiState, DbPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
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
            account_id: crate::domain::accounts::AccountId::new("user_api_1"),
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
