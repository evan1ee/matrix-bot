use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use jsonwebtoken::{decode, Validation};

use crate::api::auth::matrix::validate_matrix_access_token;
use crate::api::auth::types::{ApiState, AuthUser, Claims, JwtKeys};
use crate::error::ApiError;

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

        // 1. Try Matrix Access Token validation (if starts with syt_)
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
