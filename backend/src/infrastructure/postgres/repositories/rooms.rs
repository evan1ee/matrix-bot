//! `account_rooms`: each account's cache of the Matrix rooms it shares with
//! the bot. The only SQL for rooms; every function takes any
//! `PgExecutor`, so the service can run it on the pool or inside its own
//! transaction (`&mut *tx`).

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::rooms::{CachedRoom, JoinedRoom};

/// `account_rooms` as stored. Private: rows become domain types at this edge.
struct CachedRoomRow {
    room_id: String,
    name: String,
    is_direct: bool,
    updated_at: String,
}

// `From` rather than `TryFrom`: every stored row is a valid `CachedRoom`
// (`JoinedRoom::new` normalises the name instead of rejecting it).
impl From<CachedRoomRow> for CachedRoom {
    fn from(row: CachedRoomRow) -> Self {
        CachedRoom::new(
            JoinedRoom::new(row.room_id, Some(&row.name), row.is_direct),
            row.updated_at,
        )
    }
}

/// The account's cached rooms, in picker order (`JoinedRoom::picker_order`).
pub async fn list_for_account(
    conn: impl PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Vec<CachedRoom>, sqlx::Error> {
    let rows = sqlx::query_as!(
        CachedRoomRow,
        "SELECT room_id, name, is_direct, updated_at FROM account_rooms \
         WHERE account_id = $1 ORDER BY is_direct DESC, LOWER(name) ASC",
        account_id.as_uuid()
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(CachedRoom::from).collect())
}

/// Whether `room_id` is in the account's cache.
pub async fn is_cached(
    conn: impl PgExecutor<'_>,
    account_id: &AccountId,
    room_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM account_rooms WHERE account_id = $1 AND room_id = $2
           ) AS "cached!""#,
        account_id.as_uuid(),
        room_id
    )
    .fetch_one(conn)
    .await
}

/// Caches `room` for the account, stamped with the current time. A room
/// already cached is left as it is, so re-proving membership is harmless.
pub async fn insert(
    conn: impl PgExecutor<'_>,
    account_id: &AccountId,
    room: &JoinedRoom,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO account_rooms (account_id, room_id, name, is_direct) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        account_id.as_uuid(),
        room.room_id(),
        room.name(),
        room.is_direct()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Empties the account's cache.
pub async fn delete_for_account(
    conn: impl PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM account_rooms WHERE account_id = $1",
        account_id.as_uuid()
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_accounts(ids: &[&AccountId]) -> sqlx::PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        for id in ids {
            crate::infrastructure::postgres::repositories::accounts::core::create_with_id(
                &pool,
                id,
                "Tester",
                crate::domain::accounts::STATUS_ACTIVE,
            )
            .await
            .unwrap();
        }
        pool
    }

    fn room(room_id: &str, name: &str, is_direct: bool) -> JoinedRoom {
        JoinedRoom::new(room_id, Some(name), is_direct)
    }

    #[tokio::test]
    async fn cached_rooms_come_back_in_picker_order_with_their_cache_time() {
        let acct_a = AccountId::new("acct_a");
        let pool = pool_with_accounts(&[&acct_a]).await;
        for r in [
            room("!g:x", "general", false),
            room("!dm:x", "evan", true),
            room("!d:x", "DEV", false),
        ] {
            insert(&pool, &acct_a, &r).await.unwrap();
        }

        let cached = list_for_account(&pool, &acct_a).await.unwrap();

        let ids: Vec<&str> = cached.iter().map(|c| c.room().room_id()).collect();
        assert_eq!(ids, ["!dm:x", "!d:x", "!g:x"]);
        assert!(cached[0].room().is_direct());
        assert!(cached.iter().all(|c| !c.updated_at().is_empty()));
    }

    #[tokio::test]
    async fn an_account_sees_only_its_own_rooms() {
        let acct_a = AccountId::new("acct_a");
        let acct_b = AccountId::new("acct_b");
        let pool = pool_with_accounts(&[&acct_a, &acct_b]).await;
        insert(&pool, &acct_a, &room("!a:x", "a", false))
            .await
            .unwrap();

        assert!(list_for_account(&pool, &acct_b).await.unwrap().is_empty());
        assert!(!is_cached(&pool, &acct_b, "!a:x").await.unwrap());
        assert!(is_cached(&pool, &acct_a, "!a:x").await.unwrap());
    }

    #[tokio::test]
    async fn inserting_a_cached_room_again_keeps_the_first_entry() {
        let acct_a = AccountId::new("acct_a");
        let pool = pool_with_accounts(&[&acct_a]).await;
        insert(&pool, &acct_a, &room("!a:x", "first", false))
            .await
            .unwrap();
        insert(&pool, &acct_a, &room("!a:x", "second", true))
            .await
            .unwrap();

        let cached = list_for_account(&pool, &acct_a).await.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].room().name(), "first");
    }

    // Rows cached before names were normalised on write may carry padding
    // or be blank; reading them goes through `JoinedRoom::new` like any other.
    #[tokio::test]
    async fn a_stored_name_is_normalised_on_read() {
        let acct_a = AccountId::new("acct_a");
        let pool = pool_with_accounts(&[&acct_a]).await;
        sqlx::query!(
            "INSERT INTO account_rooms (account_id, room_id, name) \
             VALUES ($1, '!padded:x', '  Ops  '), ($1, '!blank:x', '   ')",
            acct_a.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let cached = list_for_account(&pool, &acct_a).await.unwrap();

        let names: Vec<&str> = cached.iter().map(|c| c.room().name()).collect();
        assert_eq!(names, ["!blank:x", "Ops"]);
    }

    #[tokio::test]
    async fn delete_empties_only_that_accounts_cache() {
        let acct_a = AccountId::new("acct_a");
        let acct_b = AccountId::new("acct_b");
        let pool = pool_with_accounts(&[&acct_a, &acct_b]).await;
        insert(&pool, &acct_a, &room("!a:x", "a", false))
            .await
            .unwrap();
        insert(&pool, &acct_b, &room("!b:x", "b", false))
            .await
            .unwrap();

        delete_for_account(&pool, &acct_a).await.unwrap();

        assert!(list_for_account(&pool, &acct_a).await.unwrap().is_empty());
        assert_eq!(list_for_account(&pool, &acct_b).await.unwrap().len(), 1);
    }
}
