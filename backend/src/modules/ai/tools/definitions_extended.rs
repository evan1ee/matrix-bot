use crate::modules::ai::models::{FunctionDefinition, ToolDefinition};
use serde_json::json;

pub fn extended_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // --- Todos: Update & Delete ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "update_todo".to_string(),
                description: "Update an existing task / to-do's text, priority, or due date by task ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Task numeric ID" },
                        "text": { "type": "string", "description": "New task text description" },
                        "priority": { "type": "string", "enum": ["low", "medium", "high"], "description": "New priority level" },
                        "due_in_minutes": { "type": "integer", "description": "Relative due time in minutes from now" },
                        "due_at": { "type": "string", "description": "Absolute ISO 8601 due timestamp" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "delete_todo".to_string(),
                description: "Permanently delete a task / to-do by task ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Task numeric ID to delete" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Reminders: Cancel ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "cancel_reminder".to_string(),
                description: "Cancel a pending scheduled reminder by reminder ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Reminder numeric ID to cancel" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Recurring Bills: Create & Delete ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "create_recurring_bill".to_string(),
                description: "Create a new recurring subscription or bill (e.g. Netflix, Rent, Spotify).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Name of the bill or subscription" },
                        "amount": { "type": "string", "description": "Amount, e.g. '14.99' or '1200.00'" },
                        "currency": { "type": "string", "description": "3-letter currency code (e.g. USD, EUR, CNY)" },
                        "category": { "type": "string", "description": "Category name, e.g. 'Subscriptions', 'Housing'" },
                        "frequency": { "type": "string", "enum": ["weekly", "monthly", "yearly"], "description": "Recurrence frequency (default 'monthly')" },
                        "due_day": { "type": "integer", "description": "Day of the month due (1-31)" },
                        "auto_pay": { "type": "boolean", "description": "Whether this bill auto-pays" },
                        "notes": { "type": "string", "description": "Optional notes or details" }
                    },
                    "required": ["name", "amount"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "delete_recurring_bill".to_string(),
                description: "Delete a recurring bill or subscription by bill ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Bill numeric ID to delete" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Habits: Delete ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "delete_habit".to_string(),
                description: "Delete a habit and its tracking history by habit ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Habit numeric ID to delete" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Bookmarks: Actions & Deletion ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "toggle_bookmark_favorite".to_string(),
                description: "Star or unstar a saved bookmark by bookmark ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Bookmark numeric ID" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "archive_bookmark".to_string(),
                description: "Archive or unarchive a bookmark from the unread reading list.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Bookmark numeric ID" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "delete_bookmark".to_string(),
                description: "Permanently delete a saved bookmark by bookmark ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Bookmark numeric ID to delete" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Footprints: Delete ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "delete_footprint".to_string(),
                description: "Delete a travel footprint / visited location by footprint ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Footprint numeric ID to delete" }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Daily Log / Journal: Complete Day Status ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_daily_log".to_string(),
                description: "Fetch complete day log for a specific date (YYYY-MM-DD), including user mood, weather, activities, personal reflections, and all cross-module activity events (tasks done, expenses, habits check-ins, places visited).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "date": { "type": "string", "description": "ISO date string YYYY-MM-DD (defaults to today)" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },

        // --- Webhooks: Query & Logs ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_webhooks".to_string(),
                description: "List all configured inbound webhooks, their destination rooms, and delivery statistics.".to_string(),
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
                name: "get_webhook_logs".to_string(),
                description: "Fetch recent delivery logs and incoming payloads for a specific webhook ID.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "webhook_id": { "type": "string", "description": "Webhook ID string (e.g. 'whk_...')" },
                        "limit": { "type": "integer", "description": "Maximum number of logs to return (default 10)" }
                    },
                    "required": ["webhook_id"],
                    "additionalProperties": false
                }),
            },
        },

        // --- Settings: Update ---
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "update_my_settings".to_string(),
                description: "Update the user's preferred timezone (e.g. 'America/New_York') or default currency (e.g. 'USD', 'CNY', 'EUR').".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "timezone": { "type": "string", "description": "IANA timezone string, e.g. 'America/Los_Angeles', 'Europe/London', 'Asia/Shanghai'" },
                        "currency": { "type": "string", "description": "3-letter uppercase currency code, e.g. 'USD', 'CNY', 'EUR'" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
    ]
}
