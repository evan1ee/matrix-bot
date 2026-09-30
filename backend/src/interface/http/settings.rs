use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::settings as domain;
use crate::error::ApiError;
use crate::interface::http::app_error::AppResultExt;
use crate::interface::http::auth::{ApiState, AuthUser};

/// Both preferences are always sent together — settings are saved as a whole
/// row, so there is no partial-update path to get wrong.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    /// IANA zone name, e.g. `Asia/Shanghai`.
    pub timezone: String,
    /// ISO 4217 code, e.g. `CNY`.
    pub currency: String,
    /// Optional Matrix room ID for development and error notifications.
    #[serde(default)]
    pub dev_room_id: Option<String>,
    /// Optional max conversation history turns for AI (1-100).
    #[serde(default)]
    pub max_history_turns: Option<i64>,
    /// Optional toggle for morning briefing.
    #[serde(default)]
    pub briefing_enabled: Option<bool>,
    /// Optional scheduled delivery time in 24-hour HH:MM format (e.g. 07:00).
    #[serde(default)]
    pub briefing_time: Option<String>,
    /// The voice `!say` speaks in, as the configured TTS provider names it,
    /// or `null`/absent to clear back to the server default, the same
    /// convention `dev_room_id` uses.
    #[serde(default)]
    pub tts_voice: Option<String>,
}

impl From<UpdateSettingsRequest> for domain::RawSettingsUpdate {
    fn from(req: UpdateSettingsRequest) -> Self {
        Self {
            timezone: req.timezone,
            currency: req.currency,
            dev_room_id: req.dev_room_id,
            max_history_turns: req.max_history_turns,
            briefing_enabled: req.briefing_enabled,
            briefing_time: req.briefing_time,
            tts_voice: req.tts_voice,
        }
    }
}

fn default_max_history_turns() -> i64 {
    crate::domain::settings::DEFAULT_MAX_HISTORY_TURNS
}

fn default_briefing_enabled() -> bool {
    crate::domain::settings::DEFAULT_BRIEFING_ENABLED
}

fn default_briefing_time() -> String {
    crate::domain::settings::DEFAULT_BRIEFING_TIME.to_string()
}

/// One user's display preferences, keyed by the same `owner_id` every other
/// ownership check uses (`AuthUser::owner_id`, the `owner_id` column).
///
/// Deliberately shared by both front doors: the SPA renders timestamps in
/// `timezone` and pre-selects `currency`, and the Matrix command handlers format
/// their HTML replies with the same two values, so a user sees one consistent
/// answer no matter which door they came through.
//
// A `From<&domain::settings::UserSettings>` wire copy (a plain `//` comment,
// not `///`, so it stays out of the generated schema): the domain type
// derives neither `ToSchema` nor `Deserialize`. `Deserialize` is derived here
// only so the `#[serde(default = ...)]` attributes below have a real
// consumer — this type itself is never deserialized — because utoipa also
// reads them to decide which fields the schema's `required` list omits, and
// they need to stay to keep the generated schema byte-identical to the one
// the old, shared type produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct UserSettings {
    /// IANA zone name, e.g. `Asia/Shanghai`. Validated on write.
    pub timezone: String,
    /// ISO 4217 code, e.g. `CNY`. Validated on write.
    pub currency: String,
    /// Matrix room ID where development errors and alerts are sent.
    #[serde(default)]
    pub dev_room_id: Option<String>,
    /// Number of recent conversation messages included as context for AI turns.
    #[serde(default = "default_max_history_turns")]
    pub max_history_turns: i64,
    /// Whether automated morning briefing is enabled.
    #[serde(default = "default_briefing_enabled")]
    pub briefing_enabled: bool,
    /// Scheduled delivery time in 24-hour HH:MM format (e.g. 07:00).
    #[serde(default = "default_briefing_time")]
    pub briefing_time: String,
    /// The voice `!say` speaks in, in whatever the configured provider calls
    /// it. `None` means no preference, and `modules::tts` falls back to the
    /// operator's `TTS_VOICE`: the same "no row until you change something"
    /// convention `dev_room_id` uses. `Some("0")` is a real Kokoro voice, not
    /// a stand-in for "unset". Shape-checked by `parse_tts_voice` on write.
    #[serde(default)]
    pub tts_voice: Option<String>,
}

