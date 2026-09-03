//! Immutable, append-only audit log for all portfolio activities:
//! buy, sell, deposit, withdraw, and position recording.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

/// An immutable record of a portfolio transaction or balance adjustment.
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
    pub created_at: Option<DateTime<Utc>>,
}

const AUDIT_COLUMNS: &str = "id, owner_id, action, symbol, quantity_minor, price_minor, \
                             amount_minor, currency, currency_exponent, realized_pl_minor, \
                             note, created_at";

/// Records an immutable audit log entry for a portfolio operation.
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
) -> Result<PortfolioAuditEntry> {
    let result = sqlx::query(
        "INSERT INTO portfolio_audit_log \
         (owner_id, action, symbol, quantity_minor, price_minor, amount_minor, currency, \
          currency_exponent, realized_pl_minor, note, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)",
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
            &pool, owner, "deposit", None, None, None, 500000, "USD", 2, None, Some("wire deposit"),
        )
        .await
        .unwrap();
        assert_eq!(e1.action, "deposit");
        assert_eq!(e1.amount_minor, 500000);

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
        )
        .await
        .unwrap();
        assert_eq!(e2.action, "buy");
        assert_eq!(e2.symbol.as_deref(), Some("AAPL"));

        let list = list_audit_entries(&pool, owner, 10, 0).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, e2.id);
        assert_eq!(list[1].id, e1.id);
    }
}
