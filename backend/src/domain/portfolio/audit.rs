//! Cryptographically hash-chained audit log domain types and hashing invariants.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An immutable, cryptographically signed record of a portfolio activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioAuditEntry {
    pub id: i64,
    pub owner_id: String,
    pub action: String,
    pub symbol: Option<String>,
    pub quantity_minor: Option<i64>,
    pub price_minor: Option<i64>,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub realized_pl_minor: Option<i64>,
    pub note: Option<String>,
    pub prev_hash: String,
    pub entry_hash: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditVerificationResult {
    pub is_valid: bool,
    pub total_verified: usize,
    pub genesis_hash: String,
    pub head_hash: Option<String>,
    pub broken_entry_id: Option<i64>,
    pub message: String,
}

/// Computes a deterministic SHA-256 signature for an audit record linked to `prev_hash`.
#[allow(clippy::too_many_arguments)]
pub fn compute_entry_hash(
    prev_hash: &str,
    owner_id: &str,
    action: &str,
    symbol: Option<&str>,
    quantity_minor: Option<i64>,
    price_minor: Option<i64>,
    amount_minor: i64,
    currency: &str,
    currency_exponent: i64,
    realized_pl_minor: Option<i64>,
    note: Option<&str>,
    created_at_epoch_ms: i64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(b":");
    hasher.update(owner_id.as_bytes());
    hasher.update(b":");
    hasher.update(action.as_bytes());
    hasher.update(b":");
    hasher.update(symbol.unwrap_or("").as_bytes());
    hasher.update(b":");
    hasher.update(quantity_minor.unwrap_or(0).to_string().as_bytes());
    hasher.update(b":");
    hasher.update(price_minor.unwrap_or(0).to_string().as_bytes());
    hasher.update(b":");
    hasher.update(amount_minor.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(currency.as_bytes());
    hasher.update(b":");
    hasher.update(currency_exponent.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(realized_pl_minor.unwrap_or(0).to_string().as_bytes());
    hasher.update(b":");
    hasher.update(note.unwrap_or("").as_bytes());
    hasher.update(b":");
    hasher.update(created_at_epoch_ms.to_string().as_bytes());

    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_entry_hash_deterministic() {
        let hash1 = compute_entry_hash(
            GENESIS_HASH,
            "user1",
            "buy",
            Some("AAPL"),
            Some(1000000000),
            Some(15000),
            150000,
            "USD",
            2,
            None,
            Some("initial buy"),
            1700000000000,
        );
        let hash2 = compute_entry_hash(
            GENESIS_HASH,
            "user1",
            "buy",
            Some("AAPL"),
            Some(1000000000),
            Some(15000),
            150000,
            "USD",
            2,
            None,
            Some("initial buy"),
            1700000000000,
        );
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_compute_entry_hash_changes_with_any_field() {
        let base = compute_entry_hash(
            GENESIS_HASH,
            "user1",
            "buy",
            Some("AAPL"),
            Some(1000),
            Some(150),
            150,
            "USD",
            2,
            None,
            None,
            1700000000000,
        );
        let changed = compute_entry_hash(
            GENESIS_HASH,
            "user1",
            "sell",
            Some("AAPL"),
            Some(1000),
            Some(150),
            150,
            "USD",
            2,
            None,
            None,
            1700000000000,
        );
        assert_ne!(base, changed);
    }
}
