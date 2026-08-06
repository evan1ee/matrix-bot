use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, State},
    http::request::Parts,
    Json,
};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use matrix_sdk::ruma::{ServerName, UserId};
use matrix_sdk::Client;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::ApiError;

use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthPayload {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RegisterPayload {
    pub username: String,
    pub password: String,
    pub secret: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    pub token: String,
    pub token_type: String,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub username: String,
}

pub struct JwtKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
}

impl JwtKeys {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }
}

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub broadcaster: crate::events::EventBroadcaster,
    pub jwt_secret: String,
    pub registration_secret: Option<String>,
    pub homeserver: String,
    pub http_client: reqwest::Client,
    pub matrix_access_token: std::sync::Arc<tokio::sync::RwLock<Option<String>>>,
}

impl ApiState {
    pub fn new(
        db: Database,
        broadcaster: crate::events::EventBroadcaster,
        jwt_secret: String,
        registration_secret: Option<String>,
        homeserver: String,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            db,
            broadcaster,
            jwt_secret,
            registration_secret,
            homeserver,
            http_client,
            matrix_access_token: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

impl FromRef<ApiState> for Database {
    fn from_ref(state: &ApiState) -> Self {
        state.db.clone()
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    ApiState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let api_state = ApiState::from_ref(state);

        let token_from_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|h| if h.starts_with("Bearer ") { Some(h[7..].trim()) } else { None });

        let token_from_query = parts
            .uri
            .query()
            .and_then(|q| {
                q.split('&').find_map(|pair| {
                    let mut kv = pair.split('=');
                    if kv.next()? == "token" {
                        kv.next()
                    } else {
                        None
                    }
                })
            });

        let token = match (token_from_header, token_from_query) {
            (Some(t), _) => t,
            (None, Some(t)) => t,
            (None, None) => return Err(ApiError::unauthorized("Missing Authorization header or token query param")),
        };

        // 1. Try Matrix Access Token validation (if starts with syt_ or token decode fails)
        if token.starts_with("syt_") {
            if let Some(user_id) = validate_matrix_access_token(&api_state, token).await {
                return Ok(AuthUser { username: user_id });
            } else {
                return Err(ApiError::unauthorized("Invalid or expired Matrix access token"));
            }
        }

        // 2. Try Local API JWT decoding
        let keys = JwtKeys::new(api_state.jwt_secret.as_bytes());
        match decode::<Claims>(token, &keys.decoding, &Validation::default()) {
            Ok(token_data) => Ok(AuthUser {
                username: token_data.claims.sub,
            }),
            Err(_) => {
                // 3. Fallback: try checking if it's a raw Matrix Access Token that doesn't start with syt_
                if let Some(user_id) = validate_matrix_access_token(&api_state, token).await {
                    Ok(AuthUser { username: user_id })
                } else {
                    Err(ApiError::unauthorized("Invalid or expired authentication token"))
                }
            }
        }
    }
}

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
            "Username already exists or database error: {}", err
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
    let row: Option<(String,)> = sqlx::query_as("SELECT password_hash FROM web_users WHERE username = ?")
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

