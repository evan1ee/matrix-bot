//! AI/MCP tools for monthly budgets. The model is an untrusted caller: the owner comes
//! only from `execute_tool`'s parameter, every argument is validated strictly, and a
//! malformed call returns `Err` before anything is written.

use serde_json::{json, Value};

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::{Budget, BudgetState, BudgetStatus, Month, NewBudget};
use crate::domain::finance::money::{
    exponent_for, format_amount, normalise_currency, parse_amount,
};
use crate::infrastructure::inference::llm::ToolDefinition;

/// Same cap the chat and REST interfaces apply to free-text input.
const MAX_ARG_LEN: usize = 500;

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &AccountId,
    name: &str,
    args: &Value,
) -> Option<Result<String, String>> {
    match name {
        "get_budget_status" => Some(get_status(services, owner_id, args).await),
        "set_budget" => Some(set(services, owner_id, args).await),
        "remove_budget" => Some(remove(services, owner_id, args).await),
        _ => None,
    }
}

/// Rejects any argument the schema does not declare.
fn check_keys(args: &Value, allowed: &[&str]) -> Result<(), String> {
    let obj = args.as_object().ok_or("Arguments must be a JSON object")?;
    match obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("Unknown argument '{k}'")),
        None => Ok(()),
    }
}

/// Absent or null is `None`; anything but a string of at most 500 characters is an error.
fn opt_str(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.chars().count() > MAX_ARG_LEN => {
            Err(format!("'{key}' must be at most {MAX_ARG_LEN} characters"))
        }
        Some(Value::String(s)) => Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty())),
        Some(_) => Err(format!("'{key}' must be a string")),
    }
}

fn opt_bool(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("'{key}' must be a boolean")),
    }
}

fn money(minor: i64, exponent: i64) -> String {
    format_amount(minor, exponent as u32)
}

fn budget_json(b: &Budget) -> Value {
    json!({
        "scope": b.scope.label(),
        "currency": b.currency,
        "limit": money(b.limit_minor, b.currency_exponent),
        "alerts_enabled": b.alerts_enabled,
    })
}

fn status_json(s: &BudgetStatus) -> Value {
    let b = &s.budget;
    let mut v = json!({
        "scope": b.scope.label(),
        "currency": b.currency,
        "spent": money(s.spent_minor, b.currency_exponent),
        "limit": money(b.limit_minor, b.currency_exponent),
        "remaining": money(s.remaining_minor, b.currency_exponent),
        "percent_used": s.percent_used,
        "state": s.state,
        "alerts_enabled": b.alerts_enabled,
    });
    if let Some(p) = &s.pace {
        v["expected_by_today"] = json!(money(p.expected_minor, b.currency_exponent));
        v["projected_month_end"] = json!(money(p.projected_minor, b.currency_exponent));
    }
    v
}

/// Budgets that need attention after a recorded expense (non-Ok only).
pub fn attention_json(statuses: &[BudgetStatus]) -> Vec<Value> {
    statuses
        .iter()
        .filter(|s| s.state != BudgetState::Ok)
        .map(|s| {
            let b = &s.budget;
            json!({
                "scope": b.scope.label(),
                "currency": b.currency,
                "spent": money(s.spent_minor, b.currency_exponent),
                "limit": money(b.limit_minor, b.currency_exponent),
                "percent_used": s.percent_used,
                "state": s.state,
            })
        })
        .collect()
}

async fn get_status(s: &AppServices, owner: &AccountId, args: &Value) -> Result<String, String> {
    check_keys(args, &["month"])?;
    let month = opt_str(args, "month")?
        .map(|m| Month::parse(&m).map_err(|e| e.to_string()))
        .transpose()?;
    let list = s
        .finance
        .budget_status(owner, month)
        .await
        .map_err(|e| e.to_string())?;
    let month_label = list.first().map(|x| x.month.to_string());
    Ok(json!({
        "month": month_label,
        "budgets": list.iter().map(status_json).collect::<Vec<_>>(),
    })
    .to_string())
}

