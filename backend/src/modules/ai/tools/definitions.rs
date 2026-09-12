use super::definitions_extended::extended_tool_definitions;
use super::definitions_life::life_and_habits_tool_definitions;
use crate::modules::ai::models::{FunctionDefinition, ToolDefinition};
use serde_json::json;

pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    let mut tools = vec![
        // --- Read / Query Tools ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_tasks".to_string(),
                description: "Fetch tasks and to-dos owned by the user, including status, priority, and due dates.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "completed": {
                            "type": "boolean",
                            "description": "Optional filter: true for completed tasks, false for active tasks"
                        }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_reminders".to_string(),
                description: "Fetch upcoming pending reminders scheduled for the user.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_recurring_bills".to_string(),
                description: "Fetch recurring bills and subscriptions, including amounts, recurrence patterns, and next due dates.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_financial_summary".to_string(),
                description: "Fetch financial summaries including expense and income totals broken down by category and currency. Currencies are separate and must never be summed together without exchange rates.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_portfolio_summary".to_string(),
                description: "Fetch investment portfolio holdings, valuations, uninvested cash balances, and unrealized P&L.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_settings".to_string(),
                description: "Fetch the user's configured timezone and default currency preference.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        // --- Write / Action Tools ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "add_todo".to_string(),
                description: "Add a new to-do / task to the user's list.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "The task description" },
                        "priority": { "type": "string", "enum": ["low", "medium", "high"], "description": "Task priority level" },
                        "due_in_minutes": { "type": "integer", "description": "Relative due time offset in minutes from now" },
                        "due_at": { "type": "string", "description": "Absolute ISO 8601 due timestamp" },
                        "remind_before_minutes": { "type": "integer", "description": "Minutes before due_at to trigger a reminder" }
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "complete_todo".to_string(),
                description: "Mark a task / to-do as completed by ID or description search.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Task numeric ID" },
                        "text_search": { "type": "string", "description": "Substring of task text to search and complete" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "record_finance_entry".to_string(),
                description: "Record an expense or income transaction in the financial ledger.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "amount": { "type": "string", "description": "Transaction amount, e.g. '15.50'" },
                        "direction": { "type": "string", "enum": ["expense", "income"], "description": "Whether money went out (expense) or came in (income)" },
                        "note": { "type": "string", "description": "Description of the expense or income" },
                        "currency": { "type": "string", "description": "3-letter currency code (e.g. USD, EUR, CNY). Defaults to user currency" },
                        "category": { "type": "string", "description": "Category name (e.g. Food, Transport, Rent)" },
                        "date": { "type": "string", "description": "Optional date of transaction in YYYY-MM-DD or ISO 8601 format (e.g. '2026-09-10'). Defaults to current time" }
                    },
                    "required": ["amount", "direction", "note"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "set_reminder".to_string(),
                description: "Schedule a reminder notification.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "Reminder text" },
                        "in_minutes": { "type": "integer", "description": "Relative delay in minutes from now" },
                        "trigger_at": { "type": "string", "description": "Absolute ISO 8601 trigger timestamp" },
                        "recurrence": { "type": "string", "enum": ["none", "daily", "weekly", "monthly", "yearly"], "description": "Recurrence pattern" }
                    },
                    "required": ["message"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "deposit_portfolio_cash".to_string(),
                description: "Deposit uninvested cash into the investment portfolio.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "amount": { "type": "string", "description": "Amount of cash to deposit, e.g. '500.00'" },
                        "currency": { "type": "string", "description": "3-letter currency code. Defaults to user currency" }
                    },
                    "required": ["amount"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "buy_portfolio_holding".to_string(),
                description: "Record purchasing an investment position (stocks, crypto, ETFs). Requires sufficient portfolio cash.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Ticker or symbol, e.g. AAPL, BTC" },
                        "quantity": { "type": "string", "description": "Quantity purchased, e.g. '10' or '0.5'" },
                        "price": { "type": "string", "description": "Price paid per unit, e.g. '150.00'" },
                        "currency": { "type": "string", "description": "3-letter currency code. Defaults to user currency" }
                    },
                    "required": ["symbol", "quantity", "price"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_portfolio_holding".to_string(),
                description: "Fetch details of a specific investment holding (quantity, cost basis, current market price, total value, and unrealized profit/loss).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Ticker or symbol of the holding, e.g. AAPL, BTC, NVDA" }
                    },
                    "required": ["symbol"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "sell_portfolio_holding".to_string(),
                description: "Sell a position or part of a position from the portfolio. Gross proceeds are credited to cash balance and realized gain/loss is calculated.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Ticker or symbol of the holding to sell, e.g. AAPL, BTC" },
                        "quantity": { "type": "string", "description": "Quantity to sell, e.g. '5' or '0.25'" },
                        "price": { "type": "string", "description": "Sale price per unit, e.g. '175.50'" }
                    },
                    "required": ["symbol", "quantity", "price"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "withdraw_portfolio_cash".to_string(),
                description: "Withdraw uninvested cash from the investment portfolio.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "amount": { "type": "string", "description": "Amount of cash to withdraw, e.g. '250.00'" },
                        "currency": { "type": "string", "description": "3-letter currency code. Defaults to user currency" }
                    },
                    "required": ["amount"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "create_portfolio_price_alert".to_string(),
                description: "Create a real-time price alert that sends a notification to the user's Matrix chat when a market price target is crossed.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Ticker or symbol, e.g. BTC-USD, AAPL" },
                        "condition": { "type": "string", "enum": ["gte", "lte", "above", "below"], "description": "'gte' / 'above' if price rises to or above target; 'lte' / 'below' if price falls to or below target" },
                        "target_price": { "type": "string", "description": "Target price trigger, e.g. '70000.00' or '120.50'" }
                    },
                    "required": ["symbol", "target_price"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_portfolio_price_alerts".to_string(),
                description: "Fetch all active and past market price alerts configured for the user.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "active_only": { "type": "boolean", "description": "Optional: set true to only return pending, untriggered alerts" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "cancel_portfolio_price_alert".to_string(),
                description: "Cancel a market price alert by alert ID or symbol.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Alert numeric ID" },
                        "symbol": { "type": "string", "description": "Ticker symbol to cancel active alerts for" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_portfolio_performance_history".to_string(),
                description: "Fetch historical portfolio net worth snapshots, period percentage returns, and value changes over a given timeframe.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "timeframe": { "type": "string", "enum": ["7d", "1m", "3m", "1y", "all"], "description": "History timeframe period (default '1m')" },
                        "currency": { "type": "string", "description": "3-letter currency code. Defaults to user currency" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "list_finance_transactions".to_string(),
                description: "List recent expense and income transactions from the financial ledger with optional category, direction, or time filters.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "direction": { "type": "string", "enum": ["expense", "income"], "description": "Filter by 'expense' or 'income'" },
                        "category": { "type": "string", "description": "Filter by category name, e.g. 'Food', 'Rent'" },
                        "days": { "type": "integer", "description": "Only include transactions from the last N days (default 365)" },
                        "limit": { "type": "integer", "description": "Maximum number of transactions to return (default 10, max 50)" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_spending_by_category".to_string(),
                description: "Analyze expense spending by category over a given timeframe, showing amounts and percentage shares.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "days": { "type": "integer", "description": "Lookback period in days (e.g. 7, 30, 90; default 30)" },
                        "currency": { "type": "string", "description": "3-letter currency code (e.g. USD, EUR). Defaults to user currency" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "delete_finance_entry".to_string(),
                description: "Delete an expense or income transaction from the financial ledger by transaction ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Transaction ID to delete" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },
    ];
    tools.extend(life_and_habits_tool_definitions());
    tools.extend(extended_tool_definitions());
    tools
}
