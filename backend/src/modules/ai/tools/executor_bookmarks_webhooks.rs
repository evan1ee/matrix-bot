use serde_json::json;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::bookmarks::repository::BookmarksRepository;
use crate::modules::webhook::repository::WebhookRepository;

pub async fn execute_bookmarks_webhooks_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
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

        "get_my_webhooks" => Some(
            async {
                let hooks = WebhookRepository::list_by_owner(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&hooks).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_webhook_logs" => Some(
            async {
                let webhook_id = args
                    .get("webhook_id")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'webhook_id' is required")?;
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10);
                let logs =
                    WebhookRepository::list_outbox_by_webhook(pool, webhook_id, owner_id, limit, 0)
                        .await
                        .map_err(|e| e.to_string())?;
                serde_json::to_string(&logs).map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}
