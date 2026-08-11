use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::accounts::{AccountRepository, SessionRepository};
use crate::api::auth::matrix::{get_bot_matrix_access_token, resolved_homeserver};
use crate::api::auth::types::{ApiState, AuthUser, MediaProxyQuery, UserProfileResponse};
use crate::error::ApiError;

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses((status = 204, description = "Signed out")),
    security(("session_cookie" = []))
)]
pub async fn logout(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<axum::response::Response, ApiError> {
    // Revoked server-side as well as cleared client-side. Clearing the cookie
    // alone would leave a token that still resolves, so anyone holding a copy
    // would stay signed in — which is exactly the gap a stateless JWT had.
    SessionRepository::revoke(&state.db.pool, &user.session_id_hash)
        .await
        .map_err(|err| ApiError::internal(format!("could not sign out: {err}")))?;

    Ok(crate::api::auth::webauthn::clearing_session_cookie(state.cookie_secure))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated user profile", body = UserProfileResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_current_user(
    State(state): State<ApiState>,
    auth_user: AuthUser,
) -> Result<Json<UserProfileResponse>, ApiError> {
    let mut display_name = None;
    let mut avatar_url = None;

    // An account can have several linked Matrix identities, so there is no
    // single "the" MXID any more. The first link supplies the avatar, purely
    // so the UI has a picture to show; nothing here decides anything.
    let linked = AccountRepository::linked_mxids(&state.db.pool, &auth_user.account_id)
        .await
        .unwrap_or_default();

    if let Some(mxid) = linked.first() {
        let clean_hs = resolved_homeserver(&state).await;
        let encoded_user = mxid.replace('@', "%40").replace(':', "%3A");
        let profile_url = format!("{}/_matrix/client/v3/profile/{}", clean_hs, encoded_user);

        match state.http_client.get(&profile_url).send().await {
            Err(err) => tracing::warn!("Profile lookup failed for {}: {:?}", mxid, err),
            Ok(res) if !res.status().is_success() => {
                tracing::warn!("Profile lookup for {} returned {}", mxid, res.status())
            }
            Ok(res) => {
                #[derive(Deserialize)]
                struct MatrixProfile {
                    displayname: Option<String>,
                    avatar_url: Option<String>,
                }
                if let Ok(profile) = res.json::<MatrixProfile>().await {
                    display_name = profile.displayname;
                    if let Some(mxc) = profile.avatar_url {
                        if mxc.starts_with("mxc://") {
                            let mxc_trim = &mxc[6..];
                            let mut parts = mxc_trim.splitn(2, '/');
                            if let (Some(server), Some(media_id)) = (parts.next(), parts.next()) {
                                avatar_url = Some(format!(
                                    "/api/v1/auth/media?server={}&media_id={}",
                                    server, media_id
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Json(UserProfileResponse {
        account_id: auth_user.account_id,
        username: auth_user.display_name.clone(),
        // The homeserver's own display name wins when there is one, because it
        // is what the user maintains; otherwise the account's name stands.
        display_name: display_name.or(Some(auth_user.display_name)),
        avatar_url,
        linked_mxids: linked,
    }))
}

/// Matrix server names are hostnames with an optional port, or an IP literal.
///
/// Rejecting everything outside this set keeps the value safe to interpolate as a
/// URL path segment: no `/`, `%`, `?`, `#` or `@`, so it cannot escape the path,
/// inject a query, or smuggle userinfo into the authority.
fn is_valid_server_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.contains("..")
        && s.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '[')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

/// Media IDs are opaque to clients but restricted to unreserved characters.
fn is_valid_media_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Streams an upstream media response back to the caller, refusing anything that
/// isn't an image so the proxy can't be used to relay arbitrary content.
async fn image_response(res: reqwest::Response) -> Option<axum::response::Response> {
    // Literal header name: reqwest 0.11 carries `http` 0.2, so axum's `http` 1.x
    // header constants are a different (sealed) type here.
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .filter(|ct| ct.starts_with("image/"))
        .unwrap_or("image/png")
        .to_string();

    let bytes = res.bytes().await.ok()?;

    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, content_type)
        // `private`: this is an authenticated endpoint, so shared caches must not store it.
        .header(axum::http::header::CACHE_CONTROL, "private, max-age=86400")
        .body(axum::body::Body::from(bytes))
        .ok()
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/media",
    params(MediaProxyQuery),
    responses(
        (status = 200, description = "Matrix avatar image stream"),
        (status = 400, description = "Malformed server name or media ID", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Media not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn media_proxy(
    State(state): State<ApiState>,
    _user: AuthUser,
    Query(query): Query<MediaProxyQuery>,
) -> Result<axum::response::Response, ApiError> {
    if !is_valid_server_name(&query.server) {
        return Err(ApiError::bad_request("Malformed Matrix server name"));
    }
    if !is_valid_media_id(&query.media_id) {
        return Err(ApiError::bad_request("Malformed Matrix media ID"));
    }

    let clean_hs = resolved_homeserver(&state).await;

    // Media is always fetched *through our own homeserver*, which proxies remote
    // servers on our behalf. `query.server` is a path segment identifying the
    // origin server — never a host we connect to — so a hostile value cannot
    // redirect the request (and with it the bot's access token) off-homeserver.
    let authenticated_urls = [
        format!(
            "{}/_matrix/client/v1/media/thumbnail/{}/{}?width=96&height=96&method=crop",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/client/v1/media/download/{}/{}",
            clean_hs, query.server, query.media_id
        ),
    ];

    // Pre-Matrix-1.11 homeservers still expose these unauthenticated.
    let legacy_urls = [
        format!(
            "{}/_matrix/media/v3/thumbnail/{}/{}?width=96&height=96&method=crop",
            clean_hs, query.server, query.media_id
        ),
        format!(
            "{}/_matrix/media/v3/download/{}/{}",
            clean_hs, query.server, query.media_id
        ),
    ];

    if let Some(token) = get_bot_matrix_access_token(&state).await {
        for media_url in authenticated_urls.iter().chain(legacy_urls.iter()) {
            if let Ok(res) = state.http_client.get(media_url).bearer_auth(&token).send().await {
                if res.status().is_success() {
                    if let Some(response) = image_response(res).await {
                        return Ok(response);
                    }
                }
            }
        }
    }

    for media_url in &legacy_urls {
        if let Ok(res) = state.http_client.get(media_url).send().await {
            if res.status().is_success() {
                if let Some(response) = image_response(res).await {
                    return Ok(response);
                }
            }
        }
    }

    Err(ApiError::not_found("Avatar media not found on Matrix homeserver"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_server_names_accepted() {
        assert!(is_valid_server_name("matrix.org"));
        assert!(is_valid_server_name("mbm.one"));
        assert!(is_valid_server_name("example.com:8448"));
        assert!(is_valid_server_name("127.0.0.1"));
        assert!(is_valid_server_name("[::1]:8448"));
    }

    #[test]
    fn test_server_name_rejects_path_and_authority_escapes() {
        // Path traversal out of the media route
        assert!(!is_valid_server_name(".."));
        assert!(!is_valid_server_name("evil.com/../.."));
        assert!(!is_valid_server_name("a..b"));
        // Query / fragment injection
        assert!(!is_valid_server_name("evil.com?x=1"));
        assert!(!is_valid_server_name("evil.com#frag"));
        // Userinfo smuggling and percent-encoded escapes
        assert!(!is_valid_server_name("evil.com@attacker.com"));
        assert!(!is_valid_server_name("evil.com%2f.."));
        assert!(!is_valid_server_name(""));
        assert!(!is_valid_server_name(&"a".repeat(256)));
    }

    #[test]
    fn test_media_id_rejects_separators() {
        assert!(is_valid_media_id("AbC123_-xyz"));
        assert!(!is_valid_media_id("../../secret"));
        assert!(!is_valid_media_id("abc/def"));
        assert!(!is_valid_media_id("abc?width=1"));
        assert!(!is_valid_media_id("abc.png"));
        assert!(!is_valid_media_id(""));
    }

    #[tokio::test]
    async fn test_media_proxy_rejects_malformed_input_before_any_request() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            crate::events::EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::named("alice", "acct_alice");

        let err = media_proxy(
            State(state.clone()),
            user.clone(),
            Query(MediaProxyQuery {
                server: "evil.example@attacker.test".to_string(),
                media_id: "abc".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);

        let err = media_proxy(
            State(state),
            user,
            Query(MediaProxyQuery {
                server: "matrix.org".to_string(),
                media_id: "../../etc/passwd".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
    }
}
