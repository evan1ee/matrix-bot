use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use tracing::warn;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::footprints::models::{
    CreateFootprintRequest, Footprint, FootprintsSummary, UpdateFootprintRequest,
};
use crate::modules::footprints::repository::FootprintRepository;
use crate::modules::settings::repository::SettingsRepository;

#[derive(Debug, Deserialize)]
pub struct ListFootprintsQuery {
    pub country: Option<String>,
    pub category: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GeocodeQuery {
    pub query: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

pub async fn list_footprints(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(params): Query<ListFootprintsQuery>,
) -> Result<Json<Vec<Footprint>>, ApiError> {
    let items = FootprintRepository::list_by_owner(
        &state.db.pool,
        &user.account_id,
        params.country.as_deref(),
        params.category.as_deref(),
        params.search.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(items))
}

pub async fn get_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Footprint>, ApiError> {
    let fp = FootprintRepository::find_owned(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found(format!("Footprint #{} not found", id)))?;

    Ok(Json(fp))
}

pub async fn create_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<CreateFootprintRequest>,
) -> Result<Json<Footprint>, ApiError> {
    let title = payload.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("Title / Place Name is required".to_string()));
    }
    if title.len() > 500 {
        return Err(ApiError::bad_request("Title exceeds 500 characters".to_string()));
    }

    let default_currency = SettingsRepository::get_or_default(&state.db.pool, &user.account_id).await.currency;
    let raw_currency = payload.spend_currency.as_deref().unwrap_or(&default_currency);
    let currency = normalise_currency(raw_currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exp = exponent_for(&currency);

    let spend_minor = if let Some(ref s) = payload.spend_amount {
        if s.trim().is_empty() {
            0
        } else {
            parse_amount(s.trim(), exp).map_err(|e| ApiError::bad_request(e.to_string()))?
        }
    } else {
        0
    };

    let rating = payload.rating.unwrap_or(5).clamp(1, 5);
    let category = payload.category.as_deref().unwrap_or("travel");

    let created = FootprintRepository::create(
        &state.db.pool,
        &user.account_id,
        title,
        payload.latitude,
        payload.longitude,
        payload.country.as_deref(),
        payload.city.as_deref(),
        payload.address.as_deref(),
        &payload.arrival_date,
        payload.departure_date.as_deref(),
        spend_minor,
        &currency,
        exp as i64,
        rating,
        category,
        payload.note.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::FootprintCreated(created.clone()));

    Ok(Json(created))
}

pub async fn update_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateFootprintRequest>,
) -> Result<Json<Footprint>, ApiError> {
    let existing = FootprintRepository::find_owned(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found(format!("Footprint #{} not found", id)))?;

    let title = payload.title.as_deref().unwrap_or(&existing.title).trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("Title / Place Name cannot be empty".to_string()));
    }

    let lat = payload.latitude.unwrap_or(existing.latitude);
    let lon = payload.longitude.unwrap_or(existing.longitude);
    let country = payload.country.as_deref().or(existing.country.as_deref());
    let city = payload.city.as_deref().or(existing.city.as_deref());
    let address = payload.address.as_deref().or(existing.address.as_deref());
    let arrival_date = payload.arrival_date.as_deref().unwrap_or(&existing.arrival_date);
    let departure_date = payload.departure_date.as_deref().or(existing.departure_date.as_deref());

    let raw_currency = payload.spend_currency.as_deref().unwrap_or(&existing.spend_currency);
    let currency = normalise_currency(raw_currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exp = exponent_for(&currency);

    let spend_minor = if let Some(ref s) = payload.spend_amount {
        if s.trim().is_empty() {
            0
        } else {
            parse_amount(s.trim(), exp).map_err(|e| ApiError::bad_request(e.to_string()))?
        }
    } else {
        existing.spend_amount_minor
    };

    let rating = payload.rating.unwrap_or(existing.rating).clamp(1, 5);
    let category = payload.category.as_deref().unwrap_or(&existing.category);
    let note = payload.note.as_deref().or(existing.note.as_deref());

    let updated = FootprintRepository::update(
        &state.db.pool,
        id,
        &user.account_id,
        title,
        lat,
        lon,
        country,
        city,
        address,
        arrival_date,
        departure_date,
        spend_minor,
        &currency,
        exp as i64,
        rating,
        category,
        note,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found(format!("Footprint #{} not found", id)))?;

    state.broadcaster.send(SystemEvent::FootprintUpdated(updated.clone()));

    Ok(Json(updated))
}

pub async fn delete_footprint(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let deleted = FootprintRepository::delete(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !deleted {
        return Err(ApiError::not_found(format!("Footprint #{} not found", id)));
    }

    state.broadcaster.send(SystemEvent::FootprintDeleted {
        id,
        owner_id: user.account_id,
    });

    Ok(Json(serde_json::json!({ "success": true, "id": id })))
}

pub async fn get_summary_stats(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<FootprintsSummary>, ApiError> {
    let stats = FootprintRepository::summary_stats(&state.db.pool, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(stats))
}

/// Reverse or forward geocoding proxy using OpenStreetMap Nominatim.
pub async fn geocode_proxy(
    _user: AuthUser,
    Query(params): Query<GeocodeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = reqwest::Client::builder()
        .user_agent("matrix-bot/1.0 (contact: info@mbm.one)")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if let (Some(lat), Some(lon)) = (params.lat, params.lon) {
        let resp = client
            .get("https://nominatim.openstreetmap.org/reverse")
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
        let resp = client
            .get("https://nominatim.openstreetmap.org/search")
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
        Err(ApiError::bad_request("Must provide 'query' or 'lat' and 'lon'".to_string()))
    }
}