    // 2. Fallback: Authenticate directly against the Matrix Homeserver with Well-Known discovery
    if let Some((matrix_user_id, matrix_token)) = authenticate_matrix_user(&state, &payload.username, &payload.password).await {
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

#[derive(Debug, Serialize, ToSchema)]
pub struct UserProfileResponse {
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
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

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct MediaProxyQuery {
    pub server: String,
    pub media_id: String,
    pub token: Option<String>,
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
    axum::extract::Query(query): axum::extract::Query<MediaProxyQuery>,
) -> Result<axum::response::Response, ApiError> {
    let client = get_matrix_sdk_client(&state.homeserver).await;
    let hs_url = if let Some(c) = client {
        c.homeserver().to_string()
    } else {
        state.homeserver.clone()
    };
    let clean_hs = hs_url.trim_end_matches('/');

    let bases = vec![
        format!("https://matrix.{}/_matrix/client/v1/media/download/{}/{}", query.server, query.server, query.media_id),
        format!("https://matrix.{}/_matrix/client/v1/media/thumbnail/{}/{}?width=96&height=96&method=crop", query.server, query.server, query.media_id),
        format!("{}/_matrix/client/v1/media/download/{}/{}", clean_hs, query.server, query.media_id),
        format!("{}/_matrix/client/v1/media/thumbnail/{}/{}?width=96&height=96&method=crop", clean_hs, query.server, query.media_id),
        format!("https://matrix.{}/_matrix/media/v3/download/{}/{}", query.server, query.server, query.media_id),
        format!("https://matrix.{}/_matrix/media/v3/thumbnail/{}/{}?width=96&height=96&method=crop", query.server, query.server, query.media_id),
        format!("{}/_matrix/media/v3/download/{}/{}", clean_hs, query.server, query.media_id),
        format!("{}/_matrix/media/v3/thumbnail/{}/{}?width=96&height=96&method=crop", clean_hs, query.server, query.media_id),
    ];

    let bot_token = get_bot_matrix_access_token(&state).await;

    for media_url in &bases {
        if let Some(ref tok) = bot_token {
            if let Ok(res) = state.http_client.get(media_url).bearer_auth(tok).send().await {
                tracing::info!("Media proxy bot auth attempt for {}: status={}", media_url, res.status());
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
                tracing::info!("Media proxy query auth attempt for {}: status={}", media_url, res.status());
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
            tracing::info!("Media proxy unauth attempt for {}: status={}", media_url, res.status());
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

async fn get_bot_matrix_access_token(state: &ApiState) -> Option<String> {
    if let Some(tok) = state.matrix_access_token.read().await.clone() {
        return Some(tok);
    }

    if let Ok(file_tok) = std::fs::read_to_string("data/matrix_token.json") {
        let trimmed = file_tok.trim().to_string();
        if !trimmed.is_empty() {
            *state.matrix_access_token.write().await = Some(trimmed.clone());
            return Some(trimmed);
        }
    }

    dotenvy::dotenv().ok();
    let env_user = std::env::var("MATRIX_USERNAME").ok()?;
    let env_pass = std::env::var("MATRIX_PASSWORD").ok()?;

    let client = get_matrix_sdk_client(&state.homeserver).await;
    let hs_url = if let Some(c) = client {
        c.homeserver().to_string()
    } else {
        state.homeserver.clone()
    };
    let clean_hs = hs_url.trim_end_matches('/');
    let clean_domain = state.homeserver
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let full_user_id = if env_user.starts_with('@') {
        env_user.clone()
    } else {
        format!("@{}:{}", env_user, clean_domain)
    };

    let login_url = format!("{}/_matrix/client/v3/login", clean_hs);

    #[derive(Serialize)]
    struct LoginReq<'a> {
        #[serde(rename = "type")]
        login_type: &'a str,
        identifier: UserIdentifier<'a>,
        password: &'a str,
    }

    #[derive(Serialize)]
    struct UserIdentifier<'a> {
        #[serde(rename = "type")]
        id_type: &'a str,
        user: &'a str,
    }

    let payload = LoginReq {
        login_type: "m.login.password",
        identifier: UserIdentifier {
            id_type: "m.id.user",
            user: &full_user_id,
        },
        password: &env_pass,
    };

    match state.http_client.post(&login_url).json(&payload).send().await {
        Ok(res) => {
            tracing::info!("Bot login request to {}: status={}", login_url, res.status());
            if res.status().is_success() {
                #[derive(Deserialize)]
                struct LoginResp {
                    access_token: String,
                }
                if let Ok(data) = res.json::<LoginResp>().await {
                    tracing::info!("Successfully obtained bot access_token from homeserver!");
                    std::fs::create_dir_all("data").ok();
                    std::fs::write("data/matrix_token.json", &data.access_token).ok();
                    *state.matrix_access_token.write().await = Some(data.access_token.clone());
                    return Some(data.access_token);
                }
            }
        }
        Err(err) => {
            tracing::warn!("Bot login HTTP error: {:?}", err);
        }
    }

    None
}

async fn get_matrix_sdk_client(homeserver_input: &str) -> Option<Client> {
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    if let Ok(server_name) = ServerName::parse(clean_domain) {
        if let Ok(c) = Client::builder().server_name(&server_name).build().await {
            return Some(c);
        }
    }

    let fallback_url = if homeserver_input.starts_with("http://") || homeserver_input.starts_with("https://") {
        homeserver_input.to_string()
    } else {
        format!("https://{}", clean_domain)
    };

    Client::builder().homeserver_url(&fallback_url).build().await.ok()
}

async fn validate_matrix_access_token(state: &ApiState, token: &str) -> Option<String> {
    let client = get_matrix_sdk_client(&state.homeserver).await?;
    let hs_url = client.homeserver().to_string();
    let whoami_url = format!("{}/_matrix/client/v3/account/whoami", hs_url.trim_end_matches('/'));

    let res = state
        .http_client
        .get(&whoami_url)
        .bearer_auth(token)
        .send()
        .await
        .ok()?;

    if res.status().is_success() {
        #[derive(Deserialize)]
        struct WhoAmI {
            user_id: String,
        }
        let data: WhoAmI = res.json().await.ok()?;
        Some(data.user_id)
    } else {
        None
    }
}

async fn authenticate_matrix_user(state: &ApiState, username: &str, password: &str) -> Option<(String, String)> {
    let homeserver_input = &state.homeserver;
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let user_id = if username.starts_with('@') {
        UserId::parse(username).ok()?
    } else {
        let raw_id = format!("@{}:{}", username, clean_domain);
        UserId::parse(&raw_id).ok()?
    };

    let client = get_matrix_sdk_client(&state.homeserver).await?;

    match client.matrix_auth().login_username(&user_id, password).send().await {
        Ok(res) => Some((res.user_id.to_string(), res.access_token)),
        Err(err) => {
            tracing::warn!("Matrix authentication failed for {}: {:?}", username, err);
            None
        }
    }
}

fn generate_jwt(username: &str, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: username.to_string(),
        exp: expiration,
    };

    let keys = JwtKeys::new(secret.as_bytes());
    encode(&Header::default(), &claims, &keys.encoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_jwt_generation_and_validation() {
        let secret = "my_test_secret_key_12345";
        let token = generate_jwt("alice", secret).unwrap();
        let keys = JwtKeys::new(secret.as_bytes());
        let decoded = decode::<Claims>(&token, &keys.decoding, &Validation::default()).unwrap();
        assert_eq!(decoded.claims.sub, "alice");
    }

    #[tokio::test]
    async fn test_register_and_login_flow() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "test_secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let payload = RegisterPayload {
            username: "bob".to_string(),
            password: "password123".to_string(),
            secret: None,
        };

        let reg_res = register(State(state.clone()), Json(payload)).await.unwrap();
        assert_eq!(reg_res.username, "bob");
        assert!(!reg_res.token.is_empty());

        let login_payload = AuthPayload {
            username: "bob".to_string(),
            password: "password123".to_string(),
        };
        let login_res = login(State(state), Json(login_payload)).await.unwrap();
        assert_eq!(login_res.username, "bob");
    }
}
