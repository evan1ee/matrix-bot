use crate::db::DbPool;
use serde_json::json;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::bookmarks::metadata::fetch_and_extract_metadata;
use crate::modules::bookmarks::models::BookmarkQuery;
use crate::modules::bookmarks::repository::BookmarksRepository;
use crate::services::llm::ToolDefinition;

pub async fn execute_tool(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "save_bookmark" => Some(
            async {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'url' is required")?;
                let custom_title = args.get("title").and_then(|v| v.as_str());
                let tags = args
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();

                let meta = fetch_and_extract_metadata(None, url, custom_title).await;

                let b = BookmarksRepository::create(
                    pool,
                    owner_id,
                    url,
                    &meta.title,
                    meta.description.as_deref(),
                    meta.author.as_deref(),
                    meta.image_url.as_deref(),
                    &meta.domain,
                    meta.reader_content.as_deref(),
                    meta.reading_time_minutes,
                    &tags,
                )
                .await
                .map_err(|e| e.to_string())?;

                if let Some(bc) = broadcaster {
                    bc.send(SystemEvent::BookmarkCreated(b.clone()));
                }

                serde_json::to_string(&b).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_my_bookmarks" => Some(
            async {
                let tag = args
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let fav = args.get("favorite_only").and_then(|v| v.as_bool());
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5);

                let query = BookmarkQuery {
                    tag,
                    favorite: fav,
                    archived: Some(false),
                    limit: Some(limit),
                    ..Default::default()
                };

                let list = BookmarksRepository::list(pool, owner_id, &query)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&list).map_err(|e| e.to_string())
            }
            .await,
        ),

        "toggle_bookmark_favorite" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let updated = BookmarksRepository::toggle_favorite(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::BookmarkUpdated(updated.clone()));
                }

                serde_json::to_string(&updated).map_err(|e| e.to_string())
            }
            .await,
        ),

        "archive_bookmark" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let updated = BookmarksRepository::toggle_archived(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::BookmarkUpdated(updated.clone()));
                }

                serde_json::to_string(&updated).map_err(|e| e.to_string())
            }
            .await,
        ),

        "delete_bookmark" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let ok = BookmarksRepository::delete(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;

                if !ok {
                    return Err(format!("Bookmark #{} not found or not owned by user", id));
                }

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::BookmarkDeleted {
                        id,
                        owner_id: owner_id.to_string(),
                    });
                }

                serde_json::to_string(&json!({
                    "id": id,
                    "deleted": true
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "save_bookmark",
            "Save and bookmark a URL with automated metadata title extraction and tags.",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to bookmark (required)"
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional title for the bookmark"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional notes or summary of the bookmarked link"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags (e.g. rust, ai, reading)"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_my_bookmarks",
            "Fetch saved bookmarks with optional tag filtering.",
            json!({
                "type": "object",
                "properties": {
                    "tag": {
                        "type": "string",
                        "description": "Optional tag filter"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of bookmarks to return (default 20, max 100)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "toggle_bookmark_favorite",
            "Toggle favorite / star status on a bookmark by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the bookmark to favorite/unfavorite (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "archive_bookmark",
            "Toggle archive status on a bookmark by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the bookmark to archive (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "delete_bookmark",
            "Permanently delete a saved bookmark by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the bookmark to delete (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
    ]
}
