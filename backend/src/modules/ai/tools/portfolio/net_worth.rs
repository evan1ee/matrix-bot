use crate::db::DbPool;
use serde_json::json;

use crate::infrastructure::inference::llm::ToolDefinition;
use crate::infrastructure::postgres::read_models::net_worth::compute_net_worth;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    pool: &DbPool,
    owner_id: &str,
    name: &str,
    _args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_net_worth" => Some(
            async {
                let acct = crate::domain::accounts::AccountId::from(owner_id);
                let settings = services.settings.get_or_default(&acct).await;
                let client = reqwest::Client::new();
                let report = compute_net_worth(pool, &client, &acct, &settings.currency)
                    .await
                    .map_err(|e| format!("Failed to compute net worth: {}", e))?;
                serde_json::to_string(&report).map_err(|e| e.to_string())
            }
            .await,
        ),
        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::function(
        "get_my_net_worth",
        "Calculate and return the user's unified cross-currency estimated net worth, aggregating all-time ledger cash balances, uninvested portfolio cash deposits, and investment holdings market value converted into their primary currency via daily ECB reference FX rates.",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_net_worth_tool_definition() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].function.name, "get_my_net_worth");
    }
}
