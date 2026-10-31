//! Calendar feed endpoints: the public `.ics` feed a calendar client polls and
//! the authenticated token management behind Settings.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::interface::http::auth::recovery::extract_client_ip;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
use crate::interface::http::rate_limit::check_calendar_limit;

/// Same body for a missing, unknown or revoked token, so the response is no
/// oracle for which tokens ever existed.
const FEED_NOT_FOUND: &str = "Calendar feed not found";

#[derive(Debug, Deserialize, IntoParams)]
pub struct FeedQuery {
    /// The subscription token; a `Authorization: Bearer` header works too.
    pub token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarTokenResponse {
    /// Shown once; only its hash is stored.
    pub token: String,
    /// Path (with query) of the feed, relative to the server's origin.
    pub feed_path: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CalendarTokenStatus {
    pub active: bool,
    pub created_at: Option<DateTime<Utc>>,
}

fn presented_token(query: &FeedQuery, headers: &HeaderMap) -> Option<String> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    bearer
        .or(query.token.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// GET /api/v1/calendar/feed.ics — the account's agenda as iCalendar.
#[utoipa::path(
    get,
    tag = "calendar",
    path = "/api/v1/calendar/feed.ics",
    params(FeedQuery),
    responses(
        (status = 200, description = "iCalendar document", content_type = "text/calendar", body = String),
        (status = 404, description = "Unknown or revoked token", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    )
)]
pub async fn get_feed(
    State(state): State<ApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Query(query): Query<FeedQuery>,
) -> Result<Response, ApiError> {
    let ip = extract_client_ip(connect_info.as_ref(), &headers, state.trust_forwarded_for);
    if !check_calendar_limit(&ip) {
        return Err(ApiError::too_many_requests(
            "Too many calendar requests. Please slow down.",
        ));
    }
    let token =
        presented_token(&query, &headers).ok_or_else(|| ApiError::not_found(FEED_NOT_FOUND))?;
    let ics = state
        .services
        .calendar
        .ics_feed(&token)
        .await?
        .ok_or_else(|| ApiError::not_found(FEED_NOT_FOUND))?;

    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/calendar; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-cache"),
            ),
        ],
        ics,
    )
        .into_response())
}

/// POST /api/v1/account/calendar-token — mints a feed token, replacing any old one.
#[utoipa::path(
    post,
    tag = "calendar",
    path = "/api/v1/account/calendar-token",
    responses(
        (status = 200, description = "New token; shown once", body = CalendarTokenResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn mint_token(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<CalendarTokenResponse>, ApiError> {
    let token = state.services.calendar.mint_token(&user.account_id).await?;
    let feed_path = format!("/api/v1/calendar/feed.ics?token={token}");
    Ok(Json(CalendarTokenResponse { token, feed_path }))
}

/// GET /api/v1/account/calendar-token — whether a feed token exists.
#[utoipa::path(
    get,
    tag = "calendar",
    path = "/api/v1/account/calendar-token",
    responses(
        (status = 200, description = "Token status", body = CalendarTokenStatus),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn token_status(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<CalendarTokenStatus>, ApiError> {
    let created_at = state
        .services
        .calendar
        .token_status(&user.account_id)
        .await?;
    Ok(Json(CalendarTokenStatus {
        active: created_at.is_some(),
        created_at,
    }))
}

/// DELETE /api/v1/account/calendar-token — revokes the feed token.
#[utoipa::path(
    delete,
    tag = "calendar",
    path = "/api/v1/account/calendar-token",
    responses(
        (status = 204, description = "Revoked (also when none existed)"),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn revoke_token(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<StatusCode, ApiError> {
    state
        .services
        .calendar
        .revoke_token(&user.account_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice"]).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    async fn call(state: &ApiState, token: Option<&str>, bearer: Option<&str>) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(b) = bearer {
            headers.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {b}")).unwrap(),
            );
        }
        let query = FeedQuery {
            token: token.map(str::to_owned),
        };
        match get_feed(State(state.clone()), None, headers, Query(query)).await {
            Ok(resp) => resp,
            Err(err) => err.into_response(),
        }
    }

    async fn body(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn missing_bad_and_revoked_tokens_are_the_same_404() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let minted = mint_token(State(state.clone()), alice.clone())
            .await
            .unwrap();
        revoke_token(State(state.clone()), alice).await.unwrap();

        let missing = call(&state, None, None).await;
        let bad = call(&state, Some("mb_cal_wrong"), None).await;
        let revoked = call(&state, Some(&minted.0.token), None).await;
        for resp in [&missing, &bad, &revoked] {
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }
        let (a, b, c) = (body(missing).await, body(bad).await, body(revoked).await);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[tokio::test]
    async fn feed_has_calendar_headers_and_works_with_query_or_bearer() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let minted = mint_token(State(state.clone()), alice).await.unwrap();
        assert!(minted.0.feed_path.ends_with(&minted.0.token));

        let by_query = call(&state, Some(&minted.0.token), None).await;
        assert_eq!(by_query.status(), StatusCode::OK);
        assert_eq!(
            by_query.headers()[header::CONTENT_TYPE],
            "text/calendar; charset=utf-8"
        );
        assert_eq!(
            by_query.headers()[header::CACHE_CONTROL],
            "private, no-cache"
        );
        assert!(body(by_query).await.starts_with("BEGIN:VCALENDAR\r\n"));

        let by_bearer = call(&state, None, Some(&minted.0.token)).await;
        assert_eq!(by_bearer.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_follows_mint_and_revoke() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let before = token_status(State(state.clone()), alice.clone())
            .await
            .unwrap();
        assert!(!before.0.active);
        let _ = mint_token(State(state.clone()), alice.clone())
            .await
            .unwrap();
        let after = token_status(State(state.clone()), alice.clone())
            .await
            .unwrap();
        assert!(after.0.active && after.0.created_at.is_some());
        let code = revoke_token(State(state.clone()), alice.clone())
            .await
            .unwrap();
        assert_eq!(code, StatusCode::NO_CONTENT);
        assert!(!token_status(State(state), alice).await.unwrap().0.active);
    }
}
