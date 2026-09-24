use axum::{extract::State, Json};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::modules::rooms::discover_rooms_for_account;

pub use crate::db::rooms::{
    get_cached_rooms, resolve_target_room, resolve_user_default_room, save_cached_rooms,
    JoinedRoomInfo, RoomRepository,
};

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
    use crate::events::EventBroadcaster;

    #[tokio::test]
    async fn test_list_rooms_without_a_bot_client_is_empty() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool },
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

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
            crate::db::Database { pool: pool },
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
