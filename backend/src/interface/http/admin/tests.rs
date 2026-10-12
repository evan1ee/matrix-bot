//! Integration tests for administrative configuration and secret REST endpoints.

use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
    Router,
};
use tower::ServiceExt;

use crate::application::secrets::TestResultDto;
use crate::db::Database;
use crate::domain::accounts::{AccountRole, COOKIE_NAME, STATUS_ACTIVE};
use crate::infrastructure::crypto::{AesGcmSecretProtector, MasterKeyProvider};
use crate::infrastructure::inference::testers::CredentialTester;
use crate::interface::http::admin::audit::AuditLogPageDto;
use crate::interface::http::admin::secrets::SecretMetadataDto;
use crate::interface::http::admin::settings::SettingItemDto;
use crate::interface::http::admin::status::ConfigurationStatusDto;
use crate::interface::http::auth::ApiState;
use crate::interface::http::routes::api_routes;

const ORIGIN: &str = "http://localhost:8080";

async fn start_mock_provider_server() -> String {
    use axum::routing::{get, post};

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (StatusCode::OK, r#"{"data":[]}"#) }),
        )
        .route(
            "/search",
            post(|| async { (StatusCode::OK, r#"{"results":[]}"#) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn setup_app() -> (
    Router,
    ApiState,
    String,
    String,
    crate::domain::accounts::AccountId,
) {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;

    let mock_base = start_mock_provider_server().await;
    let key_provider = MasterKeyProvider::from_raw_key([42u8; 32]);
    let protector = std::sync::Arc::new(AesGcmSecretProtector::new(key_provider));
    let mut services = crate::application::AppServices::new(&pool);
    services = services.with_secret_protector(protector);
    services.secrets = services
        .secrets
        .with_tester(CredentialTester::for_test(&mock_base));

    let state = ApiState::new(
        Database { pool: pool.clone() },
        None,
        "https://mbm.one".to_string(),
        reqwest::Client::new(),
    );
    let mut state = state;
    state.services = services;

    // Create admin user with enrolled passkey via controlled grant_admin
    let admin_acct = state
        .services
        .accounts
        .create("admin_alice", STATUS_ACTIVE)
        .await
        .unwrap();
    state
        .services
        .accounts
        .add_passkey(&admin_acct, "cred_admin_alice", "{}", "Admin Passkey")
        .await
        .unwrap();
    state
        .services
        .accounts
        .grant_admin(&admin_acct)
        .await
        .unwrap();
    let admin_token = state
        .services
        .accounts
        .create_session(&admin_acct, None, None, None)
        .await
        .unwrap();

    // Create regular user with passkey + fresh session so step-up is satisfied, proving role check blocks them
    let user_acct = state
        .services
        .accounts
        .create("user_bob", STATUS_ACTIVE)
        .await
        .unwrap();
    state
        .services
        .accounts
        .add_passkey(&user_acct, "cred_user_bob", "{}", "User Passkey")
        .await
        .unwrap();
    state
        .services
        .accounts
        .update_role(&user_acct, AccountRole::User)
        .await
        .unwrap();
    let user_token = state
        .services
        .accounts
        .create_session(&user_acct, None, None, None)
        .await
        .unwrap();

    let router = Router::new()
        .nest("/api/v1", api_routes())
        .with_state(state.clone());

    (router, state, admin_token, user_token, admin_acct)
}

async fn send_request(
    app: &Router,
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);

    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, format!("{COOKIE_NAME}={c}"));
    }
    builder = builder.header(header::ORIGIN, ORIGIN);
    builder = builder.header(header::CONTENT_TYPE, "application/json");

    let req_body = match body {
        Some(b) => Body::from(b.to_string()),
        None => Body::empty(),
    };

    let response = app
        .clone()
        .oneshot(builder.body(req_body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body_str = String::from_utf8_lossy(&bytes).to_string();

    (status, body_str)
}

#[tokio::test]
async fn test_anonymous_caller_rejected_with_401() {
    let (app, _state, _admin_token, _user_token, _admin_acct) = setup_app().await;

    for path in [
        "/api/v1/admin/config-status",
        "/api/v1/admin/settings",
        "/api/v1/admin/secrets",
        "/api/v1/admin/audit-logs",
    ] {
        let (status, _) = send_request(&app, Method::GET, path, None, None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "Anonymous request to {path} must yield 401"
        );
    }
}

/// A Matrix access-token exchange opens a session that never presented a
/// passkey. Even for an administrator who owns one, it must not reach the
/// admin reads that a passkey-verified session can.
#[tokio::test]
async fn test_an_unverified_matrix_session_is_rejected_on_admin_reads() {
    let (app, state, admin_token, _user_token, admin_acct) = setup_app().await;

    let unverified_token = state
        .services
        .accounts
        .create_unverified_session(&admin_acct, None, None, None)
        .await
        .unwrap();

    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&unverified_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("passkey authentication"), "{body}");

    // The same account with a passkey-verified session still gets through.
    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn test_non_admin_user_rejected_with_403_on_reads_and_writes() {
    let (app, state, _admin_token, user_token, admin_acct) = setup_app().await;

    for path in [
        "/api/v1/admin/config-status",
        "/api/v1/admin/settings",
        "/api/v1/admin/secrets",
        "/api/v1/admin/audit-logs",
    ] {
        let (status, body) = send_request(&app, Method::GET, path, Some(&user_token), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Non-admin GET request to {path} must yield 403"
        );
        assert!(
            body.contains("Administrative privileges required"),
            "Body must indicate admin required: {body}"
        );
    }

    for (method, path, payload) in [
        (
            Method::PUT,
            "/api/v1/admin/settings/llm.default_model",
            Some(r#"{"value": "gpt-4o"}"#),
        ),
        (
            Method::DELETE,
            "/api/v1/admin/settings/llm.default_model",
            None,
        ),
        (
            Method::POST,
            "/api/v1/admin/secrets",
            Some(r#"{"provider":"openai","name":"default","secret":"sk-1234567890abcdef"}"#),
        ),
        (
            Method::PUT,
            "/api/v1/admin/secrets/00000000-0000-0000-0000-000000000000",
            Some(r#"{"secret":"sk-1234567890abcdef"}"#),
        ),
        (
            Method::DELETE,
            "/api/v1/admin/secrets/00000000-0000-0000-0000-000000000000",
            None,
        ),
        (
            Method::POST,
            "/api/v1/admin/secrets/00000000-0000-0000-0000-000000000000/test",
            None,
        ),
    ] {
        let (status, body) = send_request(&app, method, path, Some(&user_token), payload).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Non-admin write request to {path} must yield 403"
        );
        assert!(
            body.contains("Administrative privileges required"),
            "Body must indicate admin required: {body}"
        );
    }

    // Also verify an MCP bearer token belonging to an admin account is rejected on both GET and write routes
    let (mcp_raw, _) = state
        .services
        .mcp
        .create_token(&admin_acct, "admin_mcp_token", None)
        .await
        .unwrap();
    for (method, path, body_str) in [
        (Method::GET, "/api/v1/admin/config-status", None),
        (
            Method::PUT,
            "/api/v1/admin/settings/llm.default_model",
            Some(r#"{"value": "gpt-4o"}"#),
        ),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {mcp_raw}"))
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(match body_str {
                Some(b) => Body::from(b.to_string()),
                None => Body::empty(),
            })
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "MCP token must be rejected with 403 on {path}"
        );
    }
}

#[tokio::test]
async fn test_stale_admin_session_allows_gets_but_rejects_writes_past_five_minutes() {
    let (app, state, admin_token, _user_token, admin_acct) = setup_app().await;

    // 1. Within 5-minute window (4 minutes ago): write succeeds
    state
        .services
        .accounts
        .update_last_verified_at(
            &admin_acct,
            chrono::Utc::now() - chrono::Duration::minutes(4),
        )
        .await
        .unwrap();

    let (status, body) = send_request(
        &app,
        Method::PUT,
        "/api/v1/admin/settings/llm.default_model",
        Some(&admin_token),
        Some(r#"{"value": "gpt-4o"}"#),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "4-minute-old verification must pass 5-minute window: {body}"
    );

    // 2. Past 5-minute window (6 minutes ago): ordinary GETs still succeed, writes demand step-up
    state
        .services
        .accounts
        .update_last_verified_at(
            &admin_acct,
            chrono::Utc::now() - chrono::Duration::minutes(6),
        )
        .await
        .unwrap();

    for path in [
        "/api/v1/admin/config-status",
        "/api/v1/admin/settings",
        "/api/v1/admin/settings/llm.default_model",
        "/api/v1/admin/secrets",
        "/api/v1/admin/audit-logs",
    ] {
        let (status, body) = send_request(&app, Method::GET, path, Some(&admin_token), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "Ordinary admin GET {path} must not require fresh step-up: {body}"
        );
    }

    let (status, body) = send_request(
        &app,
        Method::PUT,
        "/api/v1/admin/settings/llm.default_model",
        Some(&admin_token),
        Some(r#"{"value": "claude-3-5-sonnet"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("Confirm with your passkey to continue"),
        "Write with 6-minute-old verification must demand step-up passkey: {body}"
    );
}

#[tokio::test]
async fn test_admin_bootstrap_cli_and_passkey_enforcement() {
    let (app, state, _admin_token, _user_token, _admin_acct) = setup_app().await;

    // 1. Registering an account with a passkey never implicitly grants admin
    let first_user = crate::domain::accounts::AccountId::from_seed("first_registered_user");
    let pk = crate::application::accounts::test_passkey(b"first-user-cred", 1);
    state
        .services
        .accounts
        .register_with_passkey(&first_user, "first_user", &pk, "Primary Passkey")
        .await
        .unwrap();
    let acct = state
        .services
        .accounts
        .find(&first_user)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        acct.role,
        AccountRole::User,
        "Passkey registration must never auto-promote an account to admin"
    );

    // 2. Account without passkey cannot be granted admin via CLI or access admin endpoints even if DB role is forced
    let no_pk_acct = state
        .services
        .accounts
        .create("no_passkey_admin", STATUS_ACTIVE)
        .await
        .unwrap();
    let err = state
        .services
        .accounts
        .grant_admin(&no_pk_acct)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("enrolled passkey"), "{err}");

    // Force role in DB without a passkey and verify AdminSession still rejects
    state
        .services
        .accounts
        .update_role(&no_pk_acct, AccountRole::Admin)
        .await
        .unwrap();
    let no_pk_token = state
        .services
        .accounts
        .create_session(&no_pk_acct, None, None, None)
        .await
        .unwrap();
    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&no_pk_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("Passkey enrollment is required"), "{body}");

    // 3. Controlled CLI grant and revoke lifecycle
    let msg = state
        .services
        .accounts
        .run_admin_cli("grant", &first_user.to_string())
        .await
        .unwrap();
    assert!(msg.contains("Granted administrator privileges"));
    assert_eq!(
        state
            .services
            .accounts
            .find(&first_user)
            .await
            .unwrap()
            .unwrap()
            .role,
        AccountRole::Admin
    );

    let first_user_session = state
        .services
        .accounts
        .create_session(&first_user, None, None, None)
        .await
        .unwrap();
    let (status, _) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&first_user_session),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Revoke demotes role and terminates active sessions
    let revoke_msg = state
        .services
        .accounts
        .run_admin_cli("revoke", &first_user.to_string())
        .await
        .unwrap();
    assert!(revoke_msg.contains("Revoked administrator privileges"));
    let (status, _) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&first_user_session),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Revoking admin must terminate active sessions"
    );
}

#[tokio::test]
async fn test_verified_admin_full_lifecycle_and_zero_leakage() {
    let (app, _state, admin_token, _user_token, _admin_acct) = setup_app().await;

    // 1. GET /api/v1/admin/config-status
    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Status: {body}");
    let config_status: ConfigurationStatusDto = serde_json::from_str(&body).unwrap();
    assert!(config_status.settings_count > 0);

    // 2. GET /api/v1/admin/settings
    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/settings",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let settings: Vec<SettingItemDto> = serde_json::from_str(&body).unwrap();
    assert!(settings.iter().any(|s| s.key == "llm.default_model"));

    // 3. PUT /api/v1/admin/settings/llm.default_model
    let update_body = r#"{"value": "gpt-4o"}"#;
    let (status, body) = send_request(
        &app,
        Method::PUT,
        "/api/v1/admin/settings/llm.default_model",
        Some(&admin_token),
        Some(update_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Update: {body}");
    let updated_setting: SettingItemDto = serde_json::from_str(&body).unwrap();
    assert_eq!(updated_setting.value, serde_json::json!("gpt-4o"));
    assert!(updated_setting.is_custom);

    // 4. DELETE /api/v1/admin/settings/llm.default_model
    let (status, body) = send_request(
        &app,
        Method::DELETE,
        "/api/v1/admin/settings/llm.default_model",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Reset: {body}");

    // 5. POST /api/v1/admin/secrets
    let secret_payload = r#"{
        "provider": "openai",
        "name": "primary-key",
        "secret": "sk-super-secret-key-1234567890abcdef"
    }"#;
    let (status, body) = send_request(
        &app,
        Method::POST,
        "/api/v1/admin/secrets",
        Some(&admin_token),
        Some(secret_payload),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Create secret: {body}");
    let created_secret: SecretMetadataDto = serde_json::from_str(&body).unwrap();
    assert_eq!(created_secret.provider, "openai");
    assert_eq!(created_secret.name, "primary-key");
    assert_eq!(created_secret.masked_preview.as_deref(), Some("sk-...cdef"));

    // 6. Zero leakage assertion on GET /api/v1/admin/secrets
    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/secrets",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Strict zero-leakage invariants:
    assert!(!body.contains("super-secret-key"));
    assert!(!body.contains("1234567890"));
    assert!(!body.contains("secret_ciphertext"));
    assert!(!body.contains("ciphertext"));
    assert!(!body.contains("nonce"));
    assert!(body.contains("masked_preview"));
    assert!(body.contains("sk-...cdef"));

    // 7. POST /api/v1/admin/secrets/:id/test
    let test_url = format!("/api/v1/admin/secrets/{}/test", created_secret.id);
    let (status, body) =
        send_request(&app, Method::POST, &test_url, Some(&admin_token), None).await;
    assert_eq!(status, StatusCode::OK, "Test secret: {body}");
    let test_res: TestResultDto = serde_json::from_str(&body).unwrap();
    assert_eq!(test_res.status, "healthy");

    // 8. PUT /api/v1/admin/secrets/:id (replace)
    let replace_url = format!("/api/v1/admin/secrets/{}", created_secret.id);
    let replace_payload = r#"{"secret": "sk-updated-new-secret-99998888"}"#;
    let (status, body) = send_request(
        &app,
        Method::PUT,
        &replace_url,
        Some(&admin_token),
        Some(replace_payload),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Replace secret: {body}");
    let replaced_secret: SecretMetadataDto = serde_json::from_str(&body).unwrap();
    assert_eq!(
        replaced_secret.masked_preview.as_deref(),
        Some("sk-...8888")
    );

    // 9. DELETE /api/v1/admin/secrets/:id
    let (status, body) =
        send_request(&app, Method::DELETE, &replace_url, Some(&admin_token), None).await;
    assert_eq!(status, StatusCode::OK, "Delete secret: {body}");

    // 10. GET /api/v1/admin/audit-logs
    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/audit-logs",
        Some(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Audit logs: {body}");
    let page: AuditLogPageDto = serde_json::from_str(&body).unwrap();
    assert!(page.entries.len() >= 5);
    // Audit logs must also contain no leaked secret material:
    assert!(!body.contains("super-secret-key"));
    assert!(!body.contains("99998888"));
    assert!(!body.contains("ciphertext"));
    assert!(!body.contains("nonce"));
}
