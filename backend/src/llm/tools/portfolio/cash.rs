use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use serde_json::json;

use crate::domain::portfolio::models::WithdrawOutcome;
use crate::events::EventBroadcaster;
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    _pool: &DbPool,
    _broadcaster: Option<&EventBroadcaster>,
    owner_id: &AccountId,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "deposit_portfolio_cash" => Some(
            async {
                let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                    format!("{:.2}", num)
                } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'amount' is required".to_string());
                };
                let acct = *owner_id;
                let default_cur = services.settings.get_or_default(&acct).await.currency;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur);
                let currency = crate::domain::finance::money::normalise_currency(raw_currency)
                    .map_err(|e| e.to_string())?;
                let exponent = crate::domain::finance::money::exponent_for(&currency);
                let amount_minor =
                    crate::domain::finance::money::parse_amount(&amount_str, exponent)
                        .map_err(|e| e.to_string())?;

                let cash = services
                    .portfolio
                    .deposit_cash(&acct, &currency, exponent as i64, amount_minor, None)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&cash).map_err(|e| e.to_string())
            }
            .await,
        ),

        "withdraw_portfolio_cash" => Some(
            async {
                let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                    format!("{:.2}", num)
                } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'amount' is required".to_string());
                };
                let acct = *owner_id;
                let default_cur = services.settings.get_or_default(&acct).await.currency;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur);
                let currency = crate::domain::finance::money::normalise_currency(raw_currency)
                    .map_err(|e| e.to_string())?;
                let exponent = crate::domain::finance::money::exponent_for(&currency);
                let amount_minor =
                    crate::domain::finance::money::parse_amount(&amount_str, exponent)
                        .map_err(|e| e.to_string())?;

                let outcome = services
                    .portfolio
                    .withdraw_cash(&acct, &currency, exponent as i64, amount_minor, None)
                    .await
                    .map_err(|e| e.to_string())?;

                match outcome {
                    WithdrawOutcome::Ok(cash) => {
                        serde_json::to_string(&cash).map_err(|e| e.to_string())
                    }
                    WithdrawOutcome::InsufficientFunds { available_minor } => Err(format!(
                        "Insufficient cash in {}: only {} minor units available",
                        currency, available_minor
                    )),
                }
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "deposit_portfolio_cash",
            "Deposit uninvested liquid cash into the portfolio ledger, allowing future stock or crypto purchases. If currency is omitted, uses user's default settings currency.",
            json!({
                "type": "object",
                "properties": {
                    "amount": { "type": "string", "description": "Decimal cash amount to deposit (e.g. '1000' or '250.50')" },
                    "currency": { "type": "string", "description": "Optional ISO-4217 currency code (e.g. 'USD', 'EUR'). Defaults to user default." }
                },
                "required": ["amount"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "withdraw_portfolio_cash",
            "Withdraw uninvested liquid cash from the portfolio balance. Cannot withdraw more cash than available.",
            json!({
                "type": "object",
                "properties": {
                    "amount": { "type": "string", "description": "Decimal cash amount to withdraw (e.g. '500.00')" },
                    "currency": { "type": "string", "description": "Optional ISO-4217 currency code (e.g. 'USD', 'EUR'). Defaults to user default." }
                },
                "required": ["amount"],
                "additionalProperties": false
            }),
        ),
    ]
}
