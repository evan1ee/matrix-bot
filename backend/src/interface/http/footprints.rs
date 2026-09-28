use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

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
            owner_id: fp.owner_id.into_inner(),
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
    if !crate::interface::http::rate_limit::check_geocode_limit(&user.account_id) {
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
