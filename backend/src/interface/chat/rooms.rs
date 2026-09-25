//! The Matrix side of rooms: which rooms the bot and an account actually
//! share, read live from the homeserver.
//!
//! Needed by the REST room picker, the inbound webhook endpoints and the
//! webhook outbox worker; the worker is a background task with no HTTP
//! request behind it, so this lives with the chat adapter rather than in
//! `interface/http`. Rooms has no chat commands, hence no `BotModule`.

use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::{OwnedUserId, UserId};
use matrix_sdk::{Client, RoomMemberships, RoomState};
use tracing::warn;

use crate::application::rooms::RoomService;
use crate::domain::rooms::JoinedRoom;

/// Whether `user_id` is currently joined to `room`.
async fn is_joined_member(room: &Room, user_id: &UserId) -> bool {
    matches!(
        room.get_member(user_id).await,
        Ok(Some(member)) if *member.membership() == MembershipState::Join
    )
}

/// The name the room picker shows for `room`, if the homeserver offers one.
///
/// A direct chat is named after the other member (e.g. "evan") even when the
/// room has a name of its own, because that is how the user thinks of it.
async fn room_display_name(room: &Room, bot_id: &UserId, is_direct: bool) -> Option<String> {
    let mut name = if is_direct { None } else { room.name() };

    if name.is_none() {
        if let Ok(dn) = room.display_name().await {
            let s = dn.to_string();
            // The SDK's placeholder for a room it cannot name.
            if !s.is_empty() && s != "Empty Room" {
                name = Some(s);
            }
        }
    }

    if name.is_none() || is_direct {
        if let Ok(members) = room.members_no_sync(RoomMemberships::JOIN).await {
            let other = members
                .iter()
                .filter(|m| m.user_id() != bot_id)
                .map(|m| {
                    m.display_name()
                        .map(str::to_owned)
                        .unwrap_or_else(|| m.user_id().localpart().to_owned())
                })
                .find(|n| !n.is_empty());
            if other.is_some() {
                name = other;
            }
        }
    }

    name.or_else(|| room.canonical_alias().map(|a| a.to_string()))
}

/// Describes a joined room for the web room picker; `None` for a space.
async fn describe_room(room: &Room, bot_id: &UserId) -> Option<JoinedRoom> {
    if room.is_space() {
        return None;
    }
    let is_direct = room.is_direct().await.unwrap_or(false);
    let name = room_display_name(room, bot_id, is_direct).await;
    Some(JoinedRoom::new(
        room.room_id().to_string(),
        name.as_deref(),
        is_direct,
    ))
}

/// Parses the account's linked Matrix ids, skipping (and logging) any that
/// are not valid. A failed lookup is logged and treated as no links, so the
/// caller sees no reachable rooms rather than an error.
async fn linked_user_ids(rooms: &RoomService, account_id: &str) -> Vec<OwnedUserId> {
    let linked = rooms.linked_mxids(account_id).await.unwrap_or_else(|err| {
        warn!(account_id, error = %err, "Could not read linked Matrix ids");
        Vec::new()
    });
    linked
        .iter()
        .filter_map(|mxid| match UserId::parse(mxid) {
            Ok(id) => Some(id),
            Err(_) => {
                warn!("Linked Matrix ID is not parseable, skipping: {}", mxid);
                None
            }
        })
        .collect()
}

/// Discovers the rooms where both the bot and at least one of the account's
/// linked Matrix ids are joined, in no particular order (the service orders
/// what it returns). Accepts pending invites first, so a room the user just
/// invited the bot to is included.
pub async fn discover_rooms_for_account(
    rooms: &RoomService,
    client: &Client,
    account_id: &str,
) -> Vec<JoinedRoom> {
    let callers = linked_user_ids(rooms, account_id).await;
    if callers.is_empty() {
        return Vec::new();
    }

    for room in client.invited_rooms() {
        if let Err(err) = room.join().await {
            warn!("Failed to join invited room {}: {:?}", room.room_id(), err);
        }
    }

    let Some(bot_id) = client.user_id() else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for room in client.joined_rooms() {
        if room.state() != RoomState::Joined || !is_joined_member(&room, bot_id).await {
            continue;
        }
        if !any_is_joined(&room, &callers).await {
            continue;
        }
        if let Some(joined) = describe_room(&room, bot_id).await {
            found.push(joined);
        }
    }

    found
}

async fn any_is_joined(room: &Room, users: &[OwnedUserId]) -> bool {
    for user in users {
        if is_joined_member(room, user).await {
            return true;
        }
    }
    false
}

/// Whether the account may target `room_id`: cached as one of its rooms, or
/// — checked live when a client is connected — joined by the bot and one of
/// the account's linked Matrix ids. A room proven live is cached for next
/// time.
pub async fn is_room_authorized_for_account(
    rooms: &RoomService,
    client: Option<&Client>,
    account_id: &str,
    room_id: &str,
) -> bool {
    match rooms.is_cached(account_id, room_id).await {
        Ok(true) => return true,
        Ok(false) => {}
        // Not fatal: the live check below can still prove membership.
        Err(err) => warn!(account_id, room_id, error = %err, "Room cache unreadable"),
    }

    let Some(client) = client else {
        return false;
    };
    let Ok(parsed) = <&matrix_sdk::ruma::RoomId>::try_from(room_id) else {
        return false;
    };
    let Some(room) = client.get_room(parsed) else {
        return false;
    };
    if room.state() != RoomState::Joined {
        return false;
    }
    if !any_is_joined(&room, &linked_user_ids(rooms, account_id).await).await {
        return false;
    }

    let joined = JoinedRoom::new(room_id, room.name().as_deref(), false);
    if let Err(err) = rooms.remember(account_id, &joined).await {
        // Membership is already proven; only the cache shortcut is lost.
        warn!(account_id, room_id, error = %err, "Could not cache an authorized room");
    }
    true
}
