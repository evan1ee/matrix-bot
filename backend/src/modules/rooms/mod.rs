//! Which Matrix rooms an account may reach.
//!
//! Domain logic rather than transport: the REST room picker, the inbound
//! webhook endpoint and the outbox delivery worker all need the same answer,
//! and the worker is a background task with no HTTP request behind it. Kept out
//! of `db::rooms` deliberately — that layer has no `matrix_sdk` dependency and
//! resolving a room needs one.

use crate::db::DbPool;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::{OwnedUserId, UserId};
use tracing::warn;

use crate::accounts::AccountRepository;
use crate::db::rooms::JoinedRoomInfo;

/// Whether `user_id` is currently joined to `room`.
async fn is_joined_member(room: &Room, user_id: &UserId) -> bool {
    matches!(
        room.get_member(user_id).await,
        Ok(Some(member)) if *member.membership() == MembershipState::Join
    )
}

/// Describes a joined room for the web room picker.
async fn describe_room(room: &Room, bot_id: &UserId) -> Option<JoinedRoomInfo> {
    if room.is_space() {
        return None;
    }

    let is_direct = room.is_direct().await.unwrap_or(false);

    // If it's a direct message room, resolve the other member's name (e.g. "evan")
    let mut resolved_name = if is_direct { None } else { room.name() };

    // If still unresolved, try display_name()
    if resolved_name.is_none() {
        if let Ok(dn) = room.display_name().await {
            let s = dn.to_string();
            if !s.is_empty() && s != "Empty Room" {
                resolved_name = Some(s);
            }
        }
    }

    // If still unresolved or is_direct, look for the other joined member's display name or localpart
    if resolved_name.is_none() || is_direct {
        if let Ok(members) = room
            .members_no_sync(matrix_sdk::RoomMemberships::JOIN)
            .await
        {
            for m in &members {
                if m.user_id() != bot_id {
                    let member_name = m
                        .display_name()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| m.user_id().localpart().to_string());
                    if !member_name.is_empty() {
                        resolved_name = Some(member_name);
                        break;
                    }
                }
            }
        }
    }

    // Try canonical alias
    if resolved_name.is_none() {
        resolved_name = room.canonical_alias().map(|a| a.to_string());
    }

    let final_name = resolved_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| room.room_id().to_string());

    Some(JoinedRoomInfo {
        room_id: room.room_id().to_string(),
        name: final_name,
        is_direct,
        updated_at: None,
    })
}

/// Discovers Matrix rooms where both the bot and at least one linked Matrix account of the user are joined.
pub async fn discover_rooms_for_account(
    pool: &DbPool,
    client: &matrix_sdk::Client,
    account_id: &str,
) -> Vec<JoinedRoomInfo> {
    let linked = AccountRepository::linked_mxids(pool, account_id)
        .await
        .unwrap_or_default();

    let callers: Vec<OwnedUserId> = linked
        .iter()
        .filter_map(|mxid| match UserId::parse(mxid) {
            Ok(id) => Some(id),
            Err(_) => {
                warn!("Linked Matrix ID is not parseable, skipping: {}", mxid);
                None
            }
        })
        .collect();

    if callers.is_empty() {
        return Vec::new();
    }

    for room in client.invited_rooms() {
        if let Err(err) = room.join().await {
            warn!("Failed to join invited room {}: {:?}", room.room_id(), err);
        }
    }

    let bot_id = match client.user_id() {
        Some(id) => id,
        None => return Vec::new(),
    };

    let mut rooms = Vec::new();
    for room in client.joined_rooms() {
        if room.state() != matrix_sdk::RoomState::Joined {
            continue;
        }
        if !is_joined_member(&room, bot_id).await {
            continue;
        }
        let mut caller_is_member = false;
        for caller in &callers {
            if is_joined_member(&room, caller).await {
                caller_is_member = true;
                break;
            }
        }
        if !caller_is_member {
            continue;
        }
        if let Some(info) = describe_room(&room, bot_id).await {
            rooms.push(info);
        }
    }

    rooms.sort_by(|a, b| {
        b.is_direct
            .cmp(&a.is_direct)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    rooms
}

/// Validates whether a room is joined by the account (checking cached rooms or discovering live).
pub async fn is_room_authorized_for_account(
    pool: &DbPool,
    client: Option<&matrix_sdk::Client>,
    account_id: &str,
    room_id: &str,
) -> bool {
    // 1. Fast path: check cached account_rooms
    if let Ok(Some(_)) = crate::db_scalar_optional!(
        pool,
        i64,
        "SELECT 1 FROM account_rooms WHERE account_id = ? AND room_id = ? LIMIT 1",
        account_id,
        room_id
    ) {
        return true;
    }

    // 2. If client is present, test live membership
    if let Some(c) = client {
        if let Ok(room_id_parsed) = <&matrix_sdk::ruma::RoomId>::try_from(room_id) {
            if let Some(room) = c.get_room(room_id_parsed) {
                if room.state() == matrix_sdk::RoomState::Joined {
                    let linked = AccountRepository::linked_mxids(pool, account_id)
                        .await
                        .unwrap_or_default();
                    for mxid in linked {
                        if let Ok(uid) = UserId::parse(&mxid) {
                            if is_joined_member(&room, &uid).await {
                                let _ = crate::db_execute!(pool, "INSERT OR IGNORE INTO account_rooms (account_id, room_id, name, is_direct) VALUES (?, ?, ?, 0)", account_id, room_id, room.name().unwrap_or_else(|| room_id.to_string()));
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    false
}
