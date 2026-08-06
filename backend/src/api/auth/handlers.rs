use axum::{
    extract::{Query, State},
    Json,
};
use bcrypt::{hash, verify, DEFAULT_COST};
use serde::Deserialize;

use crate::api::auth::matrix::{
    authenticate_matrix_user, generate_jwt, get_bot_matrix_access_token, get_matrix_sdk_client,
};
use crate::api::auth::types::{
    ApiState, AuthPayload, AuthResponse, AuthUser, MediaProxyQuery, RegisterPayload,
    UserProfileResponse,
};
use crate::error::ApiError;

#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    request_body = RegisterPayload,
    responses(
        (status = 200, description = "Registration successful", body = AuthResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    )
)]
pub async fn register(
    State(state): State<ApiState>,
    Json(payload): Json<RegisterPayload>,
) -> Result<Json<AuthResponse>, ApiError> {
    if let Some(ref required_secret) = state.registration_secret {
        let provided_secret = payload.secret.as_deref().unwrap_or("");
        if required_secret != provided_secret {
            return Err(ApiError::unauthorized("Invalid registration secret"));
        }
    }

    if payload.username.trim().is_empty() || payload.password.trim().is_empty() {
        return Err(ApiError::bad_request("Username and password required"));
    }

    let password_hash = hash(payload.password, DEFAULT_COST)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let res = sqlx::query("INSERT INTO web_users (username, password_hash) VALUES (?, ?)")
        .bind(&payload.username)
        .bind(&password_hash)
        .execute(&state.db.pool)
        .await;

    if let Err(err) = res {
        return Err(ApiError::bad_request(format!(
            "Username already exists or database error: {}",
            err
        )));
    }

    let token = generate_jwt(&payload.username, &state.jwt_secret)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(AuthResponse {
        token,
        token_type: "Bearer".to_string(),
        username: payload.username,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = AuthPayload,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    )
)]
pub async fn login(
    State(state): State<ApiState>,
    Json(payload): Json<AuthPayload>,
) -> Result<Json<AuthResponse>, ApiError> {
    // 1. Try local database authentication first
    let row: Option<(String,)> =
        sqlx::query_as("SELECT password_hash FROM web_users WHERE username = ?")
            .bind(&payload.username)
            .fetch_optional(&state.db.pool)
            .await
            .map_err(ApiError::from)?;

    if let Some((stored_hash,)) = row {
        if verify(&payload.password, &stored_hash).unwrap_or(false) {
            let token = generate_jwt(&payload.username, &state.jwt_secret)
                .map_err(|e| ApiError::internal(e.to_string()))?;

            return Ok(Json(AuthResponse {
                token,
                token_type: "Bearer".to_string(),
                username: payload.username,
            }));
        }
    }

    // 2. Fallback: Authenticate directly against the Matrix Homeserver
    if let Some((matrix_user_id, matrix_token)) =
        authenticate_matrix_user(&state, &payload.username, &payload.password).await
    {
        let token = generate_jwt(&matrix_user_id, &state.jwt_secret)
            .map_err(|e| ApiError::internal(e.to_string()))?;

        *state.matrix_access_token.write().await = Some(matrix_token);

        return Ok(Json(AuthResponse {
            token,
            token_type: "Bearer".to_string(),
            username: matrix_user_id,
        }));
    }

    Err(ApiError::unauthorized("Invalid username or password"))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated user profile", body = UserProfileResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_current_user(
    State(state): State<ApiState>,
    auth_user: AuthUser,
) -> Result<Json<UserProfileResponse>, ApiError> {
    let mut display_name = None;
    let mut avatar_url = None;

    if auth_user.username.starts_with('@') && auth_user.username.contains(':') {
        let client = get_matrix_sdk_client(&state.homeserver).await;
        let hs_url = if let Some(c) = client {
            c.homeserver().to_string()
        } else {
            state.homeserver.clone()
        };
        let clean_hs = hs_url.trim_end_matches('/');
        let encoded_user = auth_user.username.replace('@', "%40").replace(':', "%3A");
        let profile_url = format!("{}/_matrix/client/v3/profile/{}", clean_hs, encoded_user);

        if let Ok(res) = state.http_client.get(&profile_url).send().await {
            if res.status().is_success() {
                #[derive(Deserialize)]
                struct MatrixProfile {
                    displayname: Option<String>,
                    avatar_url: Option<String>,
                }
                if let Ok(profile) = res.json::<MatrixProfile>().await {
                    display_name = profile.displayname;
                    if let Some(mxc) = profile.avatar_url {
                        if mxc.starts_with("mxc://") {
                            let mxc_trim = &mxc[6..];
                            let mut parts = mxc_trim.splitn(2, '/');
                            if let (Some(server), Some(media_id)) = (parts.next(), parts.next()) {
                                avatar_url = Some(format!(
                                    "/api/v1/auth/media?server={}&media_id={}",
                                    server, media_id
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Json(UserProfileResponse {
        username: auth_user.username,
        display_name,
        avatar_url,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/media",
    params(MediaProxyQuery),
    responses(
        (status = 200, description = "Matrix avatar image stream"),
        (status = 404, description = "Media not found")
    )
)]
pub async fn media_proxy(
    State(state): State<ApiState>,
    Query(query): Query<MediaProxyQuery>,
) -> Result<axum::response::Response, ApiError> {
    let client = get_matrix_sdk_client(&state.homeserver).await;
    let hs_url = if let Some(c) = client {
        c.homeserver().to_string()
    } else {
        state.homeserver.clone()
    };
    let clean_hs = hs_url.trim_end_matches('/');

    let bases = vec![
        format!(
            "https://matrix.{}/_matrix/client/v1/media/download/{}/{}",
            query.server, query.server, query.media_id
        ),
        format!(
            "https://matrix.{}/_matrix/client/v1/media/thumbnail/{}/{}?width=96&height=96&method=crop",
            query.server, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/client/v1/media/download/{}/{}",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/client/v1/media/thumbnail/{}/{}?width=96&height=96&method=crop",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "https://matrix.{}/_matrix/media/v3/download/{}/{}",
            query.server, query.server, query.media_id
        ),
        format!(
            "https://matrix.{}/_matrix/media/v3/thumbnail/{}/{}?width=96&height=96&method=crop",
            query.server, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/media/v3/download/{}/{}",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/media/v3/thumbnail/{}/{}?width=96&height=96&method=crop",
            clean_hs, query.server, query.media_id
        ),
    ];

    let bot_token = get_bot_matrix_access_token(&state).await;

    for media_url in &bases {
        if let Some(ref tok) = bot_token {
            if let Ok(res) = state.http_client.get(media_url).bearer_auth(tok).send().await {
                if res.status().is_success() {
                    let content_type = res
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("image/png")
                        .to_string();

                    if let Ok(bytes) = res.bytes().await {
                        let response = axum::response::Response::builder()
                            .header(axum::http::header::CONTENT_TYPE, content_type)
                            .header(axum::http::header::CACHE_CONTROL, "public, max-age=86400")
                            .body(axum::body::Body::from(bytes))
                            .map_err(|e| ApiError::internal(e.to_string()))?;
                        return Ok(response);
                    }
                }
            }
        }

        if let Some(ref tok) = query.token {
            if let Ok(res) = state.http_client.get(media_url).bearer_auth(tok).send().await {
                if res.status().is_success() {
                    let content_type = res
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("image/png")
                        .to_string();

                    if let Ok(bytes) = res.bytes().await {
                        let response = axum::response::Response::builder()
                            .header(axum::http::header::CONTENT_TYPE, content_type)
                            .header(axum::http::header::CACHE_CONTROL, "public, max-age=86400")
                            .body(axum::body::Body::from(bytes))
                            .map_err(|e| ApiError::internal(e.to_string()))?;
                        return Ok(response);
                    }
                }
            }
        }

        if let Ok(res) = state.http_client.get(media_url).send().await {
            if res.status().is_success() {
                let content_type = res
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/png")
                    .to_string();

                if let Ok(bytes) = res.bytes().await {
                    let response = axum::response::Response::builder()
                        .header(axum::http::header::CONTENT_TYPE, content_type)
                        .header(axum::http::header::CACHE_CONTROL, "public, max-age=86400")
                        .body(axum::body::Body::from(bytes))
                        .map_err(|e| ApiError::internal(e.to_string()))?;
                    return Ok(response);
                }
            }
        }
    }

    Err(ApiError::not_found("Avatar media not found on Matrix homeserver"))
}
