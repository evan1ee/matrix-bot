//! Tool sandboxing, whitelist enforcement, and sensitive action interception.

use crate::infrastructure::inference::llm::ToolDefinition;

/// Default tools classified as sensitive requiring explicit human confirmation.
pub const DEFAULT_SENSITIVE_TOOLS: &[&str] = &[
    // Funds & Trades
    "buy_portfolio_holding",
    "sell_portfolio_holding",
    "withdraw_portfolio_cash",
    "deposit_portfolio_cash",
    "pay_recurring_bill",
    // Irreversible deletions
    "delete_finance_entry",
    "delete_todo",
    "delete_recurring_bill",
    "delete_habit",
    "delete_bookmark",
    "delete_footprint",
    "delete_portfolio_price_alert",
    "cancel_portfolio_price_alert",
];

/// Checks if a tool name is allowed given an optional whitelist.
///
/// If `allowed_tools` is None or contains the wildcard `"*"`, all tools are permitted.
pub fn is_tool_allowed(tool_name: &str, allowed_tools: Option<&[String]>) -> bool {
    match allowed_tools {
        None => true,
        Some(tools) => tools.iter().any(|t| t == "*" || t == tool_name),
    }
}

/// Checks if a tool call requires human-in-the-loop approval before execution.
pub fn is_sensitive_tool(tool_name: &str, custom_sensitive: Option<&[String]>) -> bool {
    if DEFAULT_SENSITIVE_TOOLS.contains(&tool_name) {
        return true;
    }
    if let Some(custom) = custom_sensitive {
        return custom.iter().any(|t| t == tool_name);
    }
    false
}

/// Filters a list of tool definitions to only those permitted by the whitelist.
pub fn filter_tool_definitions(
    all_tools: &[ToolDefinition],
    allowed_tools: Option<&[String]>,
) -> Vec<ToolDefinition> {
    match allowed_tools {
        None => all_tools.to_vec(),
        Some(allowed) if allowed.iter().any(|t| t == "*") => all_tools.to_vec(),
        Some(allowed) => all_tools
            .iter()
            .filter(|t| allowed.contains(&t.function.name))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_allows_all_when_unspecified_or_wildcard() {
        assert!(is_tool_allowed("any_tool", None));
        assert!(is_tool_allowed("any_tool", Some(&["*".to_string()])));
    }

    #[test]
    fn whitelist_restricts_to_specified_tools() {
        let allowed = vec!["get_my_bookmarks".to_string(), "add_todo".to_string()];
        assert!(is_tool_allowed("get_my_bookmarks", Some(&allowed)));
        assert!(is_tool_allowed("add_todo", Some(&allowed)));
        assert!(!is_tool_allowed("buy_portfolio_holding", Some(&allowed)));
    }

    #[test]
    fn sensitive_tools_intercepted() {
        assert!(is_sensitive_tool("buy_portfolio_holding", None));
        assert!(is_sensitive_tool("withdraw_portfolio_cash", None));
        assert!(is_sensitive_tool("delete_todo", None));
        assert!(is_sensitive_tool("cancel_portfolio_price_alert", None));
        assert!(is_sensitive_tool("delete_portfolio_price_alert", None));
        assert!(!is_sensitive_tool("get_my_tasks", None));

        let custom = vec!["custom_risky_tool".to_string()];
        assert!(is_sensitive_tool("custom_risky_tool", Some(&custom)));
    }
}
