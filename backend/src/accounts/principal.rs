//! Which credential a request carries, and the account it acts as.
//!
//! Three kinds reach this application and they are not equivalent. A session
//! cookie is ambient — the browser attaches it to cross-site requests without
//! being asked — so it needs a CSRF defence that an explicitly-attached bearer
//! token does not. An MCP token is a machine credential with no human at the
//! other end. Keeping the three apart here means the extractor is left with the
//! one thing that is genuinely transport: deciding whether to trust the request
//! it arrived on.

use crate::accounts::session::COOKIE_NAME;
use crate::accounts::{AccountRepository, SessionRepository};
use crate::db::DbPool;
use crate::interface::http::auth::types::AuthUser;

/// A credential presented by a request.
pub enum Credential<'a> {
    /// The `mb_session` cookie.
    Cookie(&'a str),
    /// A session token attached explicitly as `Authorization: Bearer`, by a
    /// native client that has no cookie jar.
    BearerSession(&'a str),
    /// An MCP personal access token. Recognised by its `mb_mcp_` prefix.
    McpToken(&'a str),
}

impl<'a> Credential<'a> {
    /// Reads whichever credential the request carries.
    ///
    /// An explicit `Authorization` header wins over the cookie: a client that
    /// went to the trouble of attaching one means it.
    pub fn parse(headers: &'a axum::http::HeaderMap) -> Option<Self> {
        let bearer_token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|auth| {
                let parts: Vec<&str> = auth.split_whitespace().collect();
                if parts.len() == 2 && parts[0].eq_ignore_ascii_case("bearer") {
                    Some(parts[1])
                } else {
                    None
                }
            });

        if let Some(token) = bearer_token {
            return Some(if token.starts_with("mb_mcp_") {
                Credential::McpToken(token)
            } else {
                Credential::BearerSession(token)
            });
        }

        let cookie_token = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|cookies| cookie_value(cookies, COOKIE_NAME))?;

        Some(Credential::Cookie(cookie_token))
    }

    /// Whether the browser attaches this credential on its own.
    ///
    /// Only the cookie does, and that is exactly what makes cross-site request
    /// forgery possible, so only the cookie needs defending against it.
    pub fn is_ambient(&self) -> bool {
        matches!(self, Credential::Cookie(_))
    }
}

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

/// Resolves a credential to the account it acts as.
///
/// `Ok(None)` means the credential is not valid — expired, revoked, or naming
/// an account that no longer exists. The caller turns that into a 401; the
/// distinction between the three reasons is deliberately not exposed.
pub async fn resolve(
    pool: &DbPool,
    credential: &Credential<'_>,
) -> Result<Option<AuthUser>, sqlx::Error> {
    match credential {
        Credential::McpToken(token) => {
            let mcp_token =
                match crate::modules::mcp::repository::McpTokenRepository::resolve(pool, token)
                    .await?
                {
                    Some(mcp_token) => mcp_token,
                    None => {
                        tracing::debug!("MCP token is invalid or has expired");
                        return Ok(None);
                    }
                };

            let pool_for_touch = pool.clone();
            let token_id = mcp_token.id.clone();
            tokio::spawn(async move {
                let _ = crate::modules::mcp::repository::McpTokenRepository::touch_last_used(
                    &pool_for_touch,
                    &token_id,
                )
                .await;
            });

            let display_name = match AccountRepository::find(pool, &mcp_token.account_id).await? {
                Some(account) => account.display_name,
                None => {
                    tracing::debug!("MCP token names an account that no longer exists");
                    return Ok(None);
                }
            };

            Ok(Some(AuthUser {
                account_id: crate::domain::accounts::AccountId::new(mcp_token.account_id),
                display_name,
                session_id_hash: mcp_token.token_hash,
                // Never eligible for step-up. A personal access token is a machine
                // credential with nobody at the far end to confirm anything, so it
                // must not open a `VerifiedUser` route — and it did, for the first
                // five minutes after it was minted, because it reported its
                // creation time as the moment a passkey was last presented.
                last_verified_at: chrono::DateTime::<chrono::Utc>::MIN_UTC,
                restricted_until: None,
            }))
        }
        Credential::Cookie(token) | Credential::BearerSession(token) => {
            let session = match SessionRepository::resolve(pool, token).await? {
                Some(session) => session,
                None => {
                    tracing::debug!("session token is invalid or has expired");
                    return Ok(None);
                }
            };

            // The display name is read per request rather than copied into the
            // session, so renaming an account takes effect immediately instead of
            // on next sign-in.
            let display_name = match AccountRepository::find(pool, &session.account_id).await? {
                Some(account) => account.display_name,
                // A session whose account is gone is not a session.
                None => {
                    tracing::debug!("session names an account that no longer exists");
                    return Ok(None);
                }
            };

            Ok(Some(AuthUser {
                account_id: crate::domain::accounts::AccountId::new(session.account_id),
                display_name,
                session_id_hash: session.id_hash,
                last_verified_at: session.last_verified_at,
                restricted_until: session.restricted_until,
            }))
        }
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
}
