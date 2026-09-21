use crate::db::DbPool;
use serde_json::json;

use crate::events::EventBroadcaster;
use crate::modules::ai::models::ToolDefinition;
use crate::modules::portfolio::models::WithdrawOutcome;
use crate::modules::portfolio::service;
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_tool(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
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
                let default_cur = SettingsRepository::get_or_default(pool, owner_id)
                    .await
                    .currency;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur);

                let cash = service::deposit_cash(
                    pool,
                    broadcaster,
                    owner_id,
                    raw_currency,
                    &amount_str,
                    None,
                )
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
                let default_cur = SettingsRepository::get_or_default(pool, owner_id)
                    .await
                    .currency;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur);
                // Recomputed here only to word the insufficient-funds message
                // below — by the time it is reached, `service::withdraw_cash`
                // has already accepted this currency as well-formed.
                let currency = crate::modules::finance::money::normalise_currency(raw_currency)
                    .map_err(|e| e.to_string())?;

                let outcome = service::withdraw_cash(
                    pool,
                    broadcaster,
                    owner_id,
                    raw_currency,
                    &amount_str,
                    None,
                )
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
            "Deposit uninvested cash into the user's portfolio cash balance.",
            json!({
                "type": "object",
                "properties": {
                    "amount": {
                        "type": "number",
                        "description": "Cash amount to deposit as a positive decimal (required)"
                    },
                    "currency": {
                        "type": "string",
                        "description": "3-letter currency code (e.g. USD, EUR, TWD)"
                    }
                },
                "required": ["amount"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "withdraw_portfolio_cash",
            "Withdraw cash from the user's available portfolio cash balance.",
            json!({
                "type": "object",
                "properties": {
                    "amount": {
                        "type": "number",
                        "description": "Cash amount to withdraw as a positive decimal (required)"
                    },
                    "currency": {
                        "type": "string",
                        "description": "3-letter currency code"
                    }
                },
                "required": ["amount"],
                "additionalProperties": false
            }),
        ),
    ]
}
