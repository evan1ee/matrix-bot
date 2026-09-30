use serde_json::json;

use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::bookmarks::BookmarkQuery;
use crate::infrastructure::inference::llm::ToolDefinition;
use crate::infrastructure::web::fetch_and_extract_metadata;

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &AccountId,
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
                let tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });

                let meta = fetch_and_extract_metadata(None, url, custom_title).await;
                let b = services
                    .bookmarks
                    .create(owner_id, url, custom_title, tags.as_deref(), meta)
                    .await
                    .map_err(|e| e.to_string())?;

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

                let list = services
                    .bookmarks
                    .list(owner_id, &query)
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
                let updated = services
                    .bookmarks
                    .toggle_favorite(owner_id, id)
                    .await
                    .map_err(|e| match e {
                        AppError::NotFound(_) => format!("Bookmark #{id} not found"),
                        other => other.to_string(),
                    })?;

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
                let updated = services
                    .bookmarks
                    .toggle_archived(owner_id, id)
                    .await
                    .map_err(|e| match e {
                        AppError::NotFound(_) => format!("Bookmark #{id} not found"),
                        other => other.to_string(),
                    })?;

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
                services
                    .bookmarks
                    .delete(owner_id, id)
                    .await
                    .map_err(|e| match e {
                        AppError::NotFound(_) => {
                            format!("Bookmark #{} not found or not owned by user", id)
                        }
                        other => other.to_string(),
                    })?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppServices;
    use crate::domain::bookmarks::ExtractedMetadata;
    use crate::infrastructure::postgres::repositories;

    /// Given a seeded bookmark, When the AI tool deletes it, Then the service
    /// wrote BookmarkDeleted to the outbox. (save_bookmark fetches the URL's
    /// metadata over the network, so the tool test seeds through the service.)
    #[tokio::test]
    async fn delete_bookmark_goes_through_the_service_and_writes_the_outbox() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("bookmark_tool_outbox");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let bookmark = services
            .bookmarks
            .create(
                &owner,
                "https://example.com",
                None,
                None,
                ExtractedMetadata {
                    title: "Example".to_string(),
                    domain: "example.com".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let out = execute_tool(
            &services,
            &owner,
            "delete_bookmark",
            &json!({"id": bookmark.id}),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(out.contains("\"deleted\":true"));
        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["BookmarkCreated", "BookmarkDeleted"]
        );
    }
}
