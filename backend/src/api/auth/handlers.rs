use axum::{
    extract::{Query, State},
    Json,
};
use bcrypt::{hash, verify, DEFAULT_COST};
use serde::Deserialize;

use crate::api::auth::matrix::{
    generate_jwt, generate_scoped_jwt, get_bot_matrix_access_token, resolved_homeserver,
};
use crate::api::auth::types::{
    is_valid_local_username, ApiState, AuthKind, AuthPayload, AuthResponse, AuthUser,
    MediaProxyQuery, RegisterPayload, SseTicketResponse, TokenScope, UserProfileResponse,
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

    if payload.password.trim().is_empty() {
        return Err(ApiError::bad_request("Username and password required"));
    }

    // Reject anything that could be read as a Matrix user ID. Without this, an
    // account named `@someone:matrix.org` would resolve to that MXID's owner_id
    // and inherit their data.
    if !is_valid_local_username(&payload.username) {
        return Err(ApiError::bad_request(
            "Username must be 1-64 characters of letters, digits, '.', '-' or '_'",
        ));
    }

    let password_hash = hash(payload.password, DEFAULT_COST)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let res = sqlx::query("INSERT INTO web_users (username, password_hash) VALUES (?, ?)")
        .bind(&payload.username)
        .bind(&password_hash)
        .execute(&state.db.pool)
        .await;

    if let Err(err) = res {
        tracing::warn!("Registration failed for {}: {:?}", payload.username, err);
        return Err(ApiError::bad_request("Username already exists"));
    }

    let account_id =
        crate::accounts::AccountRepository::resolve_for_local_user(&state.db.pool, &payload.username)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    let token = generate_jwt(
        &payload.username,
        AuthKind::Local,
        &account_id,
        &state.jwt_secret,
    )
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
            let account_id = crate::accounts::AccountRepository::resolve_for_local_user(
                &state.db.pool,
                &payload.username,
            )
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

            let token = generate_jwt(
                &payload.username,
                AuthKind::Local,
                &account_id,
                &state.jwt_secret,
            )
            .map_err(|e| ApiError::internal(e.to_string()))?;

            return Ok(Json(AuthResponse {
                token,
                token_type: "Bearer".to_string(),
                username: payload.username,
            }));
        }
    }

    // 2. Fallback: Authenticate directly against the Matrix Homeserver.
    //
    // The caller's own Matrix access token is deliberately discarded. It proved
    // who they are, and that is all it is needed for — the JWT below is what they
    // authenticate to this API with. Storing it in `bot_access_token` (as this
    // once did) would hand every bot-credentialed endpoint the identity of
    // whoever logged in most recently.
    if let Some((matrix_user_id, _caller_access_token)) = state
        .matrix_identity
        .login(&state, &payload.username, &payload.password)
        .await
    {
        // The MXID is proven, so it resolves to an account the same way a
        // Matrix message from it would — provisioning one on first sight.
        let account_id = crate::accounts::AccountRepository::resolve_or_provision(
            &state.db.pool,
            &matrix_user_id,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        let token = generate_jwt(
            &matrix_user_id,
            AuthKind::Matrix,
            &account_id,
            &state.jwt_secret,
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

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

    if auth_user.kind == AuthKind::Matrix {
        let clean_hs = resolved_homeserver(&state).await;
        let encoded_user = auth_user.subject.replace('@', "%40").replace(':', "%3A");
        let profile_url = format!("{}/_matrix/client/v3/profile/{}", clean_hs, encoded_user);

        match state.http_client.get(&profile_url).send().await {
            Err(err) => tracing::warn!("Profile lookup failed for {}: {:?}", auth_user.subject, err),
            Ok(res) if !res.status().is_success() => {
                tracing::warn!("Profile lookup for {} returned {}", auth_user.subject, res.status())
            }
            Ok(res) => {
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
        username: auth_user.subject,
        display_name,
        avatar_url,
    }))
}

/// How long a stream ticket stays usable. Long enough to open a connection,
/// short enough that leaking it through a URL is largely inconsequential.
const SSE_TICKET_TTL_SECONDS: i64 = 30;

#[utoipa::path(
    post,
    path = "/api/v1/events/ticket",
    responses(
        (status = 200, description = "Short-lived ticket for opening the SSE stream", body = SseTicketResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_sse_ticket(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<SseTicketResponse>, ApiError> {
    let ticket = generate_scoped_jwt(
        &user.subject,
        user.kind,
        &user.account_id,
        TokenScope::SseStream,
        chrono::Duration::seconds(SSE_TICKET_TTL_SECONDS),
        &state.jwt_secret,
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(SseTicketResponse {
        ticket,
        expires_in: SSE_TICKET_TTL_SECONDS,
    }))
}

/// Matrix server names are hostnames with an optional port, or an IP literal.
///
/// Rejecting everything outside this set keeps the value safe to interpolate as a
/// URL path segment: no `/`, `%`, `?`, `#` or `@`, so it cannot escape the path,
/// inject a query, or smuggle userinfo into the authority.
fn is_valid_server_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.contains("..")
        && s.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '[')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

/// Media IDs are opaque to clients but restricted to unreserved characters.
fn is_valid_media_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Streams an upstream media response back to the caller, refusing anything that
/// isn't an image so the proxy can't be used to relay arbitrary content.
async fn image_response(res: reqwest::Response) -> Option<axum::response::Response> {
    // Literal header name: reqwest 0.11 carries `http` 0.2, so axum's `http` 1.x
    // header constants are a different (sealed) type here.
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .filter(|ct| ct.starts_with("image/"))
        .unwrap_or("image/png")
        .to_string();

    let bytes = res.bytes().await.ok()?;

    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, content_type)
        // `private`: this is an authenticated endpoint, so shared caches must not store it.
        .header(axum::http::header::CACHE_CONTROL, "private, max-age=86400")
        .body(axum::body::Body::from(bytes))
        .ok()
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/media",
    params(MediaProxyQuery),
    responses(
        (status = 200, description = "Matrix avatar image stream"),
        (status = 400, description = "Malformed server name or media ID", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Media not found", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn media_proxy(
    State(state): State<ApiState>,
    _user: AuthUser,
    Query(query): Query<MediaProxyQuery>,
) -> Result<axum::response::Response, ApiError> {
    if !is_valid_server_name(&query.server) {
        return Err(ApiError::bad_request("Malformed Matrix server name"));
    }
    if !is_valid_media_id(&query.media_id) {
        return Err(ApiError::bad_request("Malformed Matrix media ID"));
    }

    let clean_hs = resolved_homeserver(&state).await;

    // Media is always fetched *through our own homeserver*, which proxies remote
    // servers on our behalf. `query.server` is a path segment identifying the
    // origin server — never a host we connect to — so a hostile value cannot
    // redirect the request (and with it the bot's access token) off-homeserver.
    let authenticated_urls = [
        format!(
            "{}/_matrix/client/v1/media/thumbnail/{}/{}?width=96&height=96&method=crop",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/client/v1/media/download/{}/{}",
            clean_hs, query.server, query.media_id
        ),
    ];

    // Pre-Matrix-1.11 homeservers still expose these unauthenticated.
    let legacy_urls = [
        format!(
            "{}/_matrix/media/v3/thumbnail/{}/{}?width=96&height=96&method=crop",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/media/v3/download/{}/{}",
            clean_hs, query.server, query.media_id
        ),
    ];

    if let Some(token) = get_bot_matrix_access_token(&state).await {
        for media_url in authenticated_urls.iter().chain(legacy_urls.iter()) {
            if let Ok(res) = state.http_client.get(media_url).bearer_auth(&token).send().await {
                if res.status().is_success() {
                    if let Some(response) = image_response(res).await {
                        return Ok(response);
                    }
                }
            }
        }
    }

    for media_url in &legacy_urls {
        if let Ok(res) = state.http_client.get(media_url).send().await {
            if res.status().is_success() {
                if let Some(response) = image_response(res).await {
                    return Ok(response);
                }
            }
        }
    }

    Err(ApiError::not_found("Avatar media not found on Matrix homeserver"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_server_names_accepted() {
        assert!(is_valid_server_name("matrix.org"));
        assert!(is_valid_server_name("mbm.one"));
        assert!(is_valid_server_name("example.com:8448"));
        assert!(is_valid_server_name("127.0.0.1"));
        assert!(is_valid_server_name("[::1]:8448"));
    }

    #[test]
    fn test_server_name_rejects_path_and_authority_escapes() {
        // Path traversal out of the media route
        assert!(!is_valid_server_name(".."));
        assert!(!is_valid_server_name("evil.com/../.."));
        assert!(!is_valid_server_name("a..b"));
        // Query / fragment injection
        assert!(!is_valid_server_name("evil.com?x=1"));
        assert!(!is_valid_server_name("evil.com#frag"));
        // Userinfo smuggling and percent-encoded escapes
        assert!(!is_valid_server_name("evil.com@attacker.com"));
        assert!(!is_valid_server_name("evil.com%2f.."));
        assert!(!is_valid_server_name(""));
        assert!(!is_valid_server_name(&"a".repeat(256)));
    }

    #[test]
    fn test_media_id_rejects_separators() {
        assert!(is_valid_media_id("AbC123_-xyz"));
        assert!(!is_valid_media_id("../../secret"));
        assert!(!is_valid_media_id("abc/def"));
        assert!(!is_valid_media_id("abc?width=1"));
        assert!(!is_valid_media_id("abc.png"));
        assert!(!is_valid_media_id(""));
    }

    #[tokio::test]
    async fn test_media_proxy_rejects_malformed_input_before_any_request() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            crate::events::EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::local("alice", "acct_alice");

        let err = media_proxy(
            State(state.clone()),
            user.clone(),
            Query(MediaProxyQuery {
                server: "evil.example@attacker.test".to_string(),
                media_id: "abc".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);

        let err = media_proxy(
            State(state),
            user,
            Query(MediaProxyQuery {
                server: "matrix.org".to_string(),
                media_id: "../../etc/passwd".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
    }
}
