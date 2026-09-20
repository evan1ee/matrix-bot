use axum::{extract::State, Json};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::finance::money::normalise_currency;
use crate::modules::settings::models::MAX_TTS_SPEAKER_ID;
use crate::modules::settings::{parse_timezone, SettingsRepository, UserSettings};

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
    /// Kokoro TTS speaker index (0-1023), or `null`/absent to clear back to no
    /// preference — the same convention `dev_room_id` uses, and for the same
    /// reason: `0` is a real, selectable voice, so it cannot also mean "unset".
    #[serde(default)]
    pub tts_speaker_id: Option<i64>,
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
    let settings = SettingsRepository::get(&state.db.pool, &user.owner_id())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(settings))
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
    // Validated here rather than at render time: an unknown zone stored now
    // would silently render as UTC forever, with nothing to point at.
    let timezone = parse_timezone(&payload.timezone).ok_or_else(|| {
        ApiError::bad_request("Unknown timezone. Use an IANA name such as Asia/Shanghai")
    })?;

    let currency =
        normalise_currency(&payload.currency).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let dev_room_id = payload.dev_room_id.and_then(|r| {
        let trimmed = r.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let current = SettingsRepository::get_or_default(&state.db.pool, &user.owner_id()).await;
    let max_history_turns = if let Some(turns) = payload.max_history_turns {
        if !(1..=100).contains(&turns) {
            return Err(ApiError::bad_request(
                "max_history_turns must be between 1 and 100",
            ));
        }
        turns
    } else {
        current.max_history_turns
    };

    let briefing_enabled = payload.briefing_enabled.unwrap_or(current.briefing_enabled);
    let briefing_time = if let Some(ref bt) = payload.briefing_time {
        crate::modules::settings::models::parse_briefing_time(bt)
            .map_err(|e| ApiError::bad_request(e))?;
        bt.trim().to_string()
    } else {
        current.briefing_time
    };

    // Unlike `max_history_turns` or `briefing_time`, a missing/null value here
    // is not "leave the current value alone" — it directly clears the
    // preference, the same as `dev_room_id` above. A present value is still
    // validated: it is written straight into the row, never merged against
    // `current`.
    let tts_speaker_id = match payload.tts_speaker_id {
        Some(id) if (0..=MAX_TTS_SPEAKER_ID).contains(&id) => Some(id),
        Some(_) => {
            return Err(ApiError::bad_request(format!(
                "tts_speaker_id must be between 0 and {}",
                MAX_TTS_SPEAKER_ID
            )));
        }
        None => None,
    };

    let settings = SettingsRepository::upsert(
        &state.db.pool,
        &user.owner_id(),
        &UserSettings {
            timezone: timezone.name().to_string(),
            currency,
            dev_room_id,
            max_history_turns,
            briefing_enabled,
            briefing_time,
            tts_speaker_id,
        },
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::SettingsUpdated {
        owner_id: user.owner_id(),
        settings: settings.clone(),
    });

    Ok(Json(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use axum::http::StatusCode;
    use sqlx::SqlitePool;

    async fn state() -> ApiState {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
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
            tts_speaker_id: None,
        }
    }

    #[tokio::test]
    async fn test_get_returns_defaults_before_anything_is_saved() {
        let state = state().await;
        let settings = get_settings(
            State(state),
            AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"),
        )
        .await
        .unwrap();
        assert_eq!(settings.timezone, "UTC");
        assert_eq!(settings.currency, "USD");
        assert_eq!(settings.max_history_turns, 10);
        assert_eq!(settings.tts_speaker_id, None);
    }

    #[tokio::test]
    async fn test_update_then_read_back() {
        let state = state().await;
        let user = AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one");

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
                AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"),
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
                AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"),
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
    async fn test_saving_broadcasts_to_the_owner() {
        let state = state().await;
        let mut rx = state.broadcaster.subscribe();

        let _ = update_settings(
            State(state.clone()),
            AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"),
            Json(body("Europe/Berlin", "EUR")),
        )
        .await
        .unwrap();

        let event = rx.try_recv().unwrap();
        assert_eq!(event.owner_id(), "acct_@evan:mbm.one");
        assert!(matches!(event, SystemEvent::SettingsUpdated { .. }));
    }

    /// The two identity spaces must not share a settings row, exactly as they do
    /// not share todos or transactions.
    #[tokio::test]
    async fn test_local_and_matrix_identities_keep_separate_settings() {
        let state = state().await;

        let _ = update_settings(
            State(state.clone()),
            AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"),
            Json(body("Asia/Shanghai", "CNY")),
        )
        .await
        .unwrap();

        let local = get_settings(State(state), AuthUser::named("evan", "acct_evan"))
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
        let user = AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one");

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

    /// A valid speaker id must be stored and read back unchanged; boundary
    /// values 0 and `MAX_TTS_SPEAKER_ID` are both in range and must not be
    /// rejected as if they were the "outside the ceiling" case. Speaker 0 in
    /// particular must come back as `Some(0)`, never `None` — it is a real
    /// voice, not the "unset" state.
    #[tokio::test]
    async fn test_update_stores_a_valid_tts_speaker_id() {
        let state = state().await;
        let user = AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one");

        let mut zero_req = body("UTC", "USD");
        zero_req.tts_speaker_id = Some(0);
        let saved_zero = update_settings(State(state.clone()), user.clone(), Json(zero_req))
            .await
            .unwrap();
        assert_eq!(saved_zero.tts_speaker_id, Some(0));

        let mut req = body("UTC", "USD");
        req.tts_speaker_id = Some(42);
        let saved = update_settings(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(saved.tts_speaker_id, Some(42));

        let mut boundary_req = body("UTC", "USD");
        boundary_req.tts_speaker_id = Some(MAX_TTS_SPEAKER_ID);
        let saved2 = update_settings(State(state), user, Json(boundary_req))
            .await
            .unwrap();
        assert_eq!(saved2.tts_speaker_id, Some(MAX_TTS_SPEAKER_ID));
    }

    /// A negative id or one past the ceiling must be refused at the boundary,
    /// the same way an unknown timezone is — accepting it would store a value
    /// nothing downstream expects.
    #[tokio::test]
    async fn test_rejects_a_tts_speaker_id_outside_the_valid_range() {
        let state = state().await;

        for bad in [-1, MAX_TTS_SPEAKER_ID + 1] {
            let mut req = body("UTC", "USD");
            req.tts_speaker_id = Some(bad);
            let err = update_settings(
                State(state.clone()),
                AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"),
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
    async fn test_omitting_tts_speaker_id_clears_a_previously_saved_one() {
        let state = state().await;
        let user = AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one");

        let mut req = body("UTC", "USD");
        req.tts_speaker_id = Some(9);
        let _ = update_settings(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();

        let cleared = update_settings(State(state), user, Json(body("UTC", "USD")))
            .await
            .unwrap();
        assert_eq!(cleared.tts_speaker_id, None);
    }
}
