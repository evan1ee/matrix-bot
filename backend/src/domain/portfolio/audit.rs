//! Cryptographically hash-chained audit log domain types and hashing invariants.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An immutable, cryptographically signed record of a portfolio activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioAuditEntry {
    pub id: i64,
    pub owner_id: AccountId,
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
    // The colon-joined preimage is the stored chain's format; the pinned
    // digest test below fails if a single byte of it changes.
    let preimage = format!(
        "{prev_hash}:{owner_id}:{action}:{}:{}:{}:{amount_minor}:{currency}:\
         {currency_exponent}:{}:{}:{created_at_epoch_ms}",
        symbol.unwrap_or(""),
        quantity_minor.unwrap_or(0),
        price_minor.unwrap_or(0),
        realized_pl_minor.unwrap_or(0),
        note.unwrap_or(""),
    );
    crate::utils::secure::sha256_hex(preimage.as_bytes())
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

    /// Historical audit chains were hashed over the `acct_<32 hex>` text id.
    /// Migrating the column to UUID must not change a single digest, or every
    /// existing chain would fail verification. The expected value was computed
    /// independently (SHA-256 over the colon-joined fields), not by this function.
    #[test]
    fn test_compute_entry_hash_matches_pre_migration_pinned_digest() {
        let account: AccountId = "acct_26ab01c0f4c19b7d854f46dca154ee89".parse().unwrap();
        assert_eq!(account.to_string(), "acct_26ab01c0f4c19b7d854f46dca154ee89");
        let hash = compute_entry_hash(
            GENESIS_HASH,
            &account.to_string(),
            "deposit",
            None,
            None,
            None,
            100000,
            "USD",
            2,
            None,
            Some("Initial deposit"),
            1700000000000,
        );
        assert_eq!(
            hash,
            "72ca1f928a023b967e62543a2140a699c27afd81865fbef37b422ea07186ae3b"
        );
    }
}
