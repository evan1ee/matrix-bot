use crate::db::DbPool;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JoinedRoomInfo {
    pub room_id: String,
    pub name: String,
    pub is_direct: bool,
    pub updated_at: Option<String>,
}

/// Repository for room querying and resolution from the database.
pub struct RoomRepository;

impl RoomRepository {
    /// Reads cached rooms for an account from SQLite.
    pub async fn get_cached_rooms(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Vec<JoinedRoomInfo>, sqlx::Error> {
        // Selected as a typed tuple rather than read column-by-column off a raw
        // row: `SqliteRow` and `PgRow` are different types, so `row.get` cannot
        // be written once, while a tuple `FromRow` serves both.
        let rows: Vec<(String, String, bool, Option<String>)> = crate::db_fetch_all!(
            pool,
            (String, String, bool, Option<String>),
            "SELECT room_id, name, is_direct, updated_at FROM account_rooms              WHERE account_id = ? ORDER BY is_direct DESC, LOWER(name) ASC",
            account_id
        )?;

        let rooms = rows
            .into_iter()
            .map(|(room_id, name, is_direct, updated_at)| JoinedRoomInfo {
                room_id,
                name,
                is_direct,
                updated_at,
            })
            .collect();

        Ok(rooms)
    }

    /// Replaces the cached rooms for an account, atomically.
    pub async fn save_cached_rooms(
        pool: &DbPool,
        account_id: &str,
        rooms: &[JoinedRoomInfo],
    ) -> Result<(), sqlx::Error> {
        let mut tx = pool.begin().await?;

        crate::tx_execute!(
            &mut tx,
            "DELETE FROM account_rooms WHERE account_id = ?",
            account_id
        )?;

        for room in rooms {
            crate::tx_execute!(
                &mut tx,
                "INSERT INTO account_rooms (account_id, room_id, name, is_direct, updated_at) \
                 VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)",
                account_id,
                &room.room_id,
                &room.name,
                room.is_direct
            )?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Resolves the default Matrix room for an account.
    ///
    /// Priority:
    /// 1. Direct message room with the user (`is_direct = TRUE` in `account_rooms`, e.g. "evan").
    /// 2. Room in `account_rooms` matching the user's linked Matrix username/localpart.
    /// 3. Dev room from user settings (`dev_room_id`).
    /// 4. Any joined room in `account_rooms`.
    pub async fn resolve_user_default_room(pool: &DbPool, account_id: &str) -> Option<String> {
        // 1. Direct 1:1 room in account_rooms
        if let Ok(Some(room_id)) = crate::db_scalar_optional!(pool, String, "SELECT room_id FROM account_rooms WHERE account_id = ? AND is_direct = TRUE ORDER BY updated_at DESC LIMIT 1;", account_id)
        {
            return Some(room_id);
        }

        // 2. Room matching user's linked Matrix username
        if let Ok(linked_mxids) =
            crate::accounts::AccountRepository::linked_mxids(pool, account_id).await
        {
            for mxid in linked_mxids {
                let localpart = mxid
                    .trim_start_matches('@')
                    .split(':')
                    .next()
                    .unwrap_or(&mxid);
                if let Ok(Some(room_id)) = crate::db_scalar_optional!(pool, String, "SELECT room_id FROM account_rooms WHERE account_id = ? AND LOWER(name) = LOWER(?) LIMIT 1;", account_id, localpart)
                {
                    return Some(room_id);
                }
            }
        }

        // 3. User settings dev_room_id
        if let Ok(Some(dev_room)) = crate::db_scalar_optional!(pool, String, "SELECT dev_room_id FROM user_settings WHERE owner_id = ? AND dev_room_id IS NOT NULL AND TRIM(dev_room_id) != '' LIMIT 1;", account_id)
        {
            return Some(dev_room);
        }

        // 4. Any room in account_rooms
        crate::db_scalar_optional!(pool, String, "SELECT room_id FROM account_rooms WHERE account_id = ? ORDER BY is_direct DESC, updated_at DESC LIMIT 1;", account_id)
        .ok()
        .flatten()
    }

    /// Resolves a target room for an action (e.g. reminder, task, chat turn).
    /// If `query` is specified and not a generic placeholder ("web", "web_chat"),
    /// attempts to match a room by ID or name. Otherwise falls back to the user's
    /// default room (their 1:1 direct room).
    pub async fn resolve_target_room(
        pool: &DbPool,
        account_id: &str,
        query: Option<&str>,
    ) -> String {
        let trimmed = query.map(str::trim).unwrap_or("");
        if !trimmed.is_empty() && trimmed != "web" && trimmed != "web_chat" {
            // Direct room ID match (starts with '!')
            if trimmed.starts_with('!') {
                return trimmed.to_string();
            }

            // Exact name or ID match in account_rooms
            if let Ok(Some(room_id)) = crate::db_scalar_optional!(pool, String, "SELECT room_id FROM account_rooms WHERE account_id = ? AND (LOWER(name) = LOWER(?) OR room_id = ?) LIMIT 1;", account_id, trimmed, trimmed)
            {
                return room_id;
            }

            // Partial name match in account_rooms
            if let Ok(Some(room_id)) = crate::db_scalar_optional!(pool, String, "SELECT room_id FROM account_rooms WHERE account_id = ? AND LOWER(name) LIKE LOWER(?) LIMIT 1;", account_id, format!("%{}%", trimmed))
            {
                return room_id;
            }

            return trimmed.to_string();
        }

        // Default to the user's personal direct room
        Self::resolve_user_default_room(pool, account_id)
            .await
            .unwrap_or_else(|| "web".to_string())
    }
}

/// Convenience free function delegating to `RoomRepository::get_cached_rooms`.
pub async fn get_cached_rooms(
    pool: &DbPool,
    account_id: &str,
) -> Result<Vec<JoinedRoomInfo>, sqlx::Error> {
    RoomRepository::get_cached_rooms(pool, account_id).await
}

/// Convenience free function delegating to `RoomRepository::save_cached_rooms`.
pub async fn save_cached_rooms(
    pool: &DbPool,
    account_id: &str,
    rooms: &[JoinedRoomInfo],
) -> Result<(), sqlx::Error> {
    RoomRepository::save_cached_rooms(pool, account_id, rooms).await
}

/// Convenience free function delegating to `RoomRepository::resolve_user_default_room`.
pub async fn resolve_user_default_room(pool: &DbPool, account_id: &str) -> Option<String> {
    RoomRepository::resolve_user_default_room(pool, account_id).await
}

/// Convenience free function delegating to `RoomRepository::resolve_target_room`.
pub async fn resolve_target_room(pool: &DbPool, account_id: &str, query: Option<&str>) -> String {
    RoomRepository::resolve_target_room(pool, account_id, query).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_room_repository_crud_and_resolution() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();

        let account_id = "acct_test_rooms";
        crate::accounts::AccountRepository::create_with_id(
            &pool,
            account_id,
            "Tester",
            crate::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();

        // 1. Initial state: empty
        let initial = get_cached_rooms(&pool, account_id).await.unwrap();
        assert!(initial.is_empty());
        assert_eq!(resolve_target_room(&pool, account_id, None).await, "web");

        // 2. Save cached rooms
        let rooms = vec![
            JoinedRoomInfo {
                room_id: "!direct:matrix.org".to_string(),
                name: "Direct Chat".to_string(),
                is_direct: true,
                updated_at: None,
            },
            JoinedRoomInfo {
                room_id: "!dev:matrix.org".to_string(),
                name: "Dev Channel".to_string(),
                is_direct: false,
                updated_at: None,
            },
        ];
        save_cached_rooms(&pool, account_id, &rooms).await.unwrap();

        let cached = get_cached_rooms(&pool, account_id).await.unwrap();
        assert_eq!(cached.len(), 2);

        // 3. Default room resolves to direct room
        assert_eq!(
            resolve_user_default_room(&pool, account_id).await,
            Some("!direct:matrix.org".to_string())
        );

        // 4. Target room resolves direct when None or web
        assert_eq!(
            resolve_target_room(&pool, account_id, None).await,
            "!direct:matrix.org"
        );
        assert_eq!(
            resolve_target_room(&pool, account_id, Some("web")).await,
            "!direct:matrix.org"
        );

        // 5. Direct room ID starts with '!'
        assert_eq!(
            resolve_target_room(&pool, account_id, Some("!arbitrary:matrix.org")).await,
            "!arbitrary:matrix.org"
        );

        // 6. Name match
        assert_eq!(
            resolve_target_room(&pool, account_id, Some("Dev Channel")).await,
            "!dev:matrix.org"
        );
        assert_eq!(
            resolve_target_room(&pool, account_id, Some("dev")).await,
            "!dev:matrix.org"
        );
    }
}
