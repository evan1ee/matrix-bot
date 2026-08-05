use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, State},
    http::request::Parts,
    Json,
};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthPayload {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterPayload {
    pub username: String,
    pub password: String,
    pub secret: Option<String>,
}

#[derive(Debug, Serialize)]
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

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

        if !auth_header.starts_with("Bearer ") {
            return Err(ApiError::unauthorized("Invalid token scheme"));
        }

        let token = &auth_header[7..];

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

    // 2. Fallback: Authenticate directly against the Matrix Homeserver
    if let Some(matrix_user_id) = authenticate_matrix_user(&state, &payload.username, &payload.password).await {
        let token = generate_jwt(&matrix_user_id, &state.jwt_secret)
            .map_err(|e| ApiError::internal(e.to_string()))?;

        return Ok(Json(AuthResponse {
            token,
            token_type: "Bearer".to_string(),
            username: matrix_user_id,
        }));
    }

    Err(ApiError::unauthorized("Invalid username or password"))
}

async fn validate_matrix_access_token(state: &ApiState, token: &str) -> Option<String> {
    let url = format!("{}/_matrix/client/v3/account/whoami", state.homeserver.trim_end_matches('/'));
    let res = state
        .http_client
        .get(&url)
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

async fn authenticate_matrix_user(state: &ApiState, username: &str, password: &str) -> Option<String> {
    let url = format!("{}/_matrix/client/v3/login", state.homeserver.trim_end_matches('/'));
    
    let login_body = serde_json::json!({
        "type": "m.login.password",
        "identifier": {
            "type": "m.id.user",
            "user": username
        },
        "password": password
    });

    let res = state
        .http_client
        .post(&url)
        .json(&login_body)
        .send()
        .await
        .ok()?;

    if res.status().is_success() {
        #[derive(Deserialize)]
        struct MatrixLoginRes {
            user_id: String,
        }
        let data: MatrixLoginRes = res.json().await.ok()?;
        Some(data.user_id)
    } else {
        None
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
        let state = ApiState {
            db: crate::db::Database { pool },
            broadcaster: EventBroadcaster::default(),
            jwt_secret: "test_secret".to_string(),
            registration_secret: None,
            homeserver: "https://matrix.org".to_string(),
            http_client: reqwest::Client::new(),
        };

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
