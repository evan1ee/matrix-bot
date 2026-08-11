use serde::Deserialize;


use crate::api::auth::types::ApiState;

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
