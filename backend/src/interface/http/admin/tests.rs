//! Integration tests for administrative configuration and secret REST endpoints.

use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
    Router,
};
use tower::ServiceExt;

use crate::db::Database;
use crate::domain::accounts::{AccountRole, COOKIE_NAME, STATUS_ACTIVE};
use crate::infrastructure::crypto::{AesGcmSecretProtector, MasterKeyProvider};
use crate::interface::http::admin::audit::AuditLogPageDto;
use crate::interface::http::admin::secrets::SecretMetadataDto;
use crate::interface::http::admin::settings::SettingItemDto;
use crate::interface::http::admin::status::ConfigurationStatusDto;
use crate::interface::http::auth::ApiState;
use crate::interface::http::routes::api_routes;

const ORIGIN: &str = "http://localhost:8080";

async fn setup_app() -> (
    Router,
    ApiState,
    String,
    String,
    crate::domain::accounts::AccountId,
) {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;

    let key_provider = MasterKeyProvider::from_raw_key([42u8; 32]);
    let protector = std::sync::Arc::new(AesGcmSecretProtector::new(key_provider));
    let mut services = crate::application::AppServices::new(&pool);
    services = services.with_secret_protector(protector);

    let state = ApiState::new(
        Database { pool: pool.clone() },
        None,
        "https://mbm.one".to_string(),
        reqwest::Client::new(),
    );
    let mut state = state;
    state.services = services;

    // Create admin user
    let admin_acct = state
        .services
        .accounts
        .create("admin_alice", STATUS_ACTIVE)
        .await
        .unwrap();
    state
        .services
        .accounts
        .update_role(&admin_acct, AccountRole::Admin)
        .await
        .unwrap();
    let admin_token = state
        .services
        .accounts
        .create_session(&admin_acct, None, None, None)
        .await
        .unwrap();

    // Create regular user
    let user_acct = state
        .services
        .accounts
        .create("user_bob", STATUS_ACTIVE)
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

#[tokio::test]
async fn test_non_admin_user_rejected_with_403() {
    let (app, _state, _admin_token, user_token, _admin_acct) = setup_app().await;

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
            "Non-admin request to {path} must yield 403"
        );
        assert!(
            body.contains("Administrative privileges required"),
            "Body must indicate admin required: {body}"
        );
    }
}

#[tokio::test]
async fn test_stale_admin_session_rejected_with_403_step_up() {
    let (app, state, admin_token, _user_token, admin_acct) = setup_app().await;

    // Age admin step-up verification past 5 minutes
    state
        .services
        .accounts
        .update_last_verified_at(
            &admin_acct,
            chrono::Utc::now() - chrono::Duration::minutes(60),
        )
        .await
        .unwrap();

    let (status, body) = send_request(
        &app,
        Method::GET,
        "/api/v1/admin/config-status",
        Some(&admin_token),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("Confirm with your passkey to continue"),
        "Body must demand step up passkey: {body}"
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
    assert!(body.contains("healthy"));

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