async fn set(s: &AppServices, owner: &AccountId, args: &Value) -> Result<String, String> {
    check_keys(args, &["amount", "currency", "category", "alerts_enabled"])?;
    let amount = match args.get("amount") {
        Some(Value::String(a)) if !a.trim().is_empty() => a.trim().to_string(),
        Some(Value::String(_)) | None | Some(Value::Null) => {
            return Err("Field 'amount' is required".to_string())
        }
        Some(_) => {
            return Err("Field 'amount' must be a string such as \"300.50\", not a number".into())
        }
    };
    if amount.chars().count() > MAX_ARG_LEN {
        return Err(format!("'amount' must be at most {MAX_ARG_LEN} characters"));
    }
    let category = opt_str(args, "category")?;
    let alerts = opt_bool(args, "alerts_enabled")?;
    let currency = match opt_str(args, "currency")? {
        Some(c) => c,
        None => s.settings.get_or_default(owner).await.currency,
    };
    let currency = normalise_currency(&currency).map_err(|e| e.to_string())?;
    let limit = parse_amount(&amount, exponent_for(&currency)).map_err(|e| e.to_string())?;
    let new =
        NewBudget::new(category.as_deref(), &currency, limit, alerts).map_err(|e| e.to_string())?;
    let (budget, created) = s
        .finance
        .set_budget(owner, new)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "created": created, "budget": budget_json(&budget) }).to_string())
}

