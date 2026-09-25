use axum::{extract::State, Json};
use serde::Serialize;
use tracing::warn;
use utoipa::ToSchema;

use crate::domain::rooms::{CachedRoom, JoinedRoom};
use crate::error::ApiError;
use crate::interface::chat::rooms::discover_rooms_for_account;
use crate::interface::http::auth::{ApiState, AuthUser};

// A room in the web room picker. `updated_at` is when it was cached; a room
// discovered live but not yet cached has none. A plain comment, not `///`:
// utoipa copies doc comments into the frozen OpenAPI schema.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JoinedRoomInfo {
    pub room_id: String,
    pub name: String,
    pub is_direct: bool,
    pub updated_at: Option<String>,
}

impl From<&JoinedRoom> for JoinedRoomInfo {
    fn from(room: &JoinedRoom) -> Self {
        Self {
            room_id: room.room_id().to_owned(),
            name: room.name().to_owned(),
            is_direct: room.is_direct(),
            updated_at: None,
        }
    }
}

impl From<CachedRoom> for JoinedRoomInfo {
    fn from(cached: CachedRoom) -> Self {
        Self {
            updated_at: Some(cached.updated_at),
            ..Self::from(&cached.room)
        }
    }
}

fn to_dtos(cached: Vec<CachedRoom>) -> Json<Vec<JoinedRoomInfo>> {
    Json(cached.into_iter().map(JoinedRoomInfo::from).collect())
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
    let rooms = &state.services.rooms;

    // The cache answers instantly; discovery walks every joined room on the
    // homeserver. A failed read degrades to discovery rather than an error.
    let cached = rooms
        .cached_rooms(&user.account_id)
        .await
        .unwrap_or_else(|err| {
            warn!(account_id = %user.account_id, error = %err, "Room cache unreadable");
            Vec::new()
        });
    if !cached.is_empty() {
        return Ok(to_dtos(cached));
    }

    let Some(client) = state.bot_client.read().await.clone() else {
        return Ok(Json(Vec::new()));
    };
    let discovered = discover_rooms_for_account(rooms, &client, &user.account_id).await;
    if discovered.is_empty() {
        return Ok(Json(Vec::new()));
    }

    // Caching is an optimisation here: the picker still gets the rooms.
    match rooms.replace_cached(&user.account_id, &discovered).await {
        Ok(saved) => Ok(to_dtos(saved)),
        Err(err) => {
            warn!(account_id = %user.account_id, error = %err, "Could not cache discovered rooms");
            Ok(Json(discovered.iter().map(JoinedRoomInfo::from).collect()))
        }
    }
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

    let rooms = &state.services.rooms;
    let discovered = discover_rooms_for_account(rooms, &client, &user.account_id).await;
    let saved = rooms.replace_cached(&user.account_id, &discovered).await?;
    Ok(to_dtos(saved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn state_with_account(account_id: &str) -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::accounts::AccountRepository::create_with_id(
            &pool,
            account_id,
            "Tester",
            crate::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_list_rooms_without_a_bot_client_is_empty() {
        let state = state_with_account("acct_alice").await;
        let user = AuthUser::for_account("acct_alice");

        let res = list_rooms(State(state), user).await.unwrap();
        assert!(res.0.is_empty());
    }

    #[tokio::test]
    async fn test_list_rooms_answers_from_the_cache_without_a_bot_client() {
        let state = state_with_account("acct_test_rooms").await;
        state
            .services
            .rooms
            .replace_cached(
                "acct_test_rooms",
                &[
                    JoinedRoom::new("!room1:example.com", Some("General"), false),
                    JoinedRoom::new("!room2:example.com", Some("DEV"), false),
                ],
            )
            .await
            .unwrap();
        let user = AuthUser::for_account("acct_test_rooms");

        let res = list_rooms(State(state), user).await.unwrap();

        let names: Vec<&str> = res.0.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["DEV", "General"]);
        assert!(res.0.iter().all(|r| r.updated_at.is_some()));
    }

    #[tokio::test]
    async fn test_sync_rooms_without_a_bot_client_is_a_bad_request() {
        let state = state_with_account("acct_alice").await;
        let user = AuthUser::for_account("acct_alice");

        let err = sync_rooms(State(state), user).await.unwrap_err();

        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.message, "Matrix bot client not connected");
    }

    #[test]
    fn a_room_not_yet_cached_has_no_cache_time() {
        let dto = JoinedRoomInfo::from(&JoinedRoom::new("!r:x", Some("R"), true));
        assert_eq!(dto.updated_at, None);
        assert!(dto.is_direct);
    }
}
