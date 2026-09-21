use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

use crate::utils::html::html_escape;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AlertCondition {
    Above,
    Below,
}

impl AlertCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertCondition::Above => "above",
            AlertCondition::Below => "below",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "above" | ">" | "gte" | ">=" => Some(AlertCondition::Above),
            "below" | "<" | "lte" | "<=" => Some(AlertCondition::Below),
            _ => None,
        }
    }

    pub fn symbol_str(&self) -> &'static str {
        match self {
            AlertCondition::Above => "≥",
            AlertCondition::Below => "≤",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct PriceAlert {
    pub id: i64,
    pub owner_id: String,
    pub symbol: String,
    pub target_price_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub condition: String,
    pub is_triggered: bool,
    pub triggered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl PriceAlert {
    pub fn parsed_condition(&self) -> AlertCondition {
        AlertCondition::from_str(&self.condition).unwrap_or(AlertCondition::Above)
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePriceAlert {
    pub symbol: String,
    pub target_price: String,
    pub currency: Option<String>,
    pub condition: AlertCondition,
}

pub struct AlertsRepository;

impl AlertsRepository {
    pub async fn create(
        pool: &DbPool,
        owner_id: &str,
        symbol: &str,
        target_price_minor: i64,
        currency: &str,
        currency_exponent: i64,
        condition: AlertCondition,
    ) -> Result<PriceAlert> {
        let norm_symbol = symbol.trim().to_uppercase();
        let norm_currency = currency.trim().to_uppercase();

        let id = crate::db_scalar_one!(
            pool,
            i64,
            r#"
            INSERT INTO portfolio_price_alerts (
                owner_id, symbol, target_price_minor, currency, currency_exponent, condition
            ) VALUES (?, ?, ?, ?, ?, ?)
            RETURNING id;
            "#,
            owner_id,
            &norm_symbol,
            target_price_minor,
            &norm_currency,
            currency_exponent,
            condition.as_str()
        )?;

        Self::find_by_id(pool, id, owner_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Alert created but not found"))
    }

    pub async fn find_by_id(pool: &DbPool, id: i64, owner_id: &str) -> Result<Option<PriceAlert>> {
        let alert = crate::db_fetch_optional!(
            pool,
            PriceAlert,
            r#"
            SELECT id, owner_id, symbol, target_price_minor, currency, currency_exponent,
                   condition, is_triggered, triggered_at, created_at
            FROM portfolio_price_alerts
            WHERE id = ? AND owner_id = ?;
            "#,
            id,
            owner_id
        )?;

        Ok(alert)
    }

    pub async fn list_for_owner(pool: &DbPool, owner_id: &str) -> Result<Vec<PriceAlert>> {
        let alerts = crate::db_fetch_all!(
            pool,
            PriceAlert,
            r#"
            SELECT id, owner_id, symbol, target_price_minor, currency, currency_exponent,
                   condition, is_triggered, triggered_at, created_at
            FROM portfolio_price_alerts
            WHERE owner_id = ?
            ORDER BY is_triggered ASC, created_at DESC;
            "#,
            owner_id
        )?;

        Ok(alerts)
    }

    pub async fn list_active_for_symbol(pool: &DbPool, symbol: &str) -> Result<Vec<PriceAlert>> {
        let norm = symbol.trim().to_uppercase();
        let alerts = crate::db_fetch_all!(
            pool,
            PriceAlert,
            r#"
            SELECT id, owner_id, symbol, target_price_minor, currency, currency_exponent,
                   condition, is_triggered, triggered_at, created_at
            FROM portfolio_price_alerts
            WHERE is_triggered = FALSE AND (symbol = ? OR symbol LIKE ?);
            "#,
            &norm,
            format!("{}-%", norm)
        )?;

        Ok(alerts)
    }

    pub async fn list_active_symbols(pool: &DbPool) -> Result<Vec<String>> {
        let symbols = crate::db_scalar_all!(
            pool,
            String,
            "SELECT DISTINCT symbol FROM portfolio_price_alerts WHERE is_triggered = FALSE;"
        )?;

        Ok(symbols)
    }

    pub async fn delete(pool: &DbPool, id: i64, owner_id: &str) -> Result<bool> {
        let rows = crate::db_execute!(
            pool,
            "DELETE FROM portfolio_price_alerts WHERE id = ? AND owner_id = ?;",
            id,
            owner_id
        )?;

        Ok(rows > 0)
    }

    pub async fn mark_triggered(pool: &DbPool, id: i64) -> Result<bool> {
        let rows = crate::db_execute!(pool, "UPDATE portfolio_price_alerts SET is_triggered = TRUE, triggered_at = CURRENT_TIMESTAMP WHERE id = ?;", id)?;

        Ok(rows > 0)
    }
}

/// Formats a clean, readable notification for a triggered alert (both plain and HTML).
pub fn format_alert_notification(alert: &PriceAlert, current_price_minor: i64) -> (String, String) {
    let cond = alert.parsed_condition();
    let exp = alert.currency_exponent as i32;
    let target = (alert.target_price_minor as f64) / 10f64.powi(exp);
    let current = (current_price_minor as f64) / 10f64.powi(exp);

    let (icon, direction_word) = match cond {
        AlertCondition::Above => ("🚀", "risen above"),
        AlertCondition::Below => ("⚠️", "dropped below"),
    };

    let diff_pct = if target > 0.0 {
        ((current - target) / target) * 100.0
    } else {
        0.0
    };

    let plain = format!(
        "{} Price Alert: {} has {} target {:.2} {}! Current price: {:.2} {} ({:+.2}%)",
        icon,
        alert.symbol,
        direction_word,
        target,
        alert.currency,
        current,
        alert.currency,
        diff_pct
    );

    let html = format!(
        r#"<div style="font-family:sans-serif;padding:8px 0;">
  <p><strong>{} Price Alert: {}</strong></p>
  <p>{} has <strong>{}</strong> your target of <strong>{:.2} {}</strong>.</p>
  <p>Current Market Price: <strong>{:.2} {}</strong> (<em>{:+.2}%</em>)</p>
</div>"#,
        icon,
        html_escape(&alert.symbol),
        html_escape(&alert.symbol),
        direction_word,
        target,
        html_escape(&alert.currency),
        current,
        html_escape(&alert.currency),
        diff_pct
    );

    (plain, html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn pool() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_create_list_delete_alerts() {
        let pool = pool().await;
        let owner = "alice";

        let alert = AlertsRepository::create(
            &pool,
            owner,
            "BTC-USD",
            70000_00,
            "USD",
            2,
            AlertCondition::Above,
        )
        .await
        .unwrap();

        assert_eq!(alert.symbol, "BTC-USD");
        assert_eq!(alert.target_price_minor, 70000_00);
        assert_eq!(alert.is_triggered, false);

        let list = AlertsRepository::list_for_owner(&pool, owner)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let symbols = AlertsRepository::list_active_symbols(&pool).await.unwrap();
        assert!(symbols.contains(&"BTC-USD".to_string()));

        let active_btc = AlertsRepository::list_active_for_symbol(&pool, "BTC-USD")
            .await
            .unwrap();
        assert_eq!(active_btc.len(), 1);

        AlertsRepository::mark_triggered(&pool, alert.id)
            .await
            .unwrap();
        let updated = AlertsRepository::find_by_id(&pool, alert.id, owner)
            .await
            .unwrap()
            .unwrap();
        assert!(updated.is_triggered);

        let deleted = AlertsRepository::delete(&pool, alert.id, owner)
            .await
            .unwrap();
        assert!(deleted);
        let empty = AlertsRepository::list_for_owner(&pool, owner)
            .await
            .unwrap();
        assert!(empty.is_empty());
    }
}
