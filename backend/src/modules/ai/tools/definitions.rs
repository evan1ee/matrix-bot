use serde_json::json;
use crate::modules::ai::models::{FunctionDefinition, ToolDefinition};

pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    vec![
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
                description: "Fetch financial summaries including expense and income totals broken down by category and currency.".to_string(),
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
                        "category": { "type": "string", "description": "Category name (e.g. Food, Transport, Rent)" }
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
    ]
}
