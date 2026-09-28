use crate::db::DbPool;
use chrono::{Duration, Utc};

use super::protocol::{McpResource, ReadResourceResult, ResourceContent};
use crate::infrastructure::postgres::repositories::habits as habits_repo;
use crate::infrastructure::postgres::repositories::todos as todos_repo;
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::life::repository::LifeRepository;
use crate::modules::portfolio::cash_repo;
use crate::modules::portfolio::repository::PortfolioRepository;

/// Returns the standard read-only resources exposed by MatrixBot to MCP clients.
pub fn list_resources() -> Vec<McpResource> {
    vec![
        McpResource {
            uri: "matrixbot://todos/pending".to_string(),
            name: "Active Pending Todos".to_string(),
            description: Some("Current uncompleted to-do items and tasks".to_string()),
            mime_type: Some("application/json".to_string()),
        },
        McpResource {
            uri: "matrixbot://finance/recent".to_string(),
            name: "Recent Finance Transactions".to_string(),
            description: Some("Recent ledger transactions and expense entries".to_string()),
            mime_type: Some("application/json".to_string()),
        },
        McpResource {
            uri: "matrixbot://portfolio/summary".to_string(),
            name: "Investment Portfolio Summary".to_string(),
            description: Some("Current holdings, cash balances, and positions".to_string()),
            mime_type: Some("application/json".to_string()),
        },
        McpResource {
            uri: "matrixbot://habits/today".to_string(),
            name: "Today's Habits".to_string(),
            description: Some("Active daily habits, streaks, and checkoff states".to_string()),
            mime_type: Some("application/json".to_string()),
        },
        McpResource {
            uri: "matrixbot://life/profile".to_string(),
            name: "Life Horizon Profile".to_string(),
            description: Some("Memento Mori lifespan profile and progress".to_string()),
            mime_type: Some("application/json".to_string()),
        },
    ]
}

/// Reads a specific resource URI for an authenticated owner.
pub async fn read_resource(
    pool: &DbPool,
    owner_id: &str,
    uri: &str,
) -> Result<ReadResourceResult, String> {
    match uri {
        "matrixbot://todos/pending" => {
            let acct = crate::domain::accounts::AccountId::new(owner_id);
            let todos = todos_repo::list_all_user(pool, &acct, Some(false), 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            let json = serde_json::to_string_pretty(&todos).map_err(|e| e.to_string())?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(json),
                }],
            })
        }
        "matrixbot://finance/recent" => {
            let transactions = FinanceRepository::list(pool, owner_id, None, 20, 0)
                .await
                .map_err(|e| e.to_string())?;
            let now = Utc::now();
            let from = now - Duration::days(30);
            let summary = FinanceRepository::summary(pool, owner_id, from, now)
                .await
                .ok();
            let payload = serde_json::json!({
                "monthly_summary": summary,
                "recent_transactions": transactions,
            });
            let json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(json),
                }],
            })
        }
        "matrixbot://portfolio/summary" => {
            let holdings = PortfolioRepository::list(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            let cash = cash_repo::list_cash(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            let payload = serde_json::json!({
                "holdings": holdings,
                "cash_balances": cash,
            });
            let json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(json),
                }],
            })
        }
        "matrixbot://habits/today" => {
            let acct = crate::domain::accounts::AccountId::new(owner_id);
            let today_str = Utc::now().format("%Y-%m-%d").to_string();
            let habits = habits_repo::find_all_with_status(pool, &acct, &today_str)
                .await
                .map_err(|e| e.to_string())?;
            let json = serde_json::to_string_pretty(&habits).map_err(|e| e.to_string())?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(json),
                }],
            })
        }
        "matrixbot://life/profile" => {
            let profile = LifeRepository::get_profile(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            let json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContent {
                    uri: uri.to_string(),
                    mime_type: Some("application/json".to_string()),
                    text: Some(json),
                }],
            })
        }
        _ => Err(format!("Unknown resource URI: {}", uri)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_resources() {
        let res = list_resources();
        assert_eq!(res.len(), 5);
        assert!(res.iter().any(|r| r.uri == "matrixbot://todos/pending"));
    }
}
