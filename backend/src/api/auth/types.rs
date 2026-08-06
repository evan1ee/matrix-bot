use axum::extract::FromRef;
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::db::Database;

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

impl AuthUser {
    pub fn sender_mxid(&self) -> String {
        if self.username.starts_with('@') {
            self.username.clone()
        } else {
            format!("@web-{}:api", self.username)
        }
    }
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
    pub matrix_client: std::sync::Arc<tokio::sync::RwLock<Option<matrix_sdk::Client>>>,
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
            matrix_client: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

impl FromRef<ApiState> for Database {
    fn from_ref(state: &ApiState) -> Self {
        state.db.clone()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserProfileResponse {
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct MediaProxyQuery {
    pub server: String,
    pub media_id: String,
    pub token: Option<String>,
}
