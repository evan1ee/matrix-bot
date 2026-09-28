use serde::{Deserialize, Serialize};

/// Market performance snapshot for a single holding across 1D, 1W, and 1M timeframes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickerPerformance {
    pub symbol: String,
    pub currency: String,
    pub current_price: f64,
    pub change_1d_pct: Option<f64>,
    pub change_1w_pct: Option<f64>,
    pub change_1m_pct: Option<f64>,
}

/// Key statistics summarizing a generated morning briefing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefingSummary {
    pub pending_tasks_count: usize,
    pub habits_completed_count: usize,
    pub habits_total_count: usize,
    pub upcoming_bills_count: usize,
    pub holdings_count: usize,
}

/// Formatted briefing payload ready for Matrix delivery and REST preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BriefingPayload {
    pub owner_id: String,
    pub html_content: String,
    pub plain_text: String,
    pub summary: BriefingSummary,
    pub tickers: Vec<TickerPerformance>,
}
