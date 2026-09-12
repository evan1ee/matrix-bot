use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts, Method},
};

use crate::accounts::{session::COOKIE_NAME, AccountRepository, SessionRepository};
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;

/// Pulls one cookie out of a `Cookie` header.
///
/// Hand-rolled rather than pulling in a cookie crate: there is exactly one
/// cookie this application reads, and the header grammar for that case is a
/// semicolon-separated list of `name=value`.
pub(crate) fn cookie_value<'a>(header_value: &'a str, name: &str) -> Option<&'a str> {
    header_value.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

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

        // 1. Check for Bearer token in Authorization header.
        // Bearer tokens are explicitly attached by non-browser clients (e.g. Element X iOS)
        // and cannot be forged via ambient browser credential CSRF.
        let bearer_token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|auth| {
                let parts: Vec<&str> = auth.split_whitespace().collect();
                if parts.len() == 2 && parts[0].eq_ignore_ascii_case("bearer") {
                    Some(parts[1])
                } else {
                    None
                }
            });

        let (token, is_bearer) = if let Some(t) = bearer_token {
            (t, true)
        } else {
            let cookie_token = parts
                .headers
                .get(header::COOKIE)
                .and_then(|value| value.to_str().ok())
                .and_then(|cookies| cookie_value(cookies, COOKIE_NAME))
                .ok_or_else(|| ApiError::unauthorized("Not signed in"))?;
            (cookie_token, false)
        };

        // A session cookie rides along automatically, so a cross-site request
        // would carry full authority if nothing stopped it. `SameSite=Strict`
        // is the first defence and this is the second: a state-changing request
        // must come from our own origin, and must say so.
        //
        // Non-browser clients (such as Element X iOS) presenting an explicit
        // Authorization: Bearer token are immune to ambient credential CSRF,
        // so the Origin requirement is enforced only for cookie-authenticated requests.
        if !is_bearer && is_mutating(&parts.method) {
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

        let session = SessionRepository::resolve(&api_state.db.pool, token)
            .await
            .map_err(|err| {
                tracing::error!("Could not read the session: {:?}", err);
                ApiError::internal("Could not read the session")
            })?
            .ok_or_else(|| ApiError::unauthorized("That session has expired"))?;

        // The display name is read per request rather than copied into the
        // session, so renaming an account takes effect immediately instead of
        // on next sign-in.
        let display_name = AccountRepository::find(&api_state.db.pool, &session.account_id)
            .await
            .map_err(|err| {
                tracing::error!("Could not read the account: {:?}", err);
                ApiError::internal("Could not read the account")
            })?
            .map(|account| account.display_name)
            // A session whose account is gone is not a session.
            .ok_or_else(|| ApiError::unauthorized("That account no longer exists"))?;

        Ok(AuthUser {
            account_id: session.account_id,
            display_name,
            session_id_hash: session.id_hash,
            last_verified_at: session.last_verified_at,
            restricted_until: session.restricted_until,
        })
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

    #[test]
    fn test_cookie_parsing() {
        assert_eq!(cookie_value("mb_session=abc", "mb_session"), Some("abc"));
        assert_eq!(
            cookie_value("other=1; mb_session=abc; third=2", "mb_session"),
            Some("abc")
        );
        assert_eq!(cookie_value("other=1; third=2", "mb_session"), None);
        assert_eq!(cookie_value("", "mb_session"), None);
        assert_eq!(cookie_value("malformed", "mb_session"), None);
    }

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
