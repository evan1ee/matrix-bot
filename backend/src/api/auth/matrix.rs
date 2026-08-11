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

    let fallback_url = if homeserver_input.starts_with("http://")
        || homeserver_input.starts_with("https://")
    {
        homeserver_input.to_string()
    } else {
        format!("https://{}", clean_domain)
    };

    if let Ok(c) = Client::builder().homeserver_url(&fallback_url).build().await {
        return Some(c);
    }

    if let Ok(server_name) = ServerName::parse(clean_domain) {
        if let Ok(c) = Client::builder().server_name(&server_name).build().await {
            return Some(c);
        }
    }

    None
}

/// Reads `m.homeserver.base_url` from the configured domain's `.well-known`.
///
/// Done explicitly rather than by inspecting a freshly built SDK client: that
/// builder falls back to a client pointed at the *undelegated* URL when
/// discovery fails, which is indistinguishable from success at the call site.
async fn discover_homeserver(state: &ApiState) -> Option<String> {
    #[derive(Deserialize)]
    struct HomeserverInfo {
        base_url: String,
    }
    #[derive(Deserialize)]
    struct WellKnown {
        #[serde(rename = "m.homeserver")]
        homeserver: HomeserverInfo,
    }

    let base = state.homeserver.trim_end_matches('/');
    let res = state
        .http_client
        .get(format!("{}/.well-known/matrix/client", base))
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }

    let well_known: WellKnown = res.json().await.ok()?;
    Some(well_known.homeserver.base_url.trim_end_matches('/').to_string())
}

/// The homeserver base URL for client-API calls, resolved once and reused.
///
/// A delegating domain (`MATRIX_HOMESERVER=https://mbm.one`) does not serve the
/// client API itself — requests there 404. Resolution therefore prefers the
/// bot's live client, whose homeserver is already known-good, then explicit
/// `.well-known` discovery. The configured value is only ever a last resort and
/// is **not** cached, so a transient outage cannot poison the cache for the rest
/// of the process's life.
pub async fn resolved_homeserver(state: &ApiState) -> String {
    if let Some(url) = state.resolved_homeserver.read().await.clone() {
        return url;
    }

    if let Some(client) = state.bot_client.read().await.clone() {
        let url = client.homeserver().to_string().trim_end_matches('/').to_string();
        *state.resolved_homeserver.write().await = Some(url.clone());
        return url;
    }

    if let Some(url) = discover_homeserver(state).await {
        *state.resolved_homeserver.write().await = Some(url.clone());
        return url;
    }

    tracing::warn!(
        "Could not resolve homeserver for {}; falling back to the configured URL",
        state.homeserver
    );
    state.homeserver.trim_end_matches('/').to_string()
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
    let hs_url = resolved_homeserver(state).await;
    let clean_domain = state
        .homeserver
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let user_id = if username.starts_with('@') {
        UserId::parse(username).ok()?
    } else {
        let raw_id = format!("@{}:{}", username, clean_domain);
        UserId::parse(&raw_id).ok()?
    };

    let client = get_matrix_sdk_client(&hs_url).await?;

    // Try logging in with localpart first (e.g. "evan"), then full MXID if that fails.
    let login_localpart = client
        .matrix_auth()
        .login_username(user_id.localpart(), password)
        .send();

    if let Ok(Ok(res)) = tokio::time::timeout(std::time::Duration::from_secs(10), login_localpart).await {
        return Some((res.user_id.to_string(), res.access_token));
    }

    let login_full = client
        .matrix_auth()
        .login_username(&user_id, password)
        .send();

    match tokio::time::timeout(std::time::Duration::from_secs(10), login_full).await {
        Ok(Ok(res)) => Some((res.user_id.to_string(), res.access_token)),
        Ok(Err(err)) => {
            tracing::warn!("Matrix authentication failed for {}: {:?}", username, err);
            None
        }
        Err(_) => {
            tracing::warn!("Matrix authentication timed out after 10s for {}", username);
            None
        }
    }
}

/// Issues a full-access API token, valid for 7 days.
pub fn generate_jwt(
    subject: &str,
    kind: AuthKind,
    account_id: &str,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    generate_scoped_jwt(
        subject,
        kind,
        account_id,
        TokenScope::Api,
        Duration::days(7),
        secret,
    )
}

/// Issues a token limited to `scope` and `ttl`.
pub fn generate_scoped_jwt(
    subject: &str,
    kind: AuthKind,
    account_id: &str,
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
        acct: account_id.to_string(),
    };

    let keys = JwtKeys::new(secret.as_bytes());
    jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &keys.encoding)
}
