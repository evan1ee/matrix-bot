//! The identity and link endpoints, driven through the real router the way the
//! browser reaches them (cookie + `Origin`), against a throwaway database.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::domain::accounts::{hash_token, AccountId, STATUS_ACTIVE};
use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
use crate::infrastructure::postgres::repositories::accounts::core;
use crate::infrastructure::postgres::repositories::{identities, outbox};
use crate::interface::http::auth::types::{ApiState, Ceremony};
use crate::interface::http::routes::api_routes;

const ORIGIN: &str = "http://localhost:8080";

fn tg(raw: &str) -> PlatformIdentity {
    PlatformIdentity::new(
        Channel::Telegram,
        PlatformUserId::parse(Channel::Telegram, raw).unwrap(),
        None,
    )
}

fn mx(raw: &str) -> PlatformIdentity {
    PlatformIdentity::matrix(raw).unwrap()
}

struct Harness {
    app: Router,
    state: ApiState,
    account: AccountId,
    cookie: String,
}

/// A signed-in caller whose last passkey proof was `verified_minutes_ago`;
/// Telegram user `1` may sign up, nobody else.
async fn harness(verified_minutes_ago: i64) -> Harness {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let mut state = ApiState::new(
        crate::db::Database { pool },
        None,
        "https://mbm.one".to_string(),
        reqwest::Client::new(),
    );
    state.services = state.services.clone().with_link_config(
        "https://bot.example".to_string(),
        HashMap::from([(Channel::Telegram, vec!["1".to_string()])]),
    );
    let accounts = &state.services.accounts;
    let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
    let cookie = accounts
        .create_session(&account, None, None, None)
        .await
        .unwrap();
    accounts
        .update_last_verified_at(
            &account,
            chrono::Utc::now() - chrono::Duration::minutes(verified_minutes_ago),
        )
        .await
        .unwrap();
    let app = Router::new()
        .nest("/api/v1", api_routes())
        .with_state(state.clone());
    Harness {
        app,
        state,
        account,
        cookie,
    }
}

