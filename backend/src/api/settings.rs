use axum::{extract::State, Json};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::finance::money::normalise_currency;
use crate::modules::settings::{parse_timezone, SettingsRepository, UserSettings};

/// Both preferences are always sent together — settings are saved as a whole
/// row, so there is no partial-update path to get wrong.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    /// IANA zone name, e.g. `Asia/Shanghai`.
    pub timezone: String,
    /// ISO 4217 code, e.g. `CNY`.
    pub currency: String,
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
    let timezone = parse_timezone(&payload.timezone)
        .ok_or_else(|| ApiError::bad_request("Unknown timezone. Use an IANA name such as Asia/Shanghai"))?;

    let currency = normalise_currency(&payload.currency)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let settings = SettingsRepository::upsert(
        &state.db.pool,
        &user.owner_id(),
        &UserSettings { timezone: timezone.name().to_string(), currency },
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
        }
    }

    #[tokio::test]
    async fn test_get_returns_defaults_before_anything_is_saved() {
        let state = state().await;
        let settings = get_settings(State(state), AuthUser::named("@evan:mbm.one", "acct_@evan:mbm.one"))
            .await
            .unwrap();
        assert_eq!(settings.timezone, "UTC");
        assert_eq!(settings.currency, "USD");
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

        let read = get_settings(State(state), user).await.unwrap();
        assert_eq!(read.0, saved.0);
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
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{bad} must be rejected");
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
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{bad} must be rejected");
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

        let local = get_settings(State(state), AuthUser::named("evan", "acct_evan")).await.unwrap();
        assert_eq!(local.timezone, "UTC", "a local account must not inherit the MXID's settings");
    }
}
