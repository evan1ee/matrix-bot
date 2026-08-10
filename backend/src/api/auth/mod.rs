pub mod extractor;
pub mod handlers;
pub mod matrix;
pub mod types;

pub use handlers::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::auth::matrix::{generate_jwt, generate_scoped_jwt};
    use crate::db::schema::run_migrations;
    use crate::error::ApiError;
    use crate::events::EventBroadcaster;
    use axum::extract::{FromRequestParts, State};
    use axum::Json;
    use jsonwebtoken::{decode, Validation};
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_jwt_generation_and_validation() {
        let secret = "my_test_secret_key_12345";
        let token = generate_jwt("alice", AuthKind::Local, secret).unwrap();
        let keys = JwtKeys::new(secret.as_bytes());
        let decoded = decode::<Claims>(&token, &keys.decoding, &Validation::default()).unwrap();
        assert_eq!(decoded.claims.sub, "alice");
        assert_eq!(decoded.claims.kind, AuthKind::Local);
    }

    #[test]
    fn test_identity_spaces_never_collide() {
        // The bug this guards: a local account named `alice` and the Matrix user
        // `@alice:matrix.org` previously resolved to overlapping owners because
        // ownership also matched on the display-only `sender_name` column.
        let local = AuthUser::local("alice");
        let matrix = AuthUser::matrix("@alice:matrix.org");

        assert_eq!(local.owner_id(), "@web-alice:api");
        assert_eq!(matrix.owner_id(), "@alice:matrix.org");
        assert_ne!(local.owner_id(), matrix.owner_id());

        // They share a display name, which must not imply shared ownership.
        assert_eq!(local.display_name(), matrix.display_name());
    }

    #[test]
    fn test_local_usernames_cannot_impersonate_an_mxid() {
        assert!(!is_valid_local_username("@alice:matrix.org"));
        assert!(!is_valid_local_username("@alice"));
        assert!(!is_valid_local_username("alice:matrix.org"));
        assert!(!is_valid_local_username("web-alice:api"));
        assert!(!is_valid_local_username(""));
        assert!(!is_valid_local_username(&"a".repeat(65)));

        assert!(is_valid_local_username("alice"));
        assert!(is_valid_local_username("alice.b_c-1"));
    }

    /// Authenticating a caller must never disturb the bot's own credentials.
    ///
    /// `login` used to store the caller's Matrix access token in the shared bot
    /// slot, which repointed `/rooms` and the media proxy at whoever logged in
    /// last. Covers the local-credential path; the Matrix-credential path needs
    /// a live homeserver, so the guard there is the discarded binding plus the
    /// `bot_access_token` field name.
    #[tokio::test]
    async fn test_login_leaves_bot_credentials_untouched() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "test_secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        // Stand in for the bot having logged in at startup.
        *state.bot_access_token.write().await = Some("syt_bot_token".to_string());

        let _ = register(
            State(state.clone()),
            Json(RegisterPayload {
                username: "carol".to_string(),
                password: "password123".to_string(),
                secret: None,
            }),
        )
        .await
        .unwrap();

        let _ = login(
            State(state.clone()),
            Json(AuthPayload {
                username: "carol".to_string(),
                password: "password123".to_string(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(
            state.bot_access_token.read().await.as_deref(),
            Some("syt_bot_token"),
            "a user login must not overwrite the bot's access token"
        );
    }

    /// Stands in for a homeserver: one known access token, one known password.
    struct FakeMatrix;

    #[async_trait::async_trait]
    impl MatrixIdentityProvider for FakeMatrix {
        async fn whoami(&self, _state: &ApiState, token: &str) -> Option<String> {
            (token == "syt_valid_token_value").then(|| "@evan:mbm.one".to_string())
        }

        async fn login(
            &self,
            _state: &ApiState,
            username: &str,
            password: &str,
        ) -> Option<(String, String)> {
            (username == "evan" && password == "hunter2")
                .then(|| ("@evan:mbm.one".to_string(), "syt_caller_token".to_string()))
        }
    }

    fn fake_matrix_state(pool: SqlitePool) -> ApiState {
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "test_secret".to_string(),
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        )
        .with_matrix_identity(std::sync::Arc::new(FakeMatrix))
    }

    async fn extract(state: &ApiState, header: Option<&str>) -> Result<AuthUser, ApiError> {
        let mut builder = axum::http::Request::builder().uri("/api/v1/todos");
        if let Some(value) = header {
            builder = builder.header(axum::http::header::AUTHORIZATION, value);
        }
        let (mut parts, _) = builder.body(()).unwrap().into_parts();
        AuthUser::from_request_parts(&mut parts, state).await
    }

    /// Counts how many times the homeserver was actually asked.
    struct CountingMatrix {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl MatrixIdentityProvider for CountingMatrix {
        async fn whoami(&self, _state: &ApiState, token: &str) -> Option<String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (token == "syt_valid_token_value").then(|| "@evan:mbm.one".to_string())
        }

        async fn login(
            &self,
            _state: &ApiState,
            _username: &str,
            _password: &str,
        ) -> Option<(String, String)> {
            None
        }
    }

    /// Repeat requests on one token must cost one `/whoami`, not one each.
    ///
    /// This is what makes a mobile client viable: a screen firing several calls
    /// at once used to multiply into that many homeserver round trips, which
    /// homeservers rate limit.
    #[tokio::test]
    async fn test_matrix_identity_is_cached_across_requests() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "test_secret".to_string(),
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        )
        .with_matrix_identity(std::sync::Arc::new(CountingMatrix {
            calls: calls.clone(),
        }));

        for _ in 0..5 {
            let user = extract(&state, Some("Bearer syt_valid_token_value"))
                .await
                .unwrap();
            assert_eq!(user.owner_id(), "@evan:mbm.one");
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "five requests on one token must resolve the identity once"
        );

        // A rejected token is never cached, so it cannot be replayed past a
        // revocation — and it re-asks every time.
        for _ in 0..3 {
            assert!(extract(&state, Some("Bearer syt_wrong_token_value"))
                .await
                .is_err());
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "failures must not be served from cache"
        );

        // A second, different token gets its own entry rather than the first's.
        assert!(extract(&state, Some("Bearer syt_valid_token_value"))
            .await
            .is_ok());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "the still-valid first token stays cached"
        );
    }

    /// The extractor decides which identity space a caller lands in. Every
    /// ownership check downstream trusts that decision.
    #[tokio::test]
    async fn test_extractor_resolves_identity_space_correctly() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = fake_matrix_state(pool);

        // A Matrix access token resolves through whoami to a Matrix identity.
        let user = extract(&state, Some("Bearer syt_valid_token_value"))
            .await
            .unwrap();
        assert_eq!(user.kind, AuthKind::Matrix);
        assert_eq!(user.owner_id(), "@evan:mbm.one");

        // A local JWT stays local, even though its subject is a bare name.
        let local_jwt = generate_jwt("evan", AuthKind::Local, &state.jwt_secret).unwrap();
        let user = extract(&state, Some(&format!("Bearer {}", local_jwt)))
            .await
            .unwrap();
        assert_eq!(user.kind, AuthKind::Local);
        assert_eq!(user.owner_id(), "@web-evan:api");

        // A Matrix JWT stays Matrix — the `kind` claim decides, not the subject.
        let matrix_jwt =
            generate_jwt("@evan:mbm.one", AuthKind::Matrix, &state.jwt_secret).unwrap();
        let user = extract(&state, Some(&format!("Bearer {}", matrix_jwt)))
            .await
            .unwrap();
        assert_eq!(user.kind, AuthKind::Matrix);
        assert_eq!(user.owner_id(), "@evan:mbm.one");
    }

    /// A token predating the `kind` claim must be refused outright.
    ///
    /// Defaulting it to `Local` let an existing session keep working while
    /// silently becoming a different principal: writes landed under
    /// `@web-@evan:mbm.one:api` and the user's real rows disappeared from every
    /// view. Failing the request sends them back through login.
    #[tokio::test]
    async fn test_tokens_without_a_kind_claim_are_rejected() {
        #[derive(serde::Serialize)]
        struct LegacyClaims {
            sub: String,
            exp: usize,
        }

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = fake_matrix_state(pool);

        let legacy = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &LegacyClaims {
                sub: "@evan:mbm.one".to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::days(1)).timestamp() as usize,
            },
            &JwtKeys::new(state.jwt_secret.as_bytes()).encoding,
        )
        .unwrap();

        let err = extract(&state, Some(&format!("Bearer {}", legacy)))
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_extractor_rejects_bad_credentials() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = fake_matrix_state(pool);

        for header in [None, Some("Bearer syt_wrong_token_value"), Some("Bearer garbage")] {
            let err = extract(&state, header).await.unwrap_err();
            assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
        }

        // A stream ticket must not unlock the REST API.
        let ticket = generate_scoped_jwt(
            "evan",
            AuthKind::Local,
            TokenScope::SseStream,
            chrono::Duration::seconds(30),
            &state.jwt_secret,
        )
        .unwrap();
        let err = extract(&state, Some(&format!("Bearer {}", ticket)))
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// The Matrix-credential branch of `login` — the one that used to overwrite
    /// the bot's token — now exercised directly.
    #[tokio::test]
    async fn test_matrix_login_issues_matrix_token_and_spares_bot_credentials() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = fake_matrix_state(pool);

        *state.bot_access_token.write().await = Some("syt_bot_token".to_string());

        let res = login(
            State(state.clone()),
            Json(AuthPayload {
                username: "evan".to_string(),
                password: "hunter2".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(res.username, "@evan:mbm.one");

        // The issued JWT carries the Matrix identity space.
        let keys = JwtKeys::new(state.jwt_secret.as_bytes());
        let decoded = decode::<Claims>(&res.token, &keys.decoding, &Validation::default()).unwrap();
        assert_eq!(decoded.claims.kind, AuthKind::Matrix);
        assert_eq!(decoded.claims.sub, "@evan:mbm.one");

        // The caller's own Matrix token was discarded, not stored as the bot's.
        assert_eq!(
            state.bot_access_token.read().await.as_deref(),
            Some("syt_bot_token")
        );

        // Wrong password still fails.
        let err = login(
            State(state),
            Json(AuthPayload {
                username: "evan".to_string(),
                password: "wrong".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_register_rejects_mxid_shaped_username() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "test_secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let payload = RegisterPayload {
            username: "@evan:mbm.one".to_string(),
            password: "password123".to_string(),
            secret: None,
        };

        let err = register(State(state), Json(payload)).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_register_and_login_flow() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "test_secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let payload = RegisterPayload {
            username: "bob".to_string(),
            password: "password123".to_string(),
            secret: None,
        };

        let reg_res = register(State(state.clone()), Json(payload)).await.unwrap();
        assert_eq!(reg_res.username, "bob");
        assert!(!reg_res.token.is_empty());

        let login_payload = AuthPayload {
            username: "bob".to_string(),
            password: "password123".to_string(),
        };
        let login_res = login(State(state), Json(login_payload)).await.unwrap();
        assert_eq!(login_res.username, "bob");
    }
}
