use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use jsonwebtoken::{decode, Validation};

use crate::api::auth::types::{ApiState, AuthUser, Claims, JwtKeys, TokenScope};
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

        // Header only. Accepting `?token=` put a 7-day full-access credential in
        // URLs, where it is logged by proxies and kept in browser history; the
        // SSE stream now uses a short-lived scoped ticket instead.
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

        // 1. Try Matrix Access Token validation (if starts with syt_)
        if token.starts_with("syt_") {
            if let Some(user_id) = api_state.matrix_identity.whoami(&api_state, token).await {
                return Ok(AuthUser::matrix(user_id));
            } else {
                return Err(ApiError::unauthorized("Invalid or expired Matrix access token"));
            }
        }

        // 2. Try Local API JWT decoding. The identity space comes from the signed
        //    `kind` claim, never from the shape of `sub`.
        let keys = JwtKeys::new(api_state.jwt_secret.as_bytes());
        match decode::<Claims>(token, &keys.decoding, &Validation::default()) {
            Ok(token_data) => {
                // A stream ticket opens the SSE endpoint and nothing else.
                if token_data.claims.scope != TokenScope::Api {
                    return Err(ApiError::unauthorized(
                        "This token is not valid for the REST API",
                    ));
                }
                Ok(AuthUser {
                    subject: token_data.claims.sub,
                    kind: token_data.claims.kind,
                })
            }
            Err(_) => {
                // 3. Fallback: try checking if it's a raw Matrix Access Token that doesn't start with syt_
                if let Some(user_id) = api_state.matrix_identity.whoami(&api_state, token).await {
                    Ok(AuthUser::matrix(user_id))
                } else {
                    Err(ApiError::unauthorized("Invalid or expired authentication token"))
                }
            }
        }
    }
}
