use sqlx::SqlitePool;

use crate::modules::webhook::repository::WebhookRepository;

pub async fn execute_tool(
    pool: &SqlitePool,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
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
