//! Which credential a request carries, and the account it acts as.
//!
//! Three kinds reach this application and they are not equivalent. A session
//! cookie is ambient — the browser attaches it to cross-site requests without
//! being asked — so it needs a CSRF defence that an explicitly-attached bearer
//! token does not. An MCP token is a machine credential with no human at the
//! other end. Keeping the three apart here means the extractor is left with the
//! one thing that is genuinely transport: deciding whether to trust the request
//! it arrived on.

use chrono::{DateTime, Utc};

use super::AccountId;

/// A credential presented by a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential<'a> {
    /// The `mb_session` cookie.
    Cookie(&'a str),
    /// A session token attached explicitly as `Authorization: Bearer`, by a
    /// native client that has no cookie jar.
    BearerSession(&'a str),
    /// An MCP personal access token. Recognised by its `mb_mcp_` prefix.
    McpToken(&'a str),
}

impl Credential<'_> {
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
pub fn cookie_value<'a>(header_value: &'a str, name: &str) -> Option<&'a str> {
    header_value.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

/// The account a valid credential acts as, and the state the extractor needs to
/// decide whether it may do something sensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub account_id: AccountId,
    pub display_name: String,
    pub session_id_hash: String,
    pub last_verified_at: DateTime<Utc>,
    pub restricted_until: Option<DateTime<Utc>>,
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

    #[test]
    fn only_the_cookie_is_ambient() {
        assert!(Credential::Cookie("t").is_ambient());
        assert!(!Credential::BearerSession("t").is_ambient());
        assert!(!Credential::McpToken("mb_mcp_t").is_ambient());
    }
}
