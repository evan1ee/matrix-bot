//! Immutable, cryptographically hash-chained audit log for all portfolio activities:
//! buy, sell, deposit, withdraw, and position recording.
//!
//! Each entry stores both its own deterministic SHA-256 digest (`entry_hash`) and the
//! hash of the preceding entry (`prev_hash`), forming a tamper-evident blockchain-style
//! ledger. SQLite triggers additionally enforce immutability at the database engine level.

use crate::db::DbPool;
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
    pool: &DbPool,
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

    let prev_hash = crate::db_scalar_optional!(
        pool,
        String,
        "SELECT entry_hash FROM portfolio_audit_log WHERE owner_id = ? ORDER BY id DESC LIMIT 1",
        owner_id
    )?
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

    let result = crate::db_scalar_one!(
        pool,
        i64,
        "INSERT INTO portfolio_audit_log \
         (owner_id, action, symbol, quantity_minor, price_minor, amount_minor, currency, \
          currency_exponent, realized_pl_minor, note, prev_hash, entry_hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
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
        &prev_hash,
        &entry_hash,
        now
    )?;

    let inserted_id = result;

    let entry = crate::db_fetch_one!(
        pool,
        PortfolioAuditEntry,
        &format!("SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log WHERE id = ?"),
        inserted_id
    )?;

    Ok(entry)
}

/// Moves `from`'s ledger onto `into` and recomputes the surviving chain.
///
/// The ledger is immutable by trigger and its hashes are bound to `owner_id`,
/// so a merge cannot re-point these rows the way it re-points every other
/// owned table. It drops the immutability trigger for the length of the
/// transaction, moves the rows, appends an entry recording the absorption, and
/// rebuilds every hash for the surviving owner from the genesis hash.
///
/// Nothing a user entered is altered: action, symbol, quantity, price, amount,
/// currency, realised P&L, note and timestamp all survive untouched. Only the
/// chain linkage is recomputed, and it has to be, because `owner_id` is part
/// of every digest. The rebuild is itself recorded in the ledger rather than
/// being silent.
///
/// Returns how many rows moved. Zero means `from` had no ledger, in which case
/// nothing is dropped, appended or rebuilt.
///
/// The caller must already be inside a transaction. If it rolls back, SQLite
/// undoes the `DROP TRIGGER` with everything else, so the trigger cannot be
/// left off.
pub async fn absorb_ledger(
    conn: &mut sqlx::SqliteConnection,
    from: &str,
    into: &str,
) -> Result<i64, sqlx::Error> {
    let moved: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = ?")
            .bind(from)
            .fetch_one(&mut *conn)
            .await?;

    if moved == 0 {
        return Ok(0);
    }

    // `crate::db::ddl` is the single source for these statements so the schema
    // and this drop/restore pair cannot drift apart. Pinned to SQLite because
    // this function still takes a `SqliteConnection`; it moves to the pool's
    // dialect when transactions convert in stage 6.
    sqlx::query(&crate::db::ddl::drop_audit_update_trigger(
        crate::db::Dialect::Sqlite,
    ))
    .execute(&mut *conn)
    .await?;

    sqlx::query("UPDATE portfolio_audit_log SET owner_id = ? WHERE owner_id = ?")
        .bind(into)
        .bind(from)
        .execute(&mut *conn)
        .await?;

    // Placeholder hashes below are overwritten by the rebuild pass that
    // follows; there is no INSERT trigger, so this row is allowed to land
    // with them briefly incorrect.
    sqlx::query(
        "INSERT INTO portfolio_audit_log \
         (owner_id, action, symbol, quantity_minor, price_minor, amount_minor, currency, \
          currency_exponent, realized_pl_minor, note, prev_hash, entry_hash, created_at) \
         VALUES (?, 'merge', NULL, NULL, NULL, 0, '', 2, NULL, ?, ?, '', ?)",
    )
    .bind(into)
    .bind(format!(
        "Absorbed {moved} ledger entries from account {from}"
    ))
    .bind(GENESIS_HASH)
    .bind(Utc::now())
    .execute(&mut *conn)
    .await?;

    // Ordered by `id` ascending: ids come from a single global autoincrement,
    // so id order is insertion order across both accounts, which gives a
    // chronologically coherent merged history.
    let entries = sqlx::query_as::<_, PortfolioAuditEntry>(&format!(
        "SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log WHERE owner_id = ? ORDER BY id ASC"
    ))
    .bind(into)
    .fetch_all(&mut *conn)
    .await?;

    let mut prev = GENESIS_HASH.to_string();
    for entry in &entries {
        let epoch_ms = entry.created_at.map(|t| t.timestamp_millis()).unwrap_or(0);
        let entry_hash = compute_entry_hash(
            &prev,
            into,
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

        sqlx::query("UPDATE portfolio_audit_log SET prev_hash = ?, entry_hash = ? WHERE id = ?")
            .bind(&prev)
            .bind(&entry_hash)
            .bind(entry.id)
            .execute(&mut *conn)
            .await?;

        prev = entry_hash;
    }

    for statement in crate::db::ddl::audit_update_trigger(crate::db::Dialect::Sqlite) {
        sqlx::query(&statement).execute(&mut *conn).await?;
    }

    Ok(moved)
}

/// Retrieves the paginated audit activity log for an owner in reverse chronological order.
pub async fn list_audit_entries(
    pool: &DbPool,
    owner_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<PortfolioAuditEntry>> {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);

    let entries = crate::db_fetch_all!(
        pool,
        PortfolioAuditEntry,
        &format!(
            "SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log \
         WHERE owner_id = ? \
         ORDER BY id DESC \
         LIMIT ? OFFSET ?"
        ),
        owner_id,
        limit,
        offset
    )?;

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
pub async fn verify_audit_chain(pool: &DbPool, owner_id: &str) -> Result<AuditVerificationResult> {
    let entries = crate::db_fetch_all!(
        pool,
        PortfolioAuditEntry,
        &format!(
            "SELECT {AUDIT_COLUMNS} FROM portfolio_audit_log WHERE owner_id = ? ORDER BY id ASC"
        ),
        owner_id
    )?;

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

    async fn pool() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
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
        let update_res = crate::db_execute!(
            &pool,
            "UPDATE portfolio_audit_log SET amount_minor = 999999 WHERE id = ?",
            e.id
        );
        assert!(update_res.is_err(), "Trigger must abort UPDATE");

        // Direct DELETE must be rejected by trigger
        let delete_res =
            crate::db_execute!(&pool, "DELETE FROM portfolio_audit_log WHERE id = ?", e.id);
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
