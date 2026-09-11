use serde_json::json;

use crate::modules::ai::models::{FunctionDefinition, ToolDefinition};

pub fn life_and_habits_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_habits".to_string(),
                description: "Fetch active habits, current streaks, target frequencies, and whether each habit is completed for today.".to_string(),
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
                name: "log_habit".to_string(),
                description: "Check off or toggle a habit's completion status for today by habit ID or name search (e.g. 'workout', 'reading').".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Habit numeric ID" },
                        "name_search": { "type": "string", "description": "Substring of habit name to match and check off" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "create_habit".to_string(),
                description: "Create a new habit to track with frequency and category.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Habit name, e.g. 'Morning Run' or 'Drink 2L Water'" },
                        "category": { "type": "string", "description": "Category e.g. 'health', 'fitness', 'productivity', 'mindfulness'" },
                        "frequency": { "type": "string", "enum": ["daily", "weekly"], "description": "Tracking frequency (default 'daily')" },
                        "target_days_per_week": { "type": "integer", "description": "Target days per week between 1 and 7 (default 7)" }
                    },
                    "required": ["name"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "log_life_day".to_string(),
                description: "Log today's (or a specified day's) life grid entry with mood, weather, activities, and personal notes.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "mood": { "type": "string", "enum": ["great", "good", "neutral", "bad", "terrible"], "description": "Daily mood indicator" },
                        "note": { "type": "string", "description": "Personal reflection or highlight of the day" },
                        "weather": { "type": "string", "description": "Weather descriptor, e.g. 'sunny', 'rainy', 'cloudy'" },
                        "activities": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Activities done today, e.g. ['coding', 'gym', 'reading']"
                        },
                        "date": { "type": "string", "description": "ISO date string YYYY-MM-DD (defaults to today)" }
                    },
                    "required": ["mood"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_life_profile".to_string(),
                description: "Fetch the user's life grid profile, lifespan expectations, days lived, and days remaining.".to_string(),
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
                name: "pay_recurring_bill".to_string(),
                description: "Mark a recurring bill as paid for the current cycle by ID or name search, advancing its cycle and automatically recording the expense in the finance ledger.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Bill numeric ID" },
                        "name_search": { "type": "string", "description": "Substring of bill name to match and pay (e.g. 'Netflix', 'Spotify', 'Rent')" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_footprints".to_string(),
                description: "Fetch recent travel footprints and visited locations with ratings and spend.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Maximum number of footprints to return (default 10)" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "record_footprint".to_string(),
                description: "Record a new travel footprint or visited location on the map.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Location or spot title, e.g. 'Blue Bottle Coffee'" },
                        "latitude": { "type": "number", "description": "Latitude coordinate" },
                        "longitude": { "type": "number", "description": "Longitude coordinate" },
                        "city": { "type": "string", "description": "City name" },
                        "country": { "type": "string", "description": "Country name" },
                        "arrival_date": { "type": "string", "description": "Arrival date in YYYY-MM-DD format (e.g. '2026-09-10'). Defaults to today" },
                        "departure_date": { "type": "string", "description": "Optional departure date in YYYY-MM-DD format" },
                        "category": { "type": "string", "description": "Category e.g. 'Cafe', 'Restaurant', 'Park'" },
                        "rating": { "type": "integer", "description": "Rating from 1 to 5" },
                        "spend_amount": { "type": "string", "description": "Amount spent, e.g. '12.50'" },
                        "currency": { "type": "string", "description": "3-letter currency code, e.g. USD" },
                        "note": { "type": "string", "description": "Personal note about the place" }
                    },
                    "required": ["title", "latitude", "longitude"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_weekly_retrospective".to_string(),
                description: "Fetch a comprehensive weekly executive retrospective aggregating finances, completed tasks, habit consistency, upcoming bills, and mood tracking. Currencies are isolated per-currency and must never be summed together without exchange rates.".to_string(),
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
                name: "save_bookmark".to_string(),
                description: "Save a web URL or article to reading list / bookmarks with tags.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The full HTTP or HTTPS URL to bookmark" },
                        "title": { "type": "string", "description": "Optional custom title override" },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of tags e.g. ['tech', 'ai']"
                        }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
            },
        },
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_my_bookmarks".to_string(),
                description: "Query saved bookmarks and reading list with optional filtering by tag or favorites.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "tag": { "type": "string", "description": "Optional tag filter" },
                        "favorite_only": { "type": "boolean", "description": "Filter by favorite/starred bookmarks" },
                        "limit": { "type": "integer", "description": "Number of bookmarks to return (default 5)" }
                    },
                    "required": [],
                    "additionalProperties": false
                }),
            },
        },
    ]
}
