use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts, Method},
};

use crate::accounts::Credential;
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;

/// Whether a request method changes state.
fn is_mutating(method: &Method) -> bool {
    !matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS)
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

        let credential = Credential::parse(&parts.headers)
            .ok_or_else(|| ApiError::unauthorized("Not signed in"))?;

        // A session cookie rides along automatically, so a cross-site request
        // would carry full authority if nothing stopped it. `SameSite=Strict`
        // is the first defence and this is the second: a state-changing request
        // must come from our own origin, and must say so.
        //
        // Non-browser clients (such as Element X iOS) presenting an explicit
        // Authorization: Bearer token are immune to ambient credential CSRF,
        // so the Origin requirement is enforced only for cookie-authenticated requests.
        if credential.is_ambient() && is_mutating(&parts.method) {
            let origin = parts
                .headers
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok());

            match origin {
                Some(value) if api_state.is_allowed_origin(value) => {}
                Some(_) => {
                    return Err(ApiError::unauthorized(
                        "This request did not come from this site",
                    ))
                }
                None => {
                    return Err(ApiError::unauthorized(
                        "A state-changing request must carry an Origin header",
                    ))
                }
            }
        }

        crate::accounts::resolve_credential(&api_state.db.pool, &credential)
            .await
            .map_err(|err| {
                tracing::error!("Could not read the session: {:?}", err);
                ApiError::internal("Could not read the session")
            })?
            .ok_or_else(|| ApiError::unauthorized("That session has expired"))
    }
}

/// A caller who has proved possession of a passkey within the step-up window.
///
/// Putting this in a handler's signature is the whole opt-in — the same
/// ergonomic [`AuthUser`] already has. A sensitive endpoint that forgets it
/// silently accepts an hours-old session, so §6 of the design has a
/// table-driven test asserting every route that needs one has one.
#[derive(Debug, Clone)]
pub struct VerifiedUser(pub AuthUser);

impl std::ops::Deref for VerifiedUser {
    type Target = AuthUser;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for VerifiedUser
where
    S: Send + Sync,
    ApiState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;

        if user
            .restricted_until
            .is_some_and(|until| chrono::Utc::now() < until)
        {
            return Err(ApiError::forbidden(
                "This session was created by account recovery and cannot do that yet",
            ));
        }

        if !user.may_perform_sensitive_action(chrono::Utc::now()) {
            return Err(ApiError::forbidden("Confirm with your passkey to continue"));
        }

        Ok(VerifiedUser(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the safe methods skip the origin check.
    #[test]
    fn test_which_methods_need_an_origin() {
        for safe in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!is_mutating(&safe), "{safe} must not require an Origin");
        }
        for mutating in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(is_mutating(&mutating), "{mutating} must require an Origin");
        }
    }
}