impl From<&domain::UserSettings> for UserSettings {
    fn from(s: &domain::UserSettings) -> Self {
        Self {
            timezone: s.timezone.clone(),
            currency: s.currency.clone(),
            dev_room_id: s.dev_room_id.clone(),
            max_history_turns: s.max_history_turns,
            briefing_enabled: s.briefing_enabled,
            briefing_time: s.briefing_time.clone(),
            tts_voice: s.tts_voice.clone(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/settings",
    responses(
        (status = 200, description = "The caller's settings, or the defaults", body = UserSettings),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_settings(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<UserSettings>, ApiError> {
    let settings = state
        .services
        .settings
        .get(&user.account_id)
        .await
        .api_context("Failed to read settings")?;

    Ok(Json(UserSettings::from(&settings)))
}

#[utoipa::path(
    put,
    path = "/api/v1/settings",
    request_body = UpdateSettingsRequest,
    responses(
        (status = 200, description = "The saved settings", body = UserSettings),
        (status = 400, description = "Unknown timezone or malformed currency", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_settings(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<UpdateSettingsRequest>,
) -> Result<Json<UserSettings>, ApiError> {
    let saved = state
        .services
        .settings
        .update(&user.account_id, payload.into())
        .await
        .api_context("Failed to save settings")?;

    Ok(Json(UserSettings::from(&saved)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use axum::http::StatusCode;

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_@evan:mbm.one", "acct_evan"]).await;
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    fn body(timezone: &str, currency: &str) -> UpdateSettingsRequest {
        UpdateSettingsRequest {
            timezone: timezone.to_string(),
            currency: currency.to_string(),
            dev_room_id: None,
            max_history_turns: None,
            briefing_enabled: None,
            briefing_time: None,
            tts_voice: None,
        }
    }

    #[tokio::test]
    async fn test_get_returns_defaults_before_anything_is_saved() {
        let state = state().await;
        let settings = get_settings(
            State(state),
            AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one"),
        )
        .await
        .unwrap();
        assert_eq!(settings.timezone, "UTC");
        assert_eq!(settings.currency, "USD");
        assert_eq!(settings.max_history_turns, 10);
        assert_eq!(settings.tts_voice, None);
    }

    #[tokio::test]
    async fn test_update_then_read_back() {
        let state = state().await;
        let user = AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one");

        let saved = update_settings(
            State(state.clone()),
            user.clone(),
            Json(body("Asia/Shanghai", "cny")),
        )
        .await
        .unwrap();
        // The currency is normalised on the way in, so the stored value is canonical.
        assert_eq!(saved.currency, "CNY");
        assert_eq!(saved.timezone, "Asia/Shanghai");
        assert_eq!(saved.dev_room_id, None);
        assert_eq!(saved.max_history_turns, 10);

        let read = get_settings(State(state.clone()), user.clone())
            .await
            .unwrap();
        assert_eq!(read.0, saved.0);

        // Update dev_room_id and max_history_turns
        let mut req = body("Asia/Shanghai", "CNY");
        req.dev_room_id = Some("!dev:mbm.one".to_string());
        req.max_history_turns = Some(25);
        let saved2 = update_settings(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(saved2.dev_room_id.as_deref(), Some("!dev:mbm.one"));
        assert_eq!(saved2.max_history_turns, 25);
    }

    /// An unknown zone must be refused at the boundary. Accepting it would store
    /// a value that renders as UTC on every later read, with no error anywhere to
    /// explain why the setting appeared not to work.
    #[tokio::test]
    async fn test_rejects_a_timezone_that_is_not_an_iana_name() {
        let state = state().await;

        for bad in ["GMT+8", "Mars/Olympus", "+08:00", ""] {
            let err = update_settings(
                State(state.clone()),
                AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one"),
                Json(body(bad, "USD")),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "{bad} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn test_rejects_a_malformed_currency() {
        let state = state().await;

        for bad in ["EURO", "E1R", ""] {
            let err = update_settings(
                State(state.clone()),
                AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one"),
                Json(body("UTC", bad)),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "{bad} must be rejected"
            );
        }
    }

    /// The live UI reformats every visible timestamp when this arrives, so the
    /// event has to reach the stream — and only its owner's stream.
    #[tokio::test]
    async fn test_saving_publishes_to_the_owner() {
        let state = state().await;
        let pool = state.db.pool.clone();

        let _ = update_settings(
            State(state.clone()),
            AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one"),
            Json(body("Europe/Berlin", "EUR")),
        )
        .await
        .unwrap();

        // The outbox row is keyed by the owner, so it reaches only their stream.
        assert_eq!(
            crate::infrastructure::postgres::repositories::outbox::event_types_for(
                &pool,
                &crate::domain::accounts::AccountId::from_seed("acct_@evan:mbm.one")
            )
            .await
            .unwrap(),
            vec!["SettingsUpdated"]
        );
    }

    /// The two identity spaces must not share a settings row, exactly as they do
    /// not share todos or transactions.
    #[tokio::test]
    async fn test_local_and_matrix_identities_keep_separate_settings() {
        let state = state().await;

        let _ = update_settings(
            State(state.clone()),
            AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one"),
            Json(body("Asia/Shanghai", "CNY")),
        )
        .await
        .unwrap();

        let local = get_settings(State(state), AuthUser::named_seed("evan", "acct_evan"))
            .await
            .unwrap();
        assert_eq!(
            local.timezone, "UTC",
            "a local account must not inherit the MXID's settings"
        );
    }

    #[tokio::test]
    async fn test_update_briefing_settings_and_validation() {
        let state = state().await;
        let user = AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one");

        // Valid update
        let mut req = body("Asia/Shanghai", "CNY");
        req.briefing_enabled = Some(true);
        req.briefing_time = Some("08:15".to_string());
        let saved = update_settings(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert!(saved.briefing_enabled);
        assert_eq!(saved.briefing_time, "08:15");

        // Invalid time format
        let mut bad_req = body("Asia/Shanghai", "CNY");
        bad_req.briefing_time = Some("25:99".to_string());
        let err = update_settings(State(state), user, Json(bad_req))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    /// A valid voice must be stored and read back unchanged, across the
    /// shapes different providers use: a bare index, a short name, and the
    /// longest id this project accepts. `"0"` in particular must come back
    /// as `Some("0")`, never `None` — it is a real voice, not the "unset"
    /// state.
    #[tokio::test]
    async fn test_update_stores_a_valid_tts_voice() {
        let state = state().await;
        let user = AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one");

        let mut zero_req = body("UTC", "USD");
        zero_req.tts_voice = Some("0".to_string());
        let saved_zero = update_settings(State(state.clone()), user.clone(), Json(zero_req))
            .await
            .unwrap();
        assert_eq!(saved_zero.tts_voice.as_deref(), Some("0"));

        let mut req = body("UTC", "USD");
        req.tts_voice = Some("alloy".to_string());
        let saved = update_settings(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(saved.tts_voice.as_deref(), Some("alloy"));

        let long_voice = "a".repeat(64);
        let mut boundary_req = body("UTC", "USD");
        boundary_req.tts_voice = Some(long_voice.clone());
        let saved2 = update_settings(State(state), user, Json(boundary_req))
            .await
            .unwrap();
        assert_eq!(saved2.tts_voice.as_deref(), Some(long_voice.as_str()));
    }

    /// A malformed voice must be refused at the boundary, the same way an
    /// unknown timezone is — accepting it would store a value nothing
    /// downstream expects.
    #[tokio::test]
    async fn test_rejects_a_malformed_tts_voice() {
        let state = state().await;

        let too_long = "a".repeat(65);
        for bad in ["a b", "../x", too_long.as_str()] {
            let mut req = body("UTC", "USD");
            req.tts_voice = Some(bad.to_string());
            let err = update_settings(
                State(state.clone()),
                AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one"),
                Json(req),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "{bad} must be rejected"
            );
        }
    }

    /// A saved preference clears back to "no preference" when the field comes
    /// back `null`/absent — mirroring `dev_room_id`, not the "leave the
    /// current value alone" behaviour `max_history_turns` uses.
    #[tokio::test]
    async fn test_omitting_tts_voice_clears_a_previously_saved_one() {
        let state = state().await;
        let user = AuthUser::named_seed("@evan:mbm.one", "acct_@evan:mbm.one");

        let mut req = body("UTC", "USD");
        req.tts_voice = Some("alloy".to_string());
        let _ = update_settings(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();

        let cleared = update_settings(State(state), user, Json(body("UTC", "USD")))
            .await
            .unwrap();
        assert_eq!(cleared.tts_voice, None);
    }
}
