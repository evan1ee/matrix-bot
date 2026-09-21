use crate::db::DbPool;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::RoomId;
use matrix_sdk::Client;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::modules::settings::SettingsRepository;
use crate::utils::html::html_escape;

static AI_ERROR_ALERTED: AtomicBool = AtomicBool::new(false);

/// Resets the outage alert state when AI succeeds or settings change.
pub fn clear_ai_error_state() {
    AI_ERROR_ALERTED.store(false, Ordering::Relaxed);
}

/// Checks whether an AI outage alert has already been sent.
pub fn is_ai_error_alerted() -> bool {
    AI_ERROR_ALERTED.load(Ordering::Relaxed)
}

/// Sends an outage notification to the DEV room, sending strictly once per incident.
pub async fn notify_dev_room_ai_error(
    pool: &DbPool,
    client_handle: Option<&Arc<RwLock<Option<Client>>>>,
    owner_id: &str,
    error_msg: &str,
) {
    // Only send once per outage incident!
    if AI_ERROR_ALERTED.swap(true, Ordering::SeqCst) {
        debug!("AI outage error already alerted to DEV room; suppressing duplicate alert.");
        return;
    }

    let Some(client_handle) = client_handle else {
        warn!("No client handle available to send AI error alert to DEV room.");
        return;
    };

    let client = {
        let guard = client_handle.read().await;
        guard.clone()
    };

    let Some(client) = client else {
        warn!("Matrix client not connected; cannot send AI error alert to DEV room.");
        return;
    };

    // Determine target DEV room:
    // 1. Check user's settings.dev_room_id
    // 2. Fallback: Search joined rooms for a room named "DEV" (case-insensitive)
    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
    let target_room_id = if let Some(ref room_id) = settings.dev_room_id {
        Some(room_id.clone())
    } else {
        let mut dev_id = None;
        for room in client.joined_rooms() {
            if room.state() == matrix_sdk::RoomState::Joined {
                if let Some(name) = room.name() {
                    if name.trim().eq_ignore_ascii_case("dev") {
                        dev_id = Some(room.room_id().to_string());
                        break;
                    }
                }
            }
        }
        dev_id
    };

    let Some(room_id_str) = target_room_id else {
        info!(
            "No DEV room configured or found for owner {}; skipping error alert.",
            owner_id
        );
        return;
    };

    let Ok(room_id) = <&RoomId>::try_from(room_id_str.as_str()) else {
        warn!("Invalid dev_room_id: {}", room_id_str);
        return;
    };

    let plain = format!(
        "⚠️ AI Assistant Error: Unable to reach the AI model provider. Please verify your AI settings or endpoint. ({})",
        error_msg
    );
    let html = format!(
        "⚠️ <b>AI Assistant Error</b>: Unable to reach the AI model provider.<br/><code>{}</code><br/><i>Please verify your AI settings or endpoint.</i>",
        html_escape(error_msg)
    );
    let content = RoomMessageEventContent::text_html(&plain, &html);

    if let Some(room) = client.get_room(room_id) {
        if let Err(e) = room.send(content).await {
            error!(
                "Failed to send AI error alert to DEV room {}: {:?}",
                room_id_str, e
            );
        } else {
            info!("Dispatched AI outage alert to DEV room {}", room_id_str);
        }
    } else {
        match client.join_room_by_id(room_id).await {
            Ok(room) => {
                if let Err(e) = room.send(content).await {
                    error!(
                        "Failed to send AI error alert to joined DEV room {}: {:?}",
                        room_id_str, e
                    );
                } else {
                    info!(
                        "Dispatched AI outage alert to DEV room {} after joining",
                        room_id_str
                    );
                }
            }
            Err(e) => {
                error!(
                    "Could not join DEV room {} to send AI error alert: {:?}",
                    room_id_str, e
                );
            }
        }
    }
}
