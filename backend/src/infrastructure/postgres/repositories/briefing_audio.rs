//! `briefing_audio`: which stored recording is a day's spoken briefing.

use chrono::NaiveDate;
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

/// The recording for `local_date`, if one was kept.
pub async fn find_key(
    conn: impl PgExecutor<'_>,
    owner_id: &AccountId,
    local_date: NaiveDate,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT storage_key FROM briefing_audio WHERE owner_id = $1 AND local_date = $2",
        owner_id.as_uuid(),
        local_date
    )
    .fetch_optional(conn)
    .await
}

/// Points `local_date` at `storage_key`, replacing an earlier recording.
pub async fn upsert(
    conn: impl PgExecutor<'_>,
    owner_id: &AccountId,
    local_date: NaiveDate,
    storage_key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO briefing_audio (owner_id, local_date, storage_key) VALUES ($1, $2, $3) \
         ON CONFLICT (owner_id, local_date) DO UPDATE SET storage_key = EXCLUDED.storage_key, \
         created_at = now()",
        owner_id.as_uuid(),
        local_date,
        storage_key
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes the owner's rows dated before `cutoff` and returns their keys.
pub async fn delete_before(
    conn: impl PgExecutor<'_>,
    owner_id: &AccountId,
    cutoff: NaiveDate,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "DELETE FROM briefing_audio WHERE owner_id = $1 AND local_date < $2 \
         RETURNING storage_key",
        owner_id.as_uuid(),
        cutoff
    )
    .fetch_all(conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with(name: &str) -> (sqlx::PgPool, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(name);
        crate::db::pool::seed_account(&pool, &owner).await;
        (pool, owner)
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[tokio::test]
    async fn upsert_replaces_the_key_for_the_same_day() {
        let (pool, owner) = pool_with("acct_baudio_upsert").await;
        upsert(&pool, &owner, day(1), "k1").await.unwrap();
        upsert(&pool, &owner, day(1), "k2").await.unwrap();
        assert_eq!(
            find_key(&pool, &owner, day(1)).await.unwrap().as_deref(),
            Some("k2")
        );
    }

    #[tokio::test]
    async fn rows_are_isolated_per_owner() {
        let (pool, alice) = pool_with("acct_baudio_alice").await;
        let bob = AccountId::from_seed("acct_baudio_bob");
        crate::db::pool::seed_account(&pool, &bob).await;
        upsert(&pool, &alice, day(1), "alice-key").await.unwrap();
        assert_eq!(find_key(&pool, &bob, day(1)).await.unwrap(), None);
        assert!(delete_before(&pool, &bob, day(30))
            .await
            .unwrap()
            .is_empty());
        assert!(find_key(&pool, &alice, day(1)).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_before_returns_only_older_keys() {
        let (pool, owner) = pool_with("acct_baudio_prune").await;
        upsert(&pool, &owner, day(1), "old").await.unwrap();
        upsert(&pool, &owner, day(20), "new").await.unwrap();
        let gone = delete_before(&pool, &owner, day(10)).await.unwrap();
        assert_eq!(gone, ["old"]);
        assert!(find_key(&pool, &owner, day(20)).await.unwrap().is_some());
    }
}
