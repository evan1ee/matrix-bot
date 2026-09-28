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
pub fn register_invite_handler(client: &Client, db: Database, allowlist: Vec<String>) {
    client.add_event_handler(move |ev: StrippedRoomMemberEvent, room: Room| {
        let db = db.clone();
        let allowlist = allowlist.clone();
        async move {
            if ev.content.membership == MembershipState::Invite {
                let sender = ev.sender.as_str();
                let is_allowlisted = allowlist.iter().any(|a| a == sender);
                let is_linked = crate::db_scalar_optional!(
                    &db.pool,
                    i64,
                    "SELECT CAST(1 AS BIGINT) FROM matrix_links WHERE mxid = ?",
                    sender
                )
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
