use axum::{extract::State, Json};
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::{OwnedUserId, UserId};
use sqlx::SqlitePool;
use tracing::warn;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;

pub use crate::db::rooms::{
    get_cached_rooms, resolve_target_room, resolve_user_default_room, save_cached_rooms,
    JoinedRoomInfo, RoomRepository,
};

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
    pool: &SqlitePool,
    client: &matrix_sdk::Client,
    account_id: &str,
) -> Vec<JoinedRoomInfo> {
    let linked = crate::accounts::AccountRepository::linked_mxids(pool, account_id)
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
    pool: &SqlitePool,
    client: Option<&matrix_sdk::Client>,
    account_id: &str,
    room_id: &str,
) -> bool {
    // 1. Fast path: check cached account_rooms
    if let Ok(Some(_)) = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM account_rooms WHERE account_id = ? AND room_id = ? LIMIT 1",
    )
    .bind(account_id)
    .bind(room_id)
    .fetch_optional(pool)
    .await
    {
        return true;
    }

    // 2. If client is present, test live membership
    if let Some(c) = client {
        if let Ok(room_id_parsed) = <&matrix_sdk::ruma::RoomId>::try_from(room_id) {
            if let Some(room) = c.get_room(room_id_parsed) {
                if room.state() == matrix_sdk::RoomState::Joined {
                    let linked = crate::accounts::AccountRepository::linked_mxids(pool, account_id)
                        .await
                        .unwrap_or_default();
                    for mxid in linked {
                        if let Ok(uid) = UserId::parse(&mxid) {
                            if is_joined_member(&room, &uid).await {
                                let _ = sqlx::query(
                                    "INSERT OR IGNORE INTO account_rooms (account_id, room_id, name, is_direct) VALUES (?, ?, ?, 0)",
                                )
                                .bind(account_id)
                                .bind(room_id)
                                .bind(room.name().unwrap_or_else(|| room_id.to_string()))
                                .execute(pool)
                                .await;
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

#[utoipa::path(
    get,
    path = "/api/v1/rooms",
    responses(
        (status = 200, description = "List Matrix rooms joined by bot (cached in database)", body = Vec<JoinedRoomInfo>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_rooms(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<JoinedRoomInfo>>, ApiError> {
    // 1. Check database cache first — instantaneous response without network delay
    let cached = get_cached_rooms(&state.db.pool, &user.account_id)
        .await
        .unwrap_or_default();

    if !cached.is_empty() {
        return Ok(Json(cached));
    }

    // 2. If database has no cached rooms, perform initial discovery once and cache
    let Some(client) = state.bot_client.read().await.clone() else {
        return Ok(Json(Vec::new()));
    };

    let discovered = discover_rooms_for_account(&state.db.pool, &client, &user.account_id).await;
    if !discovered.is_empty() {
        let _ = save_cached_rooms(&state.db.pool, &user.account_id, &discovered).await;
        if let Ok(saved) = get_cached_rooms(&state.db.pool, &user.account_id).await {
            return Ok(Json(saved));
        }
    }

    Ok(Json(discovered))
}

#[utoipa::path(
    post,
    path = "/api/v1/rooms/sync",
    responses(
        (status = 200, description = "Synchronize Matrix rooms from homeserver and update database", body = Vec<JoinedRoomInfo>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 503, description = "Matrix bot client not connected", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn sync_rooms(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<JoinedRoomInfo>>, ApiError> {
    let Some(client) = state.bot_client.read().await.clone() else {
        return Err(ApiError::bad_request("Matrix bot client not connected"));
    };

    let discovered = discover_rooms_for_account(&state.db.pool, &client, &user.account_id).await;
    save_cached_rooms(&state.db.pool, &user.account_id, &discovered)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to save cached rooms: {}", e)))?;

    let refreshed = get_cached_rooms(&state.db.pool, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to read cached rooms: {}", e)))?;

    Ok(Json(refreshed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_list_rooms_without_a_bot_client_is_empty() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::for_account("acct_alice");

        let res = list_rooms(State(state), user).await.unwrap();
        assert!(res.0.is_empty());
    }

    #[tokio::test]
    async fn test_cached_rooms_crud_and_list() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Create account
        crate::accounts::AccountRepository::create_with_id(
            &pool,
            "acct_test_rooms",
            "Tester",
            crate::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();

        let initial = get_cached_rooms(&pool, "acct_test_rooms").await.unwrap();
        assert!(initial.is_empty());

        let rooms = vec![
            JoinedRoomInfo {
                room_id: "!room1:example.com".to_string(),
                name: "General".to_string(),
                is_direct: false,
                updated_at: None,
            },
            JoinedRoomInfo {
                room_id: "!room2:example.com".to_string(),
                name: "DEV".to_string(),
                is_direct: false,
                updated_at: None,
            },
        ];

        save_cached_rooms(&pool, "acct_test_rooms", &rooms)
            .await
            .unwrap();

        let cached = get_cached_rooms(&pool, "acct_test_rooms").await.unwrap();
        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0].name, "DEV");
        assert_eq!(cached[1].name, "General");
        assert!(cached[0].updated_at.is_some());

        // Test list_rooms hits cache directly without bot client
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::for_account("acct_test_rooms");

        let res = list_rooms(State(state), user).await.unwrap();
        assert_eq!(res.0.len(), 2);
        assert_eq!(res.0[0].name, "DEV");
    }
}
