use axum::{extract::State, Json};
use matrix_sdk::room::Room;
use serde::Serialize;
use tracing::warn;
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;

#[derive(Debug, Serialize, ToSchema)]
pub struct JoinedRoomInfo {
    pub room_id: String,
    pub name: String,
}

/// Describes a room if it is one a user would want to pick from a list.
///
/// That means a named group room: direct messages and rooms with no explicit
/// name or canonical alias are left out, since their only "name" would be a
/// heuristic built from the member list.
async fn describe_group_room(room: &Room) -> Option<JoinedRoomInfo> {
    if room.joined_members_count() <= 1 {
        return None;
    }

    // `is_direct` consults account data, which is authoritative for DMs —
    // unlike guessing from the member count.
    if room.is_direct().await.unwrap_or(false) {
        return None;
    }

    let name = room
        .name()
        .or_else(|| room.canonical_alias().map(|alias| alias.to_string()))?;

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
    _user: AuthUser,
) -> Result<Json<Vec<JoinedRoomInfo>>, ApiError> {
    // Everything here comes from the bot's synced SDK state. Membership is
    // implied by `joined_rooms()`, so there is no bot user ID to guess and no
    // access token to hand around.
    let Some(client) = state.bot_client.read().await.clone() else {
        return Ok(Json(Vec::new()));
    };

    // Accept pending invitations so a freshly invited room shows up here.
    for room in client.invited_rooms() {
        if let Err(err) = room.join().await {
            warn!("Failed to join invited room {}: {:?}", room.room_id(), err);
        }
    }

    let mut rooms = Vec::new();
    for room in client.joined_rooms() {
        if let Some(info) = describe_group_room(&room).await {
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
        let user = AuthUser::local("alice");

        let res = list_rooms(State(state), user).await.unwrap();
        assert!(res.0.is_empty());
    }
}
