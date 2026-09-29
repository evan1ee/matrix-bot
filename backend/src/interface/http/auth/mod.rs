pub mod extractor;
pub mod handlers;
pub mod matrix;
pub mod recovery;
pub mod types;
pub mod webauthn;

pub use handlers::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{
        session::{hash_token, COOKIE_NAME},
        AccountRepository, SessionRepository, STATUS_ACTIVE,
    };
    use crate::error::ApiError;
    use crate::events::EventBroadcaster;
    use axum::extract::FromRequestParts;
    use axum::http::{header, Method};

    const ORIGIN: &str = "http://localhost:8080";

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        )
    }

    /// Runs the extractor over a synthetic request.
    async fn extract(
        state: &ApiState,
        method: Method,
        cookie: Option<&str>,
        origin: Option<&str>,
    ) -> Result<AuthUser, ApiError> {
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri("/api/v1/todos");
        if let Some(value) = cookie {
            builder = builder.header(header::COOKIE, format!("{COOKIE_NAME}={value}"));
        }
        if let Some(value) = origin {
            builder = builder.header(header::ORIGIN, value);
        }
        let (mut parts, _) = builder.body(()).unwrap().into_parts();
        AuthUser::from_request_parts(&mut parts, state).await
    }

    async fn extract_bearer(
        state: &ApiState,
        method: Method,
        bearer: Option<&str>,
        origin: Option<&str>,
    ) -> Result<AuthUser, ApiError> {
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri("/api/v1/todos");
        if let Some(value) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {value}"));
        }
        if let Some(value) = origin {
            builder = builder.header(header::ORIGIN, value);
        }
        let (mut parts, _) = builder.body(()).unwrap().into_parts();
        AuthUser::from_request_parts(&mut parts, state).await
    }

    /// A signed-in account, and the cookie proving it.
    async fn signed_in(state: &ApiState, display_name: &str) -> (String, String) {
        let account = AccountRepository::create(&state.db.pool, display_name, STATUS_ACTIVE)
            .await
            .unwrap();
        let token = SessionRepository::create(&state.db.pool, &account, None, None, None)
            .await
            .unwrap();
        (account, token)
    }

    #[tokio::test]
    async fn test_a_session_cookie_resolves_to_its_account() {
        let state = state().await;
        let (account, token) = signed_in(&state, "evan").await;

        let user = extract(&state, Method::GET, Some(&token), None)
            .await
            .unwrap();
        assert_eq!(user.owner_id(), account);
        assert_eq!(user.display_name, "evan");
        assert_eq!(user.session_id_hash, hash_token(&token));
    }

    #[tokio::test]
    async fn test_a_bearer_token_resolves_to_its_account() {
        let state = state().await;
        let (account, token) = signed_in(&state, "evan").await;

        let user = extract_bearer(&state, Method::GET, Some(&token), None)
            .await
            .unwrap();
        assert_eq!(user.owner_id(), account);
        assert_eq!(user.display_name, "evan");
        assert_eq!(user.session_id_hash, hash_token(&token));
    }

    #[tokio::test]
    async fn test_bearer_token_mutating_requests_need_no_origin() {
        let state = state().await;
        let (_account, token) = signed_in(&state, "evan").await;

        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(
                extract_bearer(&state, method.clone(), Some(&token), None)
                    .await
                    .is_ok(),
                "{method} with Bearer auth should not require an Origin header"
            );
        }
    }

    #[tokio::test]
    async fn test_no_cookie_is_unauthorized() {
        let state = state().await;
        let err = extract(&state, Method::GET, None, None).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_an_unknown_cookie_is_unauthorized() {
        let state = state().await;
        let err = extract(&state, Method::GET, Some("nonsense"), None)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Signing out must take effect on the next request. A stateless JWT could
    /// not do this, which is most of why sessions moved server-side.
    #[tokio::test]
    async fn test_a_revoked_session_is_refused_immediately() {
        let state = state().await;
        let (_account, token) = signed_in(&state, "evan").await;

        assert!(extract(&state, Method::GET, Some(&token), None)
            .await
            .is_ok());

        SessionRepository::revoke(&state.db.pool, &hash_token(&token))
            .await
            .unwrap();

        let err = extract(&state, Method::GET, Some(&token), None)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// A session whose account has been deleted is not a session.
    #[tokio::test]
    async fn test_a_session_without_an_account_is_refused() {
        let state = state().await;
        let (account, token) = signed_in(&state, "evan").await;

        AccountRepository::delete(&state.db.pool, &account)
            .await
            .unwrap();

        let err = extract(&state, Method::GET, Some(&token), None)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// The cookie rides along on cross-site requests unless something stops it.
    /// `SameSite=Strict` is the first defence; this origin check is the second,
    /// and it applies only to methods that change state.
    #[tokio::test]
    async fn test_state_changing_requests_must_come_from_this_origin() {
        let state = state().await;
        let (_account, token) = signed_in(&state, "evan").await;

        // Reads need no origin at all.
        assert!(extract(&state, Method::GET, Some(&token), None)
            .await
            .is_ok());

        // Writes from our own origin are fine.
        assert!(extract(&state, Method::POST, Some(&token), Some(ORIGIN))
            .await
            .is_ok());

        // Writes from somewhere else are refused.
        let err = extract(
            &state,
            Method::POST,
            Some(&token),
            Some("https://evil.example"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);

        // So is a write with no Origin at all. Browsers always send one on
        // `fetch`, and there is no bearer-token client left whose omission
        // would be legitimate.
        let err = extract(&state, Method::POST, Some(&token), None)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);

        for method in [Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(
                extract(&state, method.clone(), Some(&token), None)
                    .await
                    .is_err(),
                "{method} must require an Origin"
            );
        }
    }

    /// Two accounts are two owners, whatever they call themselves.
    #[tokio::test]
    async fn test_a_shared_display_name_does_not_share_ownership() {
        let state = state().await;
        let (first, first_token) = signed_in(&state, "evan").await;
        let (second, second_token) = signed_in(&state, "evan").await;

        assert_ne!(first, second);

        let a = extract(&state, Method::GET, Some(&first_token), None)
            .await
            .unwrap();
        let b = extract(&state, Method::GET, Some(&second_token), None)
            .await
            .unwrap();

        assert_eq!(a.display_name, b.display_name);
        assert_ne!(
            a.owner_id(),
            b.owner_id(),
            "a shared name must not imply shared data"
        );
    }

    /// Step-up: a sensitive action needs a recent passkey assertion.
    #[tokio::test]
    async fn test_step_up_expires_and_recovery_sessions_never_qualify() {
        use chrono::{Duration, Utc};

        let fresh = AuthUser::for_account("acct_a");
        assert!(fresh.may_perform_sensitive_action(Utc::now()));

        // Just past the window.
        let stale = AuthUser {
            last_verified_at: Utc::now()
                - Duration::minutes(crate::accounts::session::STEP_UP_WINDOW_MINUTES + 1),
            ..fresh.clone()
        };
        assert!(!stale.may_perform_sensitive_action(Utc::now()));

        // A recovery session is refused even though it just authenticated —
        // recency and restriction are separate conditions, and both must hold.
        let restricted = AuthUser {
            restricted_until: Some(Utc::now() + Duration::hours(72)),
            ..fresh.clone()
        };
        assert!(
            !restricted.may_perform_sensitive_action(Utc::now()),
            "a restricted recovery session must never reach a sensitive action"
        );

        // Once the restriction lapses, a session that is otherwise current
        // qualifies again — the restriction expires on its own, with nothing to
        // run to clear it.
        let later = Utc::now() + Duration::hours(73);
        let lapsed = AuthUser {
            last_verified_at: later,
            ..restricted
        };
        assert!(lapsed.may_perform_sensitive_action(later));
    }

    /// An MCP token authenticates, but it is a machine credential: there is
    /// nobody at the far end to confirm anything with a passkey. It used to
    /// report its creation time as the last verification, so for the first five
    /// minutes of its life it opened every step-up-gated route.
    #[tokio::test]
    async fn test_an_mcp_token_never_satisfies_step_up() {
        let state = state().await;
        let account = AccountRepository::create(&state.db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let (raw, _token) =
            crate::infrastructure::postgres::repositories::mcp::McpTokenRepository::create(
                &state.db.pool,
                &crate::domain::accounts::AccountId::from(account.as_str()),
                "a machine",
                None,
            )
            .await
            .unwrap();

        let user = extract_bearer(&state, Method::GET, Some(&raw), None)
            .await
            .expect("a fresh MCP token must still authenticate");
        assert_eq!(user.owner_id(), account);

        assert!(
            !user.may_perform_sensitive_action(chrono::Utc::now()),
            "a token minted a moment ago must still not reach a sensitive action"
        );
    }
}
