//! Authorized Matrix Room Invitation Handler
//!
//! Handles room invitation events from allowlisted or linked Matrix accounts.

use matrix_sdk::{
    room::Room,
    ruma::events::room::member::{MembershipState, StrippedRoomMemberEvent},
    Client,
};
use tracing::{error, info, warn};

use crate::db::Database;

/// Registers the room member invitation event handler on the Matrix client.
pub fn register_invite_handler(client: &Client, db: Database) {
    let allowlist: Vec<String> = std::env::var("MATRIX_VERIFICATION_ALLOWLIST")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    client.add_event_handler(move |ev: StrippedRoomMemberEvent, room: Room| {
        let db = db.clone();
        let allowlist = allowlist.clone();
        async move {
            if ev.content.membership == MembershipState::Invite {
                let sender = ev.sender.as_str();
                let is_allowlisted = allowlist.iter().any(|a| a == sender);
                let is_linked =
                    sqlx::query_scalar::<_, i64>("SELECT 1 FROM matrix_links WHERE mxid = ?")
                        .bind(sender)
                        .fetch_optional(&db.pool)
                        .await
                        .ok()
                        .flatten()
                        .is_some();

                if is_allowlisted || is_linked {
                    info!(
                        "Authorized invitation from {} for room: {}",
                        sender,
                        room.room_id()
                    );
                    if let Err(err) = room.join().await {
                        error!("Failed to join room {}: {:?}", room.room_id(), err);
                    } else {
                        info!("Successfully joined room: {}", room.room_id());
                    }
                } else {
                    warn!(
                        "Rejected unauthorized room invitation from {} for room: {}",
                        sender,
                        room.room_id()
                    );
                    let _ = room.leave().await;
                }
            }
        }
    });
}
