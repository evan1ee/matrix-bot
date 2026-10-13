use axum::{extract::State, Json};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

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

    // Attempt delivery to user's direct or default room/channel if configured and authorized.
    // Never call `join_room_by_id` on an unverified room.
    if let Some(target_room) = state.services.rooms.default_room(&user.account_id).await {
        if target_room.starts_with('!') {
            let client_guard = state.bot_client.read().await;
            if let Some(ref client) = *client_guard {
                if crate::interface::channels::matrix::rooms::is_room_authorized_for_account(
                    &state.services.rooms,
                    Some(client),
                    &user.account_id,
                    &target_room,
                )
                .await
                {
                    if let Ok(room_id) = <&RoomId>::try_from(target_room.as_str()) {
                        if let Some(room) = client.get_room(room_id) {
                            let content = RoomMessageEventContent::text_html(
                                &payload.plain_text,
                                &payload.html_content,
                            );
                            let _ = room.send(content).await;
                        }
                    }
                }
            }
        } else if let Some(registry) = state.outbound.as_ref().and_then(|w| w.upgrade()) {
            if let Ok(addr) =
                crate::domain::channels::ConversationAddress::parse_storage(&target_room)
            {
                if registry
                    .authorize_delivery(&user.account_id, &addr)
                    .await
                    .is_ok()
                {
                    let msg = crate::interface::channels::message::OutgoingMessage::to(
                        crate::interface::channels::message::Target::Conversation(addr),
                        crate::interface::channels::message::Reply {
                            content: crate::interface::channels::message::Content {
                                plain: payload.plain_text.clone(),
                                html: Some(payload.html_content.clone()),
                                ..crate::interface::channels::message::Content::default()
                            },
                            visibility: crate::interface::channels::message::Visibility::Normal,
                        },
                    );
                    let _ = registry.send(msg).await;
                }
            }
        }
    }

    if let Err(err) = state
        .services
        .briefing
        .announce_manual(&user.account_id, &payload)
        .await
    {
        tracing::warn!(owner_id = %user.account_id, error = %err, "Failed to publish MorningBriefingSent");
    }

    Ok(Json(BriefingPayload::from(payload)))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice"]).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_trigger_briefing_returns_payload() {
        let state = state().await;
        let user = AuthUser::named_seed("@alice:example.org", "acct_alice");

        let res = trigger_briefing(State(state), user).await.unwrap();
        assert_eq!(
            res.owner_id,
            crate::domain::accounts::AccountId::from_seed("acct_alice").to_string()
        );
        assert!(res.html_content.contains("早安简报"));
        assert!(res.plain_text.contains("早安简报"));
    }
}
