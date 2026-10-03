pub mod alerts;
pub mod audit;
pub mod history;
pub mod models;

use chrono::{DateTime, Utc};
use thiserror::Error;

pub use alerts::{AlertCondition, CreatePriceAlert, PriceAlert};
pub use audit::{compute_entry_hash, AuditVerificationResult, PortfolioAuditEntry, GENESIS_HASH};
pub use history::{DailySnapshot, PortfolioHistoryPoint, PortfolioHistoryResponse};
pub use models::{
    build_summaries, cost_of, normalise_symbol, realized_pl_minor, weighted_avg_cost_minor,
    BuyOutcome, CashBalance, Holding, PortfolioCurrencySummary, SellOutcome, WithdrawOutcome,
    QUANTITY_EXPONENT,
};

/// Maximum allowed length for a holding's note.
pub const MAX_NOTE_LEN: usize = 500;

/// A buy or record to perform.
pub struct TradeRequest<'a> {
    pub symbol: &'a str,
    pub quantity: &'a str,
    pub price: &'a str,
    pub currency: &'a str,
    pub note: Option<&'a str>,
    pub executed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum PortfolioError {
    #[error("Note exceeds {max} character limit")]
    NoteTooLong { max: usize },

    #[error("{0}")]
    Invalid(String),
}

/// Checks a note's length and normalises it.
pub fn validate_note(note: Option<&str>) -> Result<Option<&str>, PortfolioError> {
    match note {
        Some(n) if n.len() > MAX_NOTE_LEN => Err(PortfolioError::NoteTooLong { max: MAX_NOTE_LEN }),
        Some(n) if n.trim().is_empty() => Ok(None),
        other => Ok(other),
    }
}
