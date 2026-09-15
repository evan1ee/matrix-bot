use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_settings" => Some(
            async {
                let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                serde_json::to_string(&settings).map_err(|e| e.to_string())
            }
            .await,
        ),

        "update_my_settings" => Some(
            async {
                let mut settings = SettingsRepository::get_or_default(pool, owner_id).await;
                if let Some(tz) = args.get("timezone").and_then(|v| v.as_str()) {
                    settings.timezone = tz.to_string();
                }
                if let Some(curr) = args.get("currency").and_then(|v| v.as_str()) {
                    settings.currency = curr.to_uppercase();
                }

                let saved = SettingsRepository::upsert(pool, owner_id, &settings)
                    .await
                    .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::SettingsUpdated {
                        owner_id: owner_id.to_string(),
                        settings: saved.clone(),
                    });
                }

                serde_json::to_string(&saved).map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}
