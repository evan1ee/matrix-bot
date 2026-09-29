use axum::{extract::State, Json};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::interface::http::auth::{ApiState, AuthUser};

/// Market performance snapshot for a single holding across 1D, 1W, and 1M timeframes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TickerPerformance {
    pub symbol: String,
    pub currency: String,
    pub current_price: f64,
    pub change_1d_pct: Option<f64>,
    pub change_1w_pct: Option<f64>,
    pub change_1m_pct: Option<f64>,
}

impl From<crate::application::briefing::models::TickerPerformance> for TickerPerformance {
    fn from(t: crate::application::briefing::models::TickerPerformance) -> Self {
        Self {
            symbol: t.symbol,
            currency: t.currency,
            current_price: t.current_price,
            change_1d_pct: t.change_1d_pct,
            change_1w_pct: t.change_1w_pct,
            change_1m_pct: t.change_1m_pct,
        }
    }
}

/// Key statistics summarizing a generated morning briefing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct BriefingSummary {
    pub pending_tasks_count: usize,
    pub habits_completed_count: usize,
    pub habits_total_count: usize,
    pub upcoming_bills_count: usize,
    pub holdings_count: usize,
}

impl From<crate::application::briefing::models::BriefingSummary> for BriefingSummary {
    fn from(s: crate::application::briefing::models::BriefingSummary) -> Self {
        Self {
            pending_tasks_count: s.pending_tasks_count,
            habits_completed_count: s.habits_completed_count,
            habits_total_count: s.habits_total_count,
            upcoming_bills_count: s.upcoming_bills_count,
            holdings_count: s.holdings_count,
        }
    }
}

/// Formatted briefing payload ready for Matrix delivery and REST preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct BriefingPayload {
    pub owner_id: String,
    pub html_content: String,
    pub plain_text: String,
    pub summary: BriefingSummary,
    pub tickers: Vec<TickerPerformance>,
}

impl From<crate::application::briefing::models::BriefingPayload> for BriefingPayload {
    fn from(p: crate::application::briefing::models::BriefingPayload) -> Self {
        Self {
            owner_id: p.owner_id,
            html_content: p.html_content,
            plain_text: p.plain_text,
            summary: BriefingSummary::from(p.summary),
            tickers: p.tickers.into_iter().map(TickerPerformance::from).collect(),
        }
    }
}

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
    let settings = state
        .services
        .settings
        .get_or_default(&user.account_id)
        .await;

    let payload = state
        .services
        .briefing
        .generate_briefing(&user.account_id, &settings)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Attempt delivery to user's direct or default Matrix room if configured
    let client_guard = state.bot_client.read().await;
    if let Some(ref client) = *client_guard {
        if let Some(target_room) = state.services.rooms.default_room(&user.account_id).await {
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

    Ok(Json(BriefingPayload::from(payload)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice"]).await;
        ApiState::new(
            crate::db::Database { pool },
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
        assert_eq!(
            res.owner_id,
            crate::domain::accounts::AccountId::from("acct_alice").to_string()
        );
        assert!(res.html_content.contains("早安简报"));
        assert!(res.plain_text.contains("早安简报"));
    }
}
