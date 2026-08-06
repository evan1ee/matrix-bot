use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

use crate::api::auth::matrix::{get_bot_matrix_access_token, get_matrix_sdk_client};
use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;

#[derive(Debug, Serialize, ToSchema)]
pub struct JoinedRoomInfo {
    pub room_id: String,
    pub name: String,
}

pub async fn check_room_bot_membership_and_name(
    http_client: &reqwest::Client,
    clean_hs: &str,
    token: &str,
    room_id: &str,
    bot_user_id: &str,
    client_guard: &Option<matrix_sdk::Client>,
) -> Option<JoinedRoomInfo> {
    let encoded_room_id: String = url::form_urlencoded::byte_serialize(room_id.as_bytes()).collect();

    // 1. Fetch joined members of the room
    let members_url = format!(
        "{}/_matrix/client/v3/rooms/{}/members?membership=join",
        clean_hs, encoded_room_id
    );

    let mut bot_is_joined = false;
    let mut member_count = 0;

    if let Ok(res) = http_client.get(&members_url).bearer_auth(token).send().await {
        if res.status().is_success() {
            #[derive(Deserialize)]
            struct MemberEvent {
                state_key: String,
            }
            #[derive(Deserialize)]
            struct MembersChunk {
                chunk: Vec<MemberEvent>,
            }
            if let Ok(data) = res.json::<MembersChunk>().await {
                member_count = data.chunk.len();
                for m in data.chunk {
                    if m.state_key == bot_user_id || m.state_key.starts_with("@bot:") {
                        bot_is_joined = true;
                    }
                }
            }
        }
    }

    // Check Matrix SDK client store if available
    if let Some(ref client) = client_guard {
        if let Ok(ruma_id) = <&matrix_sdk::ruma::RoomId>::try_from(room_id) {
            if let Some(room) = client.get_room(ruma_id) {
                if room.state() == matrix_sdk::RoomState::Joined {
                    bot_is_joined = true;
                }
            }
        }
    }

    // STRICT FILTER 1: Bot MUST be a joined member, AND room must have at least 2 members
    if !bot_is_joined || member_count <= 1 {
        return None;
    }

    // 2. Fetch room name (m.room.name)
    let name_url = format!(
        "{}/_matrix/client/v3/rooms/{}/state/m.room.name",
        clean_hs, encoded_room_id
    );
    if let Ok(res) = http_client.get(&name_url).bearer_auth(token).send().await {
        if res.status().is_success() {
            #[derive(Deserialize)]
            struct RoomNameState {
                name: Option<String>,
            }
            if let Ok(data) = res.json::<RoomNameState>().await {
                if let Some(n) = data.name {
                    let trimmed = n.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('!') {
                        return Some(JoinedRoomInfo {
                            room_id: room_id.to_string(),
                            name: trimmed.to_string(),
                        });
                    }
                }
            }
        }
    }

    // 3. Fetch canonical_alias (m.room.canonical_alias)
    let alias_url = format!(
        "{}/_matrix/client/v3/rooms/{}/state/m.room.canonical_alias",
        clean_hs, encoded_room_id
    );
    if let Ok(res) = http_client.get(&alias_url).bearer_auth(token).send().await {
        if res.status().is_success() {
            #[derive(Deserialize)]
            struct AliasState {
                alias: Option<String>,
            }
            if let Ok(data) = res.json::<AliasState>().await {
                if let Some(a) = data.alias {
                    let trimmed = a.trim();
                    if !trimmed.is_empty() {
                        return Some(JoinedRoomInfo {
                            room_id: room_id.to_string(),
                            name: trimmed.to_string(),
                        });
                    }
                }
            }
        }
    }

    // 4. Try Matrix SDK client display_name if explicit group name found
    if let Some(ref client) = client_guard {
        if let Ok(ruma_id) = <&matrix_sdk::ruma::RoomId>::try_from(room_id) {
            if let Some(room) = client.get_room(ruma_id) {
                if let Ok(name) = room.display_name().await {
                    let s = name.to_string();
                    if !s.is_empty() && !s.starts_with('!') && member_count > 2 {
                        return Some(JoinedRoomInfo {
                            room_id: room_id.to_string(),
                            name: s,
                        });
                    }
                }
            }
        }
    }

    // EXCLUDE 1-on-1 DM chats (rooms without explicit room title/alias)
    None
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
    let mut raw_room_ids = Vec::new();
    let mut seen_ids = HashSet::new();

    let client_guard = state.matrix_client.read().await;

    // Auto-join any invited rooms if client is active
    if let Some(ref client) = *client_guard {
        for room in client.invited_rooms() {
            let _ = room.join().await;
        }
    }

    // 1. Query Matrix Homeserver joined_rooms REST API directly for bot account
    let bot_token = get_bot_matrix_access_token(&state).await;
    let clean_hs = if let Some(ref token) = bot_token {
        let client = get_matrix_sdk_client(&state.homeserver).await;
        let hs_url = if let Some(c) = client {
            c.homeserver().to_string()
        } else {
            state.homeserver.clone()
        };
        let clean = hs_url.trim_end_matches('/').to_string();
        let joined_url = format!("{}/_matrix/client/v3/joined_rooms", clean);

        if let Ok(res) = state.http_client.get(&joined_url).bearer_auth(token).send().await {
            if res.status().is_success() {
                #[derive(Deserialize)]
                struct JoinedRoomsResponse {
                    joined_rooms: Vec<String>,
                }
                if let Ok(data) = res.json::<JoinedRoomsResponse>().await {
                    for r_id in data.joined_rooms {
                        if !r_id.contains("matrix.org") && seen_ids.insert(r_id.clone()) {
                            raw_room_ids.push(r_id);
                        }
                    }
                }
            }
        }
        clean
    } else {
        state.homeserver.trim_end_matches('/').to_string()
    };

    // 2. Query Matrix SDK client joined_rooms if available
    if let Some(ref client) = *client_guard {
        for room in client.joined_rooms() {
            let room_id = room.room_id().to_string();
            if !room_id.contains("matrix.org") && seen_ids.insert(room_id.clone()) {
                raw_room_ids.push(room_id);
            }
        }
    }

    // 3. Construct bot user ID
    let clean_domain = state
        .homeserver
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let bot_user_id = format!("@bot:{}", clean_domain);

    // 4. Verify bot membership & resolve display names concurrently (only returning named group rooms)
    let token_str = bot_token.unwrap_or_default();
    let mut tasks = Vec::new();
    for r_id in &raw_room_ids {
        let http_client = state.http_client.clone();
        let hs = clean_hs.clone();
        let tok = token_str.clone();
        let id = r_id.clone();
        let b_id = bot_user_id.clone();
        let sdk_client = client_guard.clone();

        tasks.push(async move {
            check_room_bot_membership_and_name(&http_client, &hs, &tok, &id, &b_id, &sdk_client).await
        });
    }

    let results = futures_util::future::join_all(tasks).await;
    let rooms_list: Vec<JoinedRoomInfo> = results.into_iter().flatten().collect();

    Ok(Json(rooms_list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_list_rooms_endpoint() {
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
        let user = AuthUser {
            username: "alice".to_string(),
        };

        let res = list_rooms(State(state), user).await.unwrap();
        assert!(res.0.is_empty());
    }
}
