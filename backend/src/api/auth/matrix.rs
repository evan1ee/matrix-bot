use matrix_sdk::ruma::{ServerName, UserId};
use matrix_sdk::Client;
use serde::Deserialize;

use chrono::Duration;

use crate::api::auth::types::{ApiState, AuthKind, Claims, JwtKeys, TokenScope};

/// The bot's access token, if the bot has one.
///
/// Reads the shared slot the bot bootstrap populates, falling back to the token
/// the bot cached on disk at last login. It deliberately does **not** log in:
/// authenticating as the bot is the bot's job, and doing it from inside a
/// request handler meant a single HTTP call could trigger a password login and
/// rewrite `data/matrix_token.json` as a side effect. If the bot has not logged
/// in, callers get `None` and degrade accordingly.
pub async fn get_bot_matrix_access_token(state: &ApiState) -> Option<String> {
    if let Some(tok) = state.bot_access_token.read().await.clone() {
        return Some(tok);
    }

    let file_tok = std::fs::read_to_string("data/matrix_token.json").ok()?;
    let trimmed = file_tok.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }

    *state.bot_access_token.write().await = Some(trimmed.clone());
    Some(trimmed)
}

pub async fn get_matrix_sdk_client(homeserver_input: &str) -> Option<Client> {
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    if let Ok(server_name) = ServerName::parse(clean_domain) {
        if let Ok(c) = Client::builder().server_name(&server_name).build().await {
            return Some(c);
        }
    }

    let fallback_url = if homeserver_input.starts_with("http://")
        || homeserver_input.starts_with("https://")
    {
        homeserver_input.to_string()
    } else {
        format!("https://{}", clean_domain)
    };

    Client::builder().homeserver_url(&fallback_url).build().await.ok()
}

/// The homeserver base URL, resolved once and reused.
///
/// Falls back to the configured value if discovery fails, so a transient
/// `.well-known` outage degrades rather than caching a wrong answer.
pub async fn resolved_homeserver(state: &ApiState) -> String {
    if let Some(url) = state.resolved_homeserver.read().await.clone() {
        return url;
    }

    let resolved = match get_matrix_sdk_client(&state.homeserver).await {
        Some(client) => {
            let url = client.homeserver().to_string();
            let url = url.trim_end_matches('/').to_string();
            *state.resolved_homeserver.write().await = Some(url.clone());
            url
        }
        None => state.homeserver.trim_end_matches('/').to_string(),
    };

    resolved
}

/// Whether a string is shaped like a Matrix access token.
///
/// Guards the extractor's last-resort `whoami` probe: without it, any malformed
/// bearer string cost an outbound round trip, letting an unauthenticated caller
/// amplify one request into homeserver traffic.
pub fn looks_like_access_token(token: &str) -> bool {
    (16..=512).contains(&token.len())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '+' | '/' | '='))
}

pub async fn validate_matrix_access_token(state: &ApiState, token: &str) -> Option<String> {
    if !looks_like_access_token(token) {
        return None;
    }

    let hs_url = resolved_homeserver(state).await;
    let whoami_url = format!("{}/_matrix/client/v3/account/whoami", hs_url.trim_end_matches('/'));

    let res = state
        .http_client
        .get(&whoami_url)
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

pub async fn authenticate_matrix_user(
    state: &ApiState,
    username: &str,
    password: &str,
) -> Option<(String, String)> {
    let homeserver_input = &state.homeserver;
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let user_id = if username.starts_with('@') {
        UserId::parse(username).ok()?
    } else {
        let raw_id = format!("@{}:{}", username, clean_domain);
        UserId::parse(&raw_id).ok()?
    };

    let client = get_matrix_sdk_client(&state.homeserver).await?;

    match client.matrix_auth().login_username(&user_id, password).send().await {
        Ok(res) => Some((res.user_id.to_string(), res.access_token)),
        Err(err) => {
            tracing::warn!("Matrix authentication failed for {}: {:?}", username, err);
            None
        }
    }
}

/// Issues a full-access API token, valid for 7 days.
pub fn generate_jwt(
    subject: &str,
    kind: AuthKind,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    generate_scoped_jwt(subject, kind, TokenScope::Api, Duration::days(7), secret)
}

/// Issues a token limited to `scope` and `ttl`.
pub fn generate_scoped_jwt(
    subject: &str,
    kind: AuthKind,
    scope: TokenScope,
    ttl: Duration,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(ttl)
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: subject.to_string(),
        exp: expiration,
        kind,
        scope,
    };

    let keys = JwtKeys::new(secret.as_bytes());
    jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &keys.encoding)
}
