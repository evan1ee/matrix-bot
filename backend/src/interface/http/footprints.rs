use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;
use utoipa::{IntoParams, ToSchema};

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Footprint {
    pub id: i64,
    pub owner_id: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub stay_duration_days: i64,
    pub spend_amount_minor: i64,
    pub spend_currency: String,
    pub spend_currency_exponent: i64,
    pub rating: i64,
    pub category: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<crate::domain::footprints::Footprint> for Footprint {
    fn from(fp: crate::domain::footprints::Footprint) -> Self {
        Self {
            id: fp.id,
            owner_id: fp.owner_id.to_string(),
            title: fp.title,
            latitude: fp.latitude,
            longitude: fp.longitude,
            country: fp.country,
            city: fp.city,
            address: fp.address,
            arrival_date: fp.arrival_date,
            departure_date: fp.departure_date,
            stay_duration_days: fp.stay_duration_days,
            spend_amount_minor: fp.spend_amount_minor,
            spend_currency: fp.spend_currency,
            spend_currency_exponent: fp.spend_currency_exponent,
            rating: fp.rating,
            category: fp.category,
            note: fp.note,
            created_at: fp.created_at,
            updated_at: fp.updated_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateFootprintRequest {
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub spend_amount: Option<String>,
    pub spend_currency: Option<String>,
    pub rating: Option<i64>,
    pub category: Option<String>,
    pub note: Option<String>,
}

impl From<CreateFootprintRequest> for crate::application::footprints::CreateFootprintInput {
    fn from(req: CreateFootprintRequest) -> Self {
        Self {
            title: req.title,
            latitude: req.latitude,
            longitude: req.longitude,
            country: req.country,
            city: req.city,
            address: req.address,
            arrival_date: req.arrival_date,
            departure_date: req.departure_date,
            spend_amount: req.spend_amount,
            spend_currency: req.spend_currency,
            rating: req.rating,
            category: req.category,
            note: req.note,
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateFootprintRequest {
    pub title: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: Option<String>,
    pub departure_date: Option<String>,
    pub spend_amount: Option<String>,
    pub spend_currency: Option<String>,
    pub rating: Option<i64>,
    pub category: Option<String>,
    pub note: Option<String>,
}

impl From<UpdateFootprintRequest> for crate::application::footprints::UpdateFootprintInput {
    fn from(req: UpdateFootprintRequest) -> Self {
        Self {
            title: req.title,
            latitude: req.latitude,
            longitude: req.longitude,
            country: req.country,
            city: req.city,
            address: req.address,
            arrival_date: req.arrival_date,
            departure_date: req.departure_date,
            spend_amount: req.spend_amount,
            spend_currency: req.spend_currency,
            rating: req.rating,
            category: req.category,
            note: req.note,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FootprintCurrencySpend {
    pub currency: String,
    pub currency_exponent: i64,
    pub total_spend_minor: i64,
}

impl From<crate::domain::footprints::FootprintCurrencySpend> for FootprintCurrencySpend {
    fn from(s: crate::domain::footprints::FootprintCurrencySpend) -> Self {
        Self {
            currency: s.currency,
            currency_exponent: s.currency_exponent,
            total_spend_minor: s.total_spend_minor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FootprintsSummary {
    pub total_places: i64,
    pub total_countries: i64,
    pub total_days_traveled: i64,
    pub spend_by_currency: Vec<FootprintCurrencySpend>,
}

impl From<crate::domain::footprints::FootprintsSummary> for FootprintsSummary {
    fn from(s: crate::domain::footprints::FootprintsSummary) -> Self {
        Self {
            total_places: s.total_places,
            total_countries: s.total_countries,
            total_days_traveled: s.total_days_traveled,
            spend_by_currency: s.spend_by_currency.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListFootprintsQuery {
    pub country: Option<String>,
    pub category: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct GeocodeQuery {
    pub query: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

#[utoipa::path(
    get,
    tag = "footprints",
    path = "/api/v1/footprints",
    params(ListFootprintsQuery),
    responses(
        (status = 200, description = "List footprints", body = Vec<Footprint>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_footprints(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(params): Query<ListFootprintsQuery>,
) -> Result<Json<Vec<Footprint>>, ApiError> {
    let items = state
        .services
        .footprints
        .list(
            &user.account_id,
            params.country.as_deref(),
            params.category.as_deref(),
            params.search.as_deref(),
        )
        .await?;

    let dtos = items.into_iter().map(Footprint::from).collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    get,
    tag = "footprints",
    path = "/api/v1/footprints/{id}",
    params(("id" = i64, Path, description = "Footprint ID")),
    responses(
        (status = 200, description = "Footprint found", body = Footprint),
        (status = 404, description = "Footprint not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Footprint>, ApiError> {
    let fp = state.services.footprints.get(&user.account_id, id).await?;
    Ok(Json(fp.into()))
}

#[utoipa::path(
    post,
    tag = "footprints",
    path = "/api/v1/footprints",
    request_body = CreateFootprintRequest,
    responses(
        (status = 200, description = "Footprint created", body = Footprint),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<CreateFootprintRequest>,
) -> Result<Json<Footprint>, ApiError> {
    let created = state
        .services
        .footprints
        .create(&user.account_id, payload.into())
        .await?;
    Ok(Json(created.into()))
}

#[utoipa::path(
    put,
    tag = "footprints",
    path = "/api/v1/footprints/{id}",
    params(("id" = i64, Path, description = "Footprint ID")),
    request_body = UpdateFootprintRequest,
    responses(
        (status = 200, description = "Footprint updated", body = Footprint),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Footprint not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateFootprintRequest>,
) -> Result<Json<Footprint>, ApiError> {
    let updated = state
        .services
        .footprints
        .update(&user.account_id, id, payload.into())
        .await?;
    Ok(Json(updated.into()))
}

#[utoipa::path(
    delete,
    tag = "footprints",
    path = "/api/v1/footprints/{id}",
    params(("id" = i64, Path, description = "Footprint ID")),
    responses(
        (status = 200, description = "Footprint deleted"),
        (status = 404, description = "Footprint not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .services
        .footprints
        .delete(&user.account_id, id)
        .await?;
    Ok(Json(serde_json::json!({ "success": true, "id": id })))
}

#[utoipa::path(
    get,
    tag = "footprints",
    path = "/api/v1/footprints/stats",
    responses(
        (status = 200, description = "Footprint travel stats", body = FootprintsSummary),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_summary_stats(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<FootprintsSummary>, ApiError> {
    let stats = state
        .services
        .footprints
        .summary_stats(&user.account_id)
        .await?;
    Ok(Json(stats.into()))
}

/// Reverse or forward geocoding proxy using OpenStreetMap Nominatim.
#[utoipa::path(
    get,
    tag = "footprints",
    path = "/api/v1/footprints/geocode",
    params(GeocodeQuery),
    responses(
        (status = 200, description = "Geocoding result, proxied verbatim from Nominatim"),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn geocode_proxy(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(params): Query<GeocodeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !crate::interface::http::rate_limit::check_geocode_limit(&user.account_id.to_string()) {
        return Err(ApiError::too_many_requests(
            "Geocoding rate limit exceeded. Please wait a moment.",
        ));
    }

    if let (Some(lat), Some(lon)) = (params.lat, params.lon) {
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return Err(ApiError::bad_request(
                "Invalid latitude or longitude coordinates",
            ));
        }
        let resp = state
            .http_client
            .get("https://nominatim.openstreetmap.org/reverse")
            .header(
                reqwest::header::USER_AGENT,
                "matrix-bot/1.0 (contact: info@mbm.one)",
            )
            .query(&[
                ("lat", lat.to_string()),
                ("lon", lon.to_string()),
                ("format", "json".to_string()),
                ("addressdetails", "1".to_string()),
            ])
            .send()
            .await
            .map_err(|e| {
                warn!("Nominatim reverse geocode request failed: {}", e);
                ApiError::internal("Geocoding service unavailable".to_string())
            })?;

        let data: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        Ok(Json(data))
    } else if let Some(query) = params.query {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Json(serde_json::json!([])));
        }
        if q.len() > 200 {
            return Err(ApiError::bad_request(
                "Geocoding query exceeds 200 characters",
            ));
        }
        let resp = state
            .http_client
            .get("https://nominatim.openstreetmap.org/search")
            .header(
                reqwest::header::USER_AGENT,
                "matrix-bot/1.0 (contact: info@mbm.one)",
            )
            .query(&[
                ("q", q.to_string()),
                ("format", "json".to_string()),
                ("limit", "5".to_string()),
                ("addressdetails", "1".to_string()),
            ])
            .send()
            .await
            .map_err(|e| {
                warn!("Nominatim forward search request failed: {}", e);
                ApiError::internal("Geocoding search unavailable".to_string())
            })?;

        let data: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!([]));
        Ok(Json(data))
    } else {
        Err(ApiError::bad_request(
            "Must provide 'query' or 'lat' and 'lon'".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use axum::http::StatusCode;

    async fn setup_state() -> (ApiState, crate::domain::accounts::AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = crate::application::AppServices::new(&pool)
            .identity
            .provision_matrix_for_test("@fp_http:mbm.one")
            .await;
        let state = ApiState::new(
            Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        (state, owner)
    }

    #[tokio::test]
    async fn footprints_http_crud_and_stats_flow() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        // 1. Create
        let created = create_footprint(
            user.clone(),
            State(state.clone()),
            Json(CreateFootprintRequest {
                title: "Shibuya Crossing".to_string(),
                latitude: 35.6595,
                longitude: 139.7004,
                country: Some("Japan".to_string()),
                city: Some("Tokyo".to_string()),
                address: None,
                arrival_date: "2026-06-01".to_string(),
                departure_date: Some("2026-06-03".to_string()),
                spend_amount: Some("5000".to_string()),
                spend_currency: Some("JPY".to_string()),
                rating: Some(5),
                category: Some("sightseeing".to_string()),
                note: Some("Busy intersection".to_string()),
            }),
        )
        .await
        .expect("create_footprint")
        .0;
        assert_eq!(created.title, "Shibuya Crossing");
        assert_eq!(created.stay_duration_days, 2);
        assert_eq!(created.spend_amount_minor, 5000);

        // 2. Get
        let fetched = get_footprint(user.clone(), State(state.clone()), Path(created.id))
            .await
            .expect("get_footprint")
            .0;
        assert_eq!(fetched.id, created.id);

        // 3. Update
        let updated = update_footprint(
            user.clone(),
            State(state.clone()),
            Path(created.id),
            Json(UpdateFootprintRequest {
                title: Some("Shibuya Scramble".to_string()),
                latitude: None,
                longitude: None,
                country: None,
                city: None,
                address: None,
                arrival_date: None,
                departure_date: None,
                spend_amount: Some("6000".to_string()),
                spend_currency: None,
                rating: Some(4),
                category: None,
                note: None,
            }),
        )
        .await
        .expect("update_footprint")
        .0;
        assert_eq!(updated.title, "Shibuya Scramble");
        assert_eq!(updated.spend_amount_minor, 6000);
        assert_eq!(updated.rating, 4);

        // 4. List
        let list = list_footprints(
            user.clone(),
            State(state.clone()),
            Query(ListFootprintsQuery {
                country: Some("Japan".to_string()),
                category: None,
                search: Some("Scramble".to_string()),
            }),
        )
        .await
        .expect("list_footprints")
        .0;
        assert_eq!(list.len(), 1);

        // 5. Summary stats
        let stats = get_summary_stats(user.clone(), State(state.clone()))
            .await
            .expect("get_summary_stats")
            .0;
        assert_eq!(stats.total_places, 1);
        assert_eq!(stats.total_countries, 1);
        assert_eq!(stats.total_days_traveled, 2);
        assert_eq!(stats.spend_by_currency[0].total_spend_minor, 6000);

        // 6. Delete
        let deleted = delete_footprint(user.clone(), State(state.clone()), Path(created.id))
            .await
            .expect("delete_footprint")
            .0;
        assert_eq!(deleted["success"], true);

        let err = get_footprint(user, State(state), Path(created.id))
            .await
            .expect_err("deleted footprint must return 404");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn geocode_proxy_validates_inputs_without_network() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        // Missing query and lat/lon -> 400
        let err = geocode_proxy(
            user.clone(),
            State(state.clone()),
            Query(GeocodeQuery {
                query: None,
                lat: None,
                lon: None,
            }),
        )
        .await
        .expect_err("missing params");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        // Out-of-bounds coordinates -> 400
        let user2 = AuthUser::for_account(crate::domain::accounts::AccountId::from_seed("geo_2"));
        let err = geocode_proxy(
            user2,
            State(state.clone()),
            Query(GeocodeQuery {
                query: None,
                lat: Some(95.0),
                lon: Some(0.0),
            }),
        )
        .await
        .expect_err("invalid latitude");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        // Empty query returns empty JSON array without contacting external Nominatim
        let user3 = AuthUser::for_account(crate::domain::accounts::AccountId::from_seed("geo_3"));
        let empty = geocode_proxy(
            user3,
            State(state.clone()),
            Query(GeocodeQuery {
                query: Some("   ".to_string()),
                lat: None,
                lon: None,
            }),
        )
        .await
        .expect("blank query")
        .0;
        assert_eq!(empty, serde_json::json!([]));

        // Oversized query (>200 chars) -> 400
        let user4 = AuthUser::for_account(crate::domain::accounts::AccountId::from_seed("geo_4"));
        let err = geocode_proxy(
            user4,
            State(state),
            Query(GeocodeQuery {
                query: Some("q".repeat(201)),
                lat: None,
                lon: None,
            }),
        )
        .await
        .expect_err("oversized query");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }
}
