//! Reads the credential a request carries out of its headers.
//!
//! The credential types are pure and live in `domain::accounts`; this is the
//! transport half — knowing which header holds what.

use axum::http::{header, HeaderMap};

use crate::domain::accounts::{cookie_value, Credential, COOKIE_NAME};

/// Reads whichever credential the request carries.
///
/// An explicit `Authorization` header wins over the cookie: a client that went
/// to the trouble of attaching one means it.
pub fn parse_credential(headers: &HeaderMap) -> Option<Credential<'_>> {
    let bearer_token = headers
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

    if let Some(token) = bearer_token {
        return Some(if token.starts_with("mb_mcp_") {
            Credential::McpToken(token)
        } else {
            Credential::BearerSession(token)
        });
    }

    let cookie_token = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| cookie_value(cookies, COOKIE_NAME))?;

    Some(Credential::Cookie(cookie_token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn cookie_is_read_by_name() {
        let map = headers(&[(header::COOKIE, "other=1; mb_session=abc; third=2")]);
        assert_eq!(parse_credential(&map), Some(Credential::Cookie("abc")));

        let map = headers(&[(header::COOKIE, "other=1; third=2")]);
        assert_eq!(parse_credential(&map), None);
        assert_eq!(parse_credential(&HeaderMap::new()), None);
    }

    #[test]
    fn bearer_beats_cookie() {
        let map = headers(&[
            (header::COOKIE, "mb_session=from_cookie"),
            (header::AUTHORIZATION, "Bearer from_header"),
        ]);
        assert_eq!(
            parse_credential(&map),
            Some(Credential::BearerSession("from_header"))
        );
    }

    #[test]
    fn mcp_prefix_is_an_mcp_token() {
        let map = headers(&[(header::AUTHORIZATION, "bearer mb_mcp_secret")]);
        assert_eq!(
            parse_credential(&map),
            Some(Credential::McpToken("mb_mcp_secret"))
        );
    }

    #[test]
    fn a_malformed_authorization_header_yields_no_bearer() {
        let map = headers(&[(header::AUTHORIZATION, "Basic abc")]);
        assert_eq!(parse_credential(&map), None);
        let map = headers(&[(header::AUTHORIZATION, "Bearer")]);
        assert_eq!(parse_credential(&map), None);
    }
}