async fn remove(s: &AppServices, owner: &AccountId, args: &Value) -> Result<String, String> {
    check_keys(args, &["category", "currency"])?;
    let category = opt_str(args, "category")?;
    let currency = opt_str(args, "currency")?;
    let budget = s
        .finance
        .resolve_budget(owner, category.as_deref(), currency.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    s.finance
        .delete_budget(owner, budget.id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "removed": budget_json(&budget) }).to_string())
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "get_budget_status",
            "Show monthly spending budgets measured against a month's expenses: spent, limit, percent used, state (ok, warning at 80%, exceeded at 100%) and pace. Defaults to the current month.",
            json!({
                "type": "object",
                "properties": {
                    "month": {
                        "type": "string",
                        "pattern": "^\\d{4}-\\d{2}$",
                        "description": "Optional month in YYYY-MM format. Defaults to the current month."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "set_budget",
            "Create or update a monthly spending limit, overall or for one existing category, in one currency. Updating keeps the alert setting unless alerts_enabled is given.",
            json!({
                "type": "object",
                "properties": {
                    "amount": {
                        "type": "string",
                        "description": "Monthly limit as a decimal string, e.g. \"300.50\". Must be greater than zero."
                    },
                    "currency": {
                        "type": "string",
                        "description": "ISO currency code. Defaults to the user's currency."
                    },
                    "category": {
                        "type": "string",
                        "description": "Existing category name. Omit for an overall budget."
                    },
                    "alerts_enabled": {
                        "type": "boolean",
                        "description": "Whether to send alerts at 80% and 100%. New budgets default to on."
                    }
                },
                "required": ["amount"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "remove_budget",
            "Remove exactly one budget (destructive). Omit category for the overall budget. If several currencies exist for the scope, the currency is required.",
            json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "description": "Category name. Omit for the overall budget."
                    },
                    "currency": {
                        "type": "string",
                        "description": "ISO currency code; required when the scope has budgets in several currencies."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppServices;
    use serde_json::json;

    async fn setup() -> (crate::db::DbPool, AppServices, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a", "@bob:b"]).await;
        let services = AppServices::new(&pool);
        (pool, services, AccountId::from_seed("@alice:a"))
    }

    async fn call(
        s: &AppServices,
        owner: &AccountId,
        name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        execute_tool(s, owner, name, &args)
            .await
            .expect("tool must be handled")
    }

    async fn record(
        pool: &crate::db::DbPool,
        s: &AppServices,
        owner: &AccountId,
        args: serde_json::Value,
    ) -> String {
        super::super::finance::execute_tool(s, pool, owner, "!r", "record_finance_entry", &args)
            .await
            .expect("handled")
            .expect("recorded")
    }

    #[tokio::test]
    async fn set_budget_tool_creates_budget_for_owner() {
        let (_pool, s, alice) = setup().await;
        let out = call(
            &s,
            &alice,
            "set_budget",
            json!({"amount": "300.50", "currency": "eur"}),
        )
        .await
        .unwrap();
        assert!(out.contains("300.50") && out.contains("EUR"), "{out}");
        let list = s.finance.list_budgets(&alice).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].limit_minor, 30050);
        let bob = AccountId::from_seed("@bob:b");
        assert!(s.finance.list_budgets(&bob).await.unwrap().is_empty());
        // The owner can never come from the arguments.
        let err = call(
            &s,
            &alice,
            "set_budget",
            json!({"amount": "1", "owner_id": "x"}),
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn set_budget_tool_rejects_numeric_amount_type_and_bad_currency() {
        let (_pool, s, alice) = setup().await;
        for args in [
            json!({"amount": 300}),
            json!({"amount": "300", "currency": "EURO"}),
            json!({"amount": "300", "currency": 5}),
            json!({"amount": "0"}),
            json!({"amount": "abc"}),
            json!({"amount": "300", "alerts_enabled": "yes"}),
            json!({"amount": "300", "category": "x".repeat(501)}),
            json!({"amount": "300", "category": "nonexistent"}),
            json!({}),
        ] {
            assert!(
                call(&s, &alice, "set_budget", args.clone()).await.is_err(),
                "{args}"
            );
        }
        assert!(s.finance.list_budgets(&alice).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_budget_tool_ambiguous_is_error() {
        let (_pool, s, alice) = setup().await;
        for cur in ["EUR", "USD"] {
            call(
                &s,
                &alice,
                "set_budget",
                json!({"amount": "100", "currency": cur}),
            )
            .await
            .unwrap();
        }
        let err = call(&s, &alice, "remove_budget", json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("currency"), "{err}");
        assert_eq!(s.finance.list_budgets(&alice).await.unwrap().len(), 2);
        call(&s, &alice, "remove_budget", json!({"currency": "usd"}))
            .await
            .unwrap();
        let left = s.finance.list_budgets(&alice).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].currency, "EUR");
    }

    #[tokio::test]
    async fn get_budget_status_tool_bad_month_is_error() {
        let (_pool, s, alice) = setup().await;
        for m in ["2026-13", "2026-1", "garbage", "2026-10-01"] {
            assert!(
                call(&s, &alice, "get_budget_status", json!({"month": m}))
                    .await
                    .is_err(),
                "{m}"
            );
        }
        assert!(call(&s, &alice, "get_budget_status", json!({"month": 5}))
            .await
            .is_err());
        let ok = call(&s, &alice, "get_budget_status", json!({}))
            .await
            .unwrap();
        assert!(ok.contains("budgets"), "{ok}");
    }

    #[tokio::test]
    async fn record_finance_entry_reports_budgets() {
        let (pool, s, alice) = setup().await;
        let plain = record(&pool, &s, &alice, json!({"amount": "5", "currency": "EUR"})).await;
        assert!(!plain.contains("\"budgets\""), "{plain}");
        call(
            &s,
            &alice,
            "set_budget",
            json!({"amount": "100", "currency": "EUR"}),
        )
        .await
        .unwrap();
        let out = record(
            &pool,
            &s,
            &alice,
            json!({"amount": "90", "currency": "EUR"}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["budgets"][0]["state"], "warning", "{out}");
        assert_eq!(v["budgets"][0]["scope"], "Overall");
    }

    #[test]
    fn all_tool_definitions_contain_budget_tools_once() {
        let defs = crate::llm::tools::definitions::all_tool_definitions();
        for name in ["get_budget_status", "set_budget", "remove_budget"] {
            assert_eq!(
                defs.iter().filter(|d| d.function.name == name).count(),
                1,
                "{name}"
            );
        }
        let set = defs
            .iter()
            .find(|d| d.function.name == "set_budget")
            .unwrap();
        assert_eq!(set.function.parameters["additionalProperties"], false);
        assert_eq!(set.function.parameters["required"], json!(["amount"]));
    }
}
