//! Immutable, cryptographically hash-chained audit log for all portfolio activities:
//! buy, sell, deposit, withdraw, and position recording.
//!
//! Each entry stores both its own deterministic SHA-256 digest (`entry_hash`) and the
//! hash of the preceding entry (`prev_hash`), forming a tamper-evident blockchain-style
//! ledger. SQLite triggers additionally enforce immutability at the database engine level.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use utoipa::ToSchema;

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An immutable, cryptographically signed record of a portfolio activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct PortfolioAuditEntry {
    pub id: i64,
    pub owner_id: String,
    /// Operation kind: "buy", "sell", "deposit", "withdraw", "record"
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

const AUDIT_COLUMNS: &str = "id, owner_id, action, symbol, quantity_minor, price_minor, \
                             amount_minor, currency, currency_exponent, realized_pl_minor, \
                             note, prev_hash, entry_hash, created_at";

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

/// Records a cryptographically hash-chained audit entry for a portfolio operation.
#[allow(clippy::too_many_arguments)]
pub async fn record_audit_entry(
    pool: &SqlitePool,
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
    custom_timestamp: Option<DateTime<Utc>>,
) -> Result<PortfolioAuditEntry> {
    let now = custom_timestamp.unwrap_or_else(Utc::now);
    let epoch_ms = now.timestamp_millis();

    let prev_hash = sqlx::query_scalar::<_, String>(
        "SELECT entry_hash FROM portfolio_audit_log WHERE owner_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(owner_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or_else(|| GENESIS_HASH.to_string());

    let entry_hash = compute_entry_hash(
        &prev_hash,
        owner_id,
        action,
        symbol,
        quantity_minor,
        price_minor,
        amount_minor,
        currency,
        currency_exponent,
        realized_pl_minor,
        note,
        epoch_ms,
    );

    let result = sqlx::query(
        "INSERT INTO portfolio_audit_log \
         (owner_id, action, symbol, quantity_minor, price_minor, amount_minor, currency, \
          currency_exponent, realized_pl_minor, note, prev_hash, entry_hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner_id)
    .bind(action)
    .bind(symbol)
    .bind(quantity_minor)
    .bind(price_minor)
    .bind(amount_minor)
    .bind(currency)
    .bind(currency_exponent)
    .bind(realized_pl_minor)
    .bind(note)
    .bind(&prev_hash)
    .bind(&entry_hash)
    .bind(now)
    .execute(pool)
    .await?;

    let inserted_id = result.last_insert_rowid();

    let entry = sqlx::query_as::<_, PortfolioAuditEntry>(&format!(
        "SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log WHERE id = ?"
    ))
    .bind(inserted_id)
    .fetch_one(pool)
    .await?;

    Ok(entry)
}

/// Retrieves the paginated audit activity log for an owner in reverse chronological order.
pub async fn list_audit_entries(
    pool: &SqlitePool,
    owner_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<PortfolioAuditEntry>> {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);

    let entries = sqlx::query_as::<_, PortfolioAuditEntry>(&format!(
        "SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log \
         WHERE owner_id = ? \
         ORDER BY id DESC \
         LIMIT ? OFFSET ?"
    ))
    .bind(owner_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(entries)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditVerificationResult {
    pub is_valid: bool,
    pub total_verified: usize,
    pub genesis_hash: String,
    pub head_hash: Option<String>,
    pub broken_entry_id: Option<i64>,
    pub message: String,
}

/// Verifies the cryptographic integrity of an owner's audit log from genesis to head.
pub async fn verify_audit_chain(
    pool: &SqlitePool,
    owner_id: &str,
) -> Result<AuditVerificationResult> {
    let entries = sqlx::query_as::<_, PortfolioAuditEntry>(&format!(
        "SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log WHERE owner_id = ? ORDER BY id ASC"
    ))
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    if entries.is_empty() {
        return Ok(AuditVerificationResult {
            is_valid: true,
            total_verified: 0,
            genesis_hash: GENESIS_HASH.to_string(),
            head_hash: None,
            broken_entry_id: None,
            message: "Audit ledger is empty (no operations recorded yet)".to_string(),
        });
    }

    let mut expected_prev_hash = GENESIS_HASH.to_string();

    for entry in &entries {
        if entry.prev_hash != expected_prev_hash {
            return Ok(AuditVerificationResult {
                is_valid: false,
                total_verified: entries.len(),
                genesis_hash: GENESIS_HASH.to_string(),
                head_hash: None,
                broken_entry_id: Some(entry.id),
                message: format!(
                    "Chain broken at entry #{}: prev_hash does not match previous entry's hash",
                    entry.id
                ),
            });
        }

        let epoch_ms = entry.created_at.map(|t| t.timestamp_millis()).unwrap_or(0);
        let computed = compute_entry_hash(
            &entry.prev_hash,
            &entry.owner_id,
            &entry.action,
            entry.symbol.as_deref(),
            entry.quantity_minor,
            entry.price_minor,
            entry.amount_minor,
            &entry.currency,
            entry.currency_exponent,
            entry.realized_pl_minor,
            entry.note.as_deref(),
            epoch_ms,
        );

        if computed != entry.entry_hash {
            return Ok(AuditVerificationResult {
                is_valid: false,
                total_verified: entries.len(),
                genesis_hash: GENESIS_HASH.to_string(),
                head_hash: None,
                broken_entry_id: Some(entry.id),
                message: format!(
                    "Data alteration detected at entry #{}: cryptographic signature mismatch",
                    entry.id
                ),
            });
        }

        expected_prev_hash = entry.entry_hash.clone();
    }

    Ok(AuditVerificationResult {
        is_valid: true,
        total_verified: entries.len(),
        genesis_hash: GENESIS_HASH.to_string(),
        head_hash: Some(expected_prev_hash),
        broken_entry_id: None,
        message: format!(
            "Cryptographic integrity verified: all {} entries mathematically intact",
            entries.len()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_audit_log_crud_and_ordering() {
        let pool = pool().await;
        let owner = "acct_alice";

        let e1 = record_audit_entry(
            &pool,
            owner,
            "deposit",
            None,
            None,
            None,
            500000,
            "USD",
            2,
            None,
            Some("wire deposit"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(e1.action, "deposit");
        assert_eq!(e1.prev_hash, GENESIS_HASH);
        assert!(!e1.entry_hash.is_empty());

        let e2 = record_audit_entry(
            &pool,
            owner,
            "buy",
            Some("AAPL"),
            Some(10_00000000),
            Some(15000),
            150000,
            "USD",
            2,
            None,
            Some("initial buy"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(e2.action, "buy");
        assert_eq!(e2.prev_hash, e1.entry_hash);

        let list = list_audit_entries(&pool, owner, 10, 0).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, e2.id);
        assert_eq!(list[1].id, e1.id);
    }

    #[tokio::test]
    async fn test_verify_audit_chain_intact() {
        let pool = pool().await;
        let owner = "acct_bob";

        for i in 1..=5 {
            record_audit_entry(
                &pool,
                owner,
                "deposit",
                None,
                None,
                None,
                i * 10000,
                "USD",
                2,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        }

        let report = verify_audit_chain(&pool, owner).await.unwrap();
        assert!(report.is_valid);
        assert_eq!(report.total_verified, 5);
        assert!(report.head_hash.is_some());
    }

    #[tokio::test]
    async fn test_sqlite_triggers_prevent_update_and_delete() {
        let pool = pool().await;
        let owner = "acct_charlie";

        let e = record_audit_entry(
            &pool, owner, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        // Direct UPDATE must be rejected by trigger
        let update_res =
            sqlx::query("UPDATE portfolio_audit_log SET amount_minor = 999999 WHERE id = ?")
                .bind(e.id)
                .execute(&pool)
                .await;
        assert!(update_res.is_err(), "Trigger must abort UPDATE");

        // Direct DELETE must be rejected by trigger
        let delete_res = sqlx::query("DELETE FROM portfolio_audit_log WHERE id = ?")
            .bind(e.id)
            .execute(&pool)
            .await;
        assert!(delete_res.is_err(), "Trigger must abort DELETE");
    }

    #[tokio::test]
    async fn test_custom_timestamp_audit_chain() {
        let pool = pool().await;
        let owner = "acct_dave";

        let past_time = Utc::now() - chrono::Duration::days(7);
        let e = record_audit_entry(
            &pool,
            owner,
            "buy",
            Some("AAPL"),
            Some(10_00000000),
            Some(15000),
            150000,
            "USD",
            2,
            None,
            Some("bought a week ago"),
            Some(past_time),
        )
        .await
        .unwrap();

        assert_eq!(
            e.created_at.unwrap().timestamp_millis(),
            past_time.timestamp_millis()
        );

        let report = verify_audit_chain(&pool, owner).await.unwrap();
        assert!(report.is_valid);
        assert_eq!(report.total_verified, 1);
    }
}
