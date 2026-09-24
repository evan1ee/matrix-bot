use axum::{extract::State, Json};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};

use crate::db::rooms::RoomRepository;
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::modules::briefing::generator::generate_briefing;
use crate::modules::briefing::models::BriefingPayload;
use crate::modules::settings::SettingsRepository;

#[utoipa::path(
    post,
    path = "/api/v1/briefing/trigger",
    responses(
        (status = 200, description = "Generated morning briefing snapshot", body = BriefingPayload),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn trigger_briefing(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<BriefingPayload>, ApiError> {
    let settings = SettingsRepository::get_or_default(&state.db.pool, &user.owner_id()).await;

    let payload = generate_briefing(&state.db.pool, &user.owner_id(), &settings)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Attempt delivery to user's direct or default Matrix room if configured
    let client_guard = state.bot_client.read().await;
    if let Some(ref client) = *client_guard {
        if let Some(target_room) =
            RoomRepository::resolve_user_default_room(&state.db.pool, &user.owner_id()).await
        {
            if let Ok(room_id) = <&RoomId>::try_from(target_room.as_str()) {
                let content =
                    RoomMessageEventContent::text_html(&payload.plain_text, &payload.html_content);
                if let Some(room) = client.get_room(room_id) {
                    let _ = room.send(content).await;
                } else if let Ok(joined) = client.join_room_by_id(room_id).await {
                    let _ = joined.send(content).await;
                }
            }
        }
    }

    state.broadcaster.send(SystemEvent::MorningBriefingSent {
        owner_id: user.owner_id(),
        delivered_at: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        summary: format!(
            "{} tasks, {} habits, {} bills, {} holdings",
            payload.summary.pending_tasks_count,
            payload.summary.habits_completed_count,
            payload.summary.upcoming_bills_count,
            payload.summary.holdings_count
        ),
    });

    Ok(Json(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_trigger_briefing_returns_payload() {
        let state = state().await;
        let user = AuthUser::named("@alice:example.org", "acct_alice");

        let res = trigger_briefing(State(state), user).await.unwrap();
        assert_eq!(res.owner_id, "acct_alice");
        assert!(res.html_content.contains("早安简报"));
        assert!(res.plain_text.contains("早安简报"));
    }
}
