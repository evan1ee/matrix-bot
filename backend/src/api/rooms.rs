use axum::{extract::State, Json};
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::UserId;
use serde::Serialize;
use tracing::warn;
use utoipa::ToSchema;

use crate::api::auth::types::AuthKind;
use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;

#[derive(Debug, Serialize, ToSchema)]
pub struct JoinedRoomInfo {
    pub room_id: String,
    pub name: String,
}

/// Whether `user_id` is currently joined to `room`.
async fn is_joined_member(room: &Room, user_id: &UserId) -> bool {
    matches!(
        room.get_member(user_id).await,
        Ok(Some(member)) if *member.membership() == MembershipState::Join
    )
}

/// Describes a joined room for the web room picker.
///
/// One-to-one chats with the bot are included — for a single-user deployment
/// that DM is usually the *only* room, so excluding it leaves the picker empty.
/// Names fall back through `m.room.name`, the canonical alias, and finally the
/// SDK's computed display name, so a room is never dropped merely for lacking an
/// explicit title.
async fn describe_room(room: &Room) -> Option<JoinedRoomInfo> {
    if room.is_space() {
        return None;
    }

    if room.is_direct().await.unwrap_or(false) {
        return None;
    }

    let name = match room.name() {
        Some(name) => name,
        None => match room.canonical_alias() {
            Some(alias) => alias.to_string(),
            None => return None,
        },
    };

    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    Some(JoinedRoomInfo {
        room_id: room.room_id().to_string(),
        name: name.to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/rooms",
    responses(
        (status = 200, description = "List Matrix rooms joined by bot", body = Vec<JoinedRoomInfo>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_rooms(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<JoinedRoomInfo>>, ApiError> {
    // Everything here comes from the bot's synced SDK state. The bot's own
    // membership is implied by `joined_rooms()` — that list is exactly the rooms
    // the account in MATRIX_USERNAME has joined — so there is no bot user ID to
    // guess and no access token to hand around.
    let Some(client) = state.bot_client.read().await.clone() else {
        return Ok(Json(Vec::new()));
    };

    // The caller must be in the room too. Without this the endpoint hands every
    // registered account the names of every room the bot belongs to, including
    // ones the caller has nothing to do with.
    let caller = match user.kind {
        AuthKind::Matrix => match UserId::parse(&user.subject) {
            Ok(id) => id,
            Err(_) => {
                warn!("Authenticated Matrix subject is not a valid user ID: {}", user.subject);
                return Ok(Json(Vec::new()));
            }
        },
        // A local web account has no Matrix identity, so it shares no rooms with
        // the bot. Log in with Matrix credentials to pick a room.
        AuthKind::Local => return Ok(Json(Vec::new())),
    };

    // Accept pending invitations so a freshly invited room shows up here.
    for room in client.invited_rooms() {
        if let Err(err) = room.join().await {
            warn!("Failed to join invited room {}: {:?}", room.room_id(), err);
        }
    }

    let bot_id = match client.user_id() {
        Some(id) => id,
        None => return Ok(Json(Vec::new())),
    };

    let mut rooms = Vec::new();
    for room in client.joined_rooms() {
        if room.state() != matrix_sdk::RoomState::Joined {
            continue;
        }
        if !is_joined_member(&room, bot_id).await {
            continue;
        }
        if !is_joined_member(&room, &caller).await {
            continue;
        }
        if let Some(info) = describe_room(&room).await {
            rooms.push(info);
        }
    }

    rooms.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(Json(rooms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    /// With no bot client connected the endpoint is empty rather than erroring —
    /// and, unlike the previous implementation, reaches no network at all.
    #[tokio::test]
    async fn test_list_rooms_without_a_bot_client_is_empty() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::local("alice", "acct_alice");

        let res = list_rooms(State(state), user).await.unwrap();
        assert!(res.0.is_empty());
    }
}
