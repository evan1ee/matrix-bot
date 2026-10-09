//! AI and MCP tool definitions and executors for the notes knowledge base.

use serde_json::json;

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::NoteFormat;
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &AccountId,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "knowledge_search" => Some(
            async {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'query' is required")?;
                let filter_types = args
                    .get("filter_types")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    });
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);

                let results = services
                    .knowledge_search
                    .search(owner_id, query, filter_types.as_deref(), limit)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&results).map_err(|e| e.to_string())
            }
            .await,
        ),

        "knowledge_create_note" => Some(
            async {
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'title' is required")?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'content' is required")?;
                let tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });

                let note = services
                    .notes
                    .create_note(
                        owner_id,
                        title,
                        content,
                        Some(NoteFormat::Markdown),
                        Some("ai"),
                        None,
                        tags.as_deref(),
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&note).map_err(|e| e.to_string())
            }
            .await,
        ),

        "knowledge_list_tags" => Some(
            async {
                let tags = services
                    .notes
                    .list_tags(owner_id)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&tags).map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "knowledge_search",
            "Perform hybrid vector semantic and keyword search across your notes, bookmarks, journal entries and footprints.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language query, question, or keyword to search for (required)"
                    },
                    "filter_types": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["note", "bookmark", "journal", "footprint"]
                        },
                        "description": "Optional list of item types to filter search results"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of ranked results to return (default 5)"
                    }
                },
                "required": ["query"]
            }),
        ),
        ToolDefinition::function(
            "knowledge_create_note",
            "Save an idea, thought or document as a note.",
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Title of the note (required, 1-100 characters)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Note content in Markdown format (required)"
                    },
                    "tags": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Optional tags to categorize this note"
                    }
                },
                "required": ["title", "content"]
            }),
        ),
        ToolDefinition::function(
            "knowledge_list_tags",
            "List all unique tags used across notes in the knowledge base.",
            json!({
                "type": "object",
                "properties": {}
            }),
        ),
    ]
}