impl Harness {
    async fn call(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, format!("mb_session={cookie}"));
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A browser request with the session cookie and our own `Origin`.
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call("POST", path, Some(&self.cookie), Some(ORIGIN), body)
            .await
    }

    /// A request with no session at all.
    async fn anonymous(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call("POST", path, None, Some(ORIGIN), body).await
    }

    async fn mint(&self, who: &PlatformIdentity, chat: i64) -> String {
        self.state
            .services
            .link
            .start(who, &ConversationAddress::Telegram(chat), true)
            .await
            .unwrap()
            .token
    }

    async fn owner(&self, who: &PlatformIdentity) -> Option<AccountId> {
        identities::find_account(&self.state.db.pool, who)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn identities_lists_the_callers_matrix_and_telegram_only() {
    let h = harness(0).await;
    let pool = &h.state.db.pool;
    identities::bind(pool, &h.account, &mx("@evan:mbm.one"))
        .await
        .unwrap();
    identities::bind(pool, &h.account, &tg("7")).await.unwrap();
    let other = core::create(pool, "other", STATUS_ACTIVE).await.unwrap();
    identities::bind(pool, &other, &tg("8")).await.unwrap();

    let (status, body) = h
        .call(
            "GET",
            "/api/v1/account/identities",
            Some(&h.cookie),
            None,
            Value::Null,
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    let listed: Vec<(String, String)> = body["identities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["channel"].as_str().unwrap().to_string(),
                i["user_id"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(listed.len(), 2, "{body}");
    assert!(listed.contains(&("matrix".into(), "@evan:mbm.one".into())));
    assert!(listed.contains(&("telegram".into(), "7".into())));
    assert!(body["identities"][0]["linked_at"].is_string());
}

#[tokio::test]
async fn the_old_matrix_link_routes_are_gone() {
    let h = harness(0).await;
    for (method, path) in [
        ("GET", "/api/v1/account/matrix"),
        ("POST", "/api/v1/account/matrix/unlink"),
    ] {
        let (status, _) = h
            .call(method, path, Some(&h.cookie), Some(ORIGIN), json!({}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
    }
}

#[tokio::test]
async fn unlinking_one_of_several_ways_in_succeeds() {
    let h = harness(0).await;
    let pool = &h.state.db.pool;
    identities::bind(pool, &h.account, &mx("@evan:mbm.one"))
        .await
        .unwrap();
    identities::bind(pool, &h.account, &tg("7")).await.unwrap();

    let (status, body) = h
        .post(
            "/api/v1/account/identities/unlink",
            json!({"channel": "telegram", "user_id": "7"}),
        )
        .await;
    assert_eq!((status, &body["unlinked"]), (StatusCode::OK, &json!(true)));
    assert_eq!(h.owner(&tg("7")).await, None);

    let (_, again) = h
        .post(
            "/api/v1/account/identities/unlink",
            json!({"channel": "telegram", "user_id": "7"}),
        )
        .await;
    assert_eq!(again["unlinked"], json!(false));
}

#[tokio::test]
async fn unlinking_removes_only_your_own_identity() {
    let h = harness(0).await;
    let pool = &h.state.db.pool;
    let theirs = core::create(pool, "theirs", STATUS_ACTIVE).await.unwrap();
    identities::bind(pool, &theirs, &mx("@evan:mbm.one"))
        .await
        .unwrap();

    let (status, body) = h
        .post(
            "/api/v1/account/identities/unlink",
            json!({"channel": "matrix", "user_id": "@evan:mbm.one"}),
        )
        .await;

    assert_eq!((status, &body["unlinked"]), (StatusCode::OK, &json!(false)));
    assert_eq!(h.owner(&mx("@evan:mbm.one")).await, Some(theirs));
}

#[tokio::test]
async fn unlinking_the_last_way_in_is_a_conflict() {
    let h = harness(0).await;
    identities::bind(&h.state.db.pool, &h.account, &tg("7"))
        .await
        .unwrap();

    let (status, body) = h
        .post(
            "/api/v1/account/identities/unlink",
            json!({"channel": "telegram", "user_id": "7"}),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("only way to sign in"));
    assert_eq!(h.owner(&tg("7")).await, Some(h.account));
}

#[tokio::test]
async fn unlinking_an_unknown_channel_is_a_bad_request() {
    let h = harness(0).await;
    let (status, _) = h
        .post(
            "/api/v1/account/identities/unlink",
            json!({"channel": "carrier-pigeon", "user_id": "7"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn claim_with_a_stale_session_is_forbidden() {
    let h = harness(60).await;
    let token = h.mint(&tg("7"), 7).await;
    let (status, _) = h.post("/api/v1/link/claim", json!({"token": token})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(h.owner(&tg("7")).await, None);
}

#[tokio::test]
async fn claim_with_a_fresh_session_binds_and_publishes_identity_linked() {
    let h = harness(0).await;
    let token = h.mint(&tg("7"), 7).await;

    let (status, body) = h.post("/api/v1/link/claim", json!({"token": token})).await;

    assert_eq!(
        (status, &body["outcome"]),
        (StatusCode::OK, &json!("linked"))
    );
    assert_eq!(h.owner(&tg("7")).await, Some(h.account));
    assert_eq!(
        outbox::event_types_for(&h.state.db.pool, &h.account)
            .await
            .unwrap(),
        vec!["IdentityLinked"]
    );
}

#[tokio::test]
async fn claiming_an_unknown_token_is_expired_and_changes_nothing() {
    let h = harness(0).await;
    let (status, body) = h
        .post("/api/v1/link/claim", json!({"token": "not-a-token"}))
        .await;
    assert_eq!(
        (status, &body["outcome"]),
        (StatusCode::OK, &json!("expired"))
    );
}

/// The CSRF layer is the `AuthUser` extractor: a cookie-authenticated POST
/// must say where it came from.
#[tokio::test]
async fn claim_without_an_origin_is_refused() {
    let h = harness(0).await;
    let token = h.mint(&tg("7"), 7).await;

    let (status, _) = h
        .call(
            "POST",
            "/api/v1/link/claim",
            Some(&h.cookie),
            None,
            json!({"token": token}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h
        .call(
            "POST",
            "/api/v1/link/claim",
            Some(&h.cookie),
            Some("https://evil.example"),
            json!({"token": token}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(h.owner(&tg("7")).await, None);
}

#[tokio::test]
async fn inspect_without_a_session_offers_signup_only_to_the_allowlisted() {
    let h = harness(0).await;
    let allowed = h.mint(&tg("1"), 1).await;
    let stranger = h.mint(&tg("2"), 2).await;

    let (status, body) = h
        .anonymous("/api/v1/link/inspect", json!({"token": stranger}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], json!("sign_in_required"));

    let (_, body) = h
        .anonymous("/api/v1/link/inspect", json!({"token": allowed}))
        .await;
    assert_eq!(body["state"], json!("signup_allowed"));
    assert_eq!(body["channel"], json!("telegram"));
    assert_eq!(body["user_id"], json!("1"));
}

#[tokio::test]
async fn inspect_with_a_session_previews_the_bind() {
    let h = harness(60).await; // reading needs no step-up
    let token = h.mint(&tg("7"), 7).await;
    let (_, body) = h
        .post("/api/v1/link/inspect", json!({"token": token}))
        .await;
    assert_eq!(body["state"], json!("bind"));
    assert_eq!(body["channel"], json!("telegram"));
    assert!(body["merge_item_count"].is_null());
}

#[tokio::test]
async fn inspect_with_an_unknown_token_is_expired() {
    let h = harness(0).await;
    let (status, body) = h
        .anonymous("/api/v1/link/inspect", json!({"token": "nope"}))
        .await;
    assert_eq!(
        (status, &body["state"]),
        (StatusCode::OK, &json!("expired"))
    );
    assert!(body["channel"].is_null() && body["user_id"].is_null());
}

#[tokio::test]
async fn every_link_endpoint_caps_the_token_at_500_characters() {
    let h = harness(0).await;
    let long = "x".repeat(501);
    for path in [
        "/api/v1/link/inspect",
        "/api/v1/link/claim",
        "/api/v1/link/signup/start",
    ] {
        let (status, body) = h.post(path, json!({"token": long})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert!(!body.to_string().contains(&long), "{path} echoed the token");
    }
}

#[tokio::test]
async fn signup_start_for_a_non_allowlisted_identity_is_forbidden() {
    let h = harness(0).await;
    let token = h.mint(&tg("2"), 2).await;
    let (status, body) = h
        .anonymous("/api/v1/link/signup/start", json!({"token": token}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.to_string().contains(&token));
    assert!(h.state.webauthn_ceremonies.read().await.is_empty());
}

#[tokio::test]
async fn signup_start_for_an_allowlisted_identity_returns_a_challenge() {
    let h = harness(0).await;
    let token = h.mint(&tg("1"), 1).await;
    let (status, body) = h
        .anonymous("/api/v1/link/signup/start", json!({"token": token}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["ceremony_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    assert!(body["options"].is_object());
    let before = h.state.services.accounts.count().await.unwrap();
    assert_eq!(
        before, 1,
        "starting creates no account (only the caller exists)"
    );
}

/// A structurally valid but unverifiable credential: enough to reach the
/// handler's own checks.
fn bogus_credential() -> Value {
    json!({
        "id": "AAAA",
        "rawId": "AAAA",
        "response": {"attestationObject": "AAAA", "clientDataJSON": "AAAA"},
        "type": "public-key",
        "extensions": {}
    })
}

async fn stash_signup(h: &Harness, token: &str) -> String {
    let id = AccountId::generate();
    let (_, reg_state) = h
        .state
        .webauthn
        .start_passkey_registration(id.to_handle(), "x", "x", None)
        .unwrap();
    h.state
        .stash_ceremony(Ceremony::LinkSignup {
            account_id: id,
            display_name: "x".to_string(),
            token_hash: hash_token(token),
            state: reg_state,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn signup_finish_for_a_non_allowlisted_token_is_forbidden_and_creates_no_account() {
    let h = harness(0).await;
    let token = h.mint(&tg("2"), 2).await;
    let ceremony_id = stash_signup(&h, &token).await;
    let before = h.state.services.accounts.count().await.unwrap();

    let (status, body) = h
        .anonymous(
            "/api/v1/link/signup/finish",
            json!({"ceremony_id": ceremony_id, "token": token, "credential": bogus_credential()}),
        )
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.to_string().contains(&token));
    assert_eq!(h.state.services.accounts.count().await.unwrap(), before);
    assert_eq!(h.owner(&tg("2")).await, None);
}

#[tokio::test]
async fn signup_finish_with_an_unverifiable_credential_creates_no_account() {
    let h = harness(0).await;
    let token = h.mint(&tg("1"), 1).await;
    let ceremony_id = stash_signup(&h, &token).await;
    let before = h.state.services.accounts.count().await.unwrap();

    let (status, _) = h
        .anonymous(
            "/api/v1/link/signup/finish",
            json!({"ceremony_id": ceremony_id, "token": token, "credential": bogus_credential()}),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(h.state.services.accounts.count().await.unwrap(), before);
    assert_eq!(h.owner(&tg("1")).await, None);
}

#[tokio::test]
async fn signup_finish_refuses_a_ceremony_started_for_another_token() {
    let h = harness(0).await;
    let started_for = h.mint(&tg("1"), 1).await;
    let ceremony_id = stash_signup(&h, &started_for).await;
    let other = h.mint(&tg("1"), 1).await; // burns the first token

    let (status, _) = h
        .anonymous(
            "/api/v1/link/signup/finish",
            json!({"ceremony_id": ceremony_id, "token": other, "credential": bogus_credential()}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
