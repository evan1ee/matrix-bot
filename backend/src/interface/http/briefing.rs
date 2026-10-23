use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
use crate::interface::jobs::briefing::BriefingSendHandler;
use crate::interface::jobs::deliverable;

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
    #[serde(default)]
    pub budgets_attention_count: usize,
}

impl From<crate::application::briefing::models::BriefingSummary> for BriefingSummary {
    fn from(s: crate::application::briefing::models::BriefingSummary) -> Self {
        Self {
            pending_tasks_count: s.pending_tasks_count,
            habits_completed_count: s.habits_completed_count,
            habits_total_count: s.habits_total_count,
            upcoming_bills_count: s.upcoming_bills_count,
            holdings_count: s.holdings_count,
            budgets_attention_count: s.budgets_attention_count,
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

    // Delegate external channel delivery to BriefingSendHandler so scheduled
    // and on-demand briefings share one delivery path, and surface send
    // failures rather than returning 200 when nothing was delivered.
    if state
        .services
        .rooms
        .default_room(&user.account_id)
        .await
        .as_deref()
        .and_then(deliverable)
        .is_some_and(|(_, ch)| state.services.rooms.is_channel_configured(ch))
    {
        let registry = state
            .outbound
            .as_ref()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| {
                ApiError::service_unavailable("Outbound channel registry is not available")
            })?;
        let handler = BriefingSendHandler::new(state.services.clone(), registry);
        handler
            .deliver_on_demand(&user.account_id, &payload)
            .await
            .map_err(|err| {
                ApiError::service_unavailable(format!("Briefing delivery failed: {err}"))
            })?;
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
    use std::sync::Arc;

    use super::*;
    use crate::domain::channels::{Channel, PlatformIdentity, PlatformUserId};
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::registry::ChannelRegistry;

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

    #[tokio::test]
    async fn test_trigger_briefing_delivers_to_default_channel_and_surfaces_failures() {
        let mut state = state().await;
        let user = AuthUser::named_seed("@alice:example.org", "acct_alice");
        let tg = PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, "777").unwrap(),
            Some("Alice".into()),
        );
        state
            .services
            .identity
            .bind_for_test(&user.account_id, &tg)
            .await;

        // When the configured channel is disconnected, trigger_briefing must fail
        // rather than quietly returning 200 OK.
        let down = Arc::new(FakeAdapter::disconnected(Channel::Telegram));
        let reg_down = Arc::new(ChannelRegistry::new(vec![down]).unwrap());
        state.outbound = Some(Arc::downgrade(&reg_down));

        let err = trigger_briefing(State(state.clone()), user.clone())
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);

        // When connected, trigger_briefing delivers through BriefingSendHandler and returns 200.
        let up = Arc::new(FakeAdapter::connected(Channel::Telegram));
        let reg_up = Arc::new(ChannelRegistry::new(vec![up.clone()]).unwrap());
        state.outbound = Some(Arc::downgrade(&reg_up));

        let res = trigger_briefing(State(state), user).await.unwrap();
        assert!(res.html_content.contains("早安简报"));
        assert_eq!(up.sent().len(), 1);
    }
}
