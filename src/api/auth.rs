use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, State},
    http::{request::Parts, StatusCode},
    Json,
};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::db::Database;

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
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let api_state = ApiState::from_ref(state);

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

        if !auth_header.starts_with("Bearer ") {
            return Err((StatusCode::UNAUTHORIZED, "Invalid token scheme"));
        }

        let token = &auth_header[7..];
        let keys = JwtKeys::new(api_state.jwt_secret.as_bytes());

        let token_data = decode::<Claims>(token, &keys.decoding, &Validation::default())
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired token"))?;

        Ok(AuthUser {
            username: token_data.claims.sub,
        })
    }
}

pub async fn register(
    State(state): State<ApiState>,
    Json(payload): Json<AuthPayload>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    if payload.username.trim().is_empty() || payload.password.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Username and password required".to_string()));
    }

    let password_hash = hash(payload.password, DEFAULT_COST)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let res = sqlx::query("INSERT INTO web_users (username, password_hash) VALUES (?, ?)")
        .bind(&payload.username)
        .bind(&password_hash)
        .execute(&state.db.pool)
        .await;

    if let Err(err) = res {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Username already exists or database error: {}", err),
        ));
    }

    let token = generate_jwt(&payload.username, &state.jwt_secret)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AuthResponse {
        token,
        token_type: "Bearer".to_string(),
        username: payload.username,
    }))
}

pub async fn login(
    State(state): State<ApiState>,
    Json(payload): Json<AuthPayload>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    let row: Option<(String,)> = sqlx::query_as("SELECT password_hash FROM web_users WHERE username = ?")
        .bind(&payload.username)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (stored_hash,) = match row {
        Some(r) => r,
        None => return Err((StatusCode::UNAUTHORIZED, "Invalid username or password".to_string())),
    };

    let valid = verify(&payload.password, &stored_hash).unwrap_or(false);
    if !valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid username or password".to_string()));
    }

    let token = generate_jwt(&payload.username, &state.jwt_secret)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AuthResponse {
        token,
        token_type: "Bearer".to_string(),
        username: payload.username,
    }))
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
        };

        let payload = AuthPayload {
            username: "bob".to_string(),
            password: "password123".to_string(),
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
