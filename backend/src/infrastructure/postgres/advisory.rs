//! Session advisory locks for leader election.
//!
//! A session lock belongs to the connection that took it. Taken on a pooled
//! connection that later went back to the pool, it would stay held by
//! whichever caller borrowed that connection next, and never be released.
//! So the lock gets its own connection, detached from the pool
//! (`PoolConnection::detach`; the pool opens a replacement), and the lock
//! lives exactly as long as that connection.

use sqlx::{Connection, PgConnection, PgPool};

pub struct LeaderLock {
    conn: PgConnection,
    key: i64,
}

impl LeaderLock {
    /// `Some` while this session holds `pg_try_advisory_lock(key)`.
    pub async fn try_acquire(pool: &PgPool, key: i64) -> sqlx::Result<Option<Self>> {
        let mut conn = pool.acquire().await?.detach();
        let locked = sqlx::query_scalar!(r#"SELECT pg_try_advisory_lock($1) AS "locked!""#, key)
            .fetch_one(&mut conn)
            .await;
        match locked {
            Ok(true) => Ok(Some(Self { conn, key })),
            Ok(false) => {
                let _ = conn.close().await;
                Ok(None)
            }
            Err(err) => {
                let _ = conn.close().await;
                Err(err)
            }
        }
    }

    /// `false` once the lock's connection is gone (the server already
    /// released the lock).
    pub async fn is_alive(&mut self) -> bool {
        self.conn.ping().await.is_ok()
    }

    /// Unlocks and closes the connection (best effort; closing alone also
    /// releases).
    pub async fn release(mut self) {
        let _ = sqlx::query_scalar!(r#"SELECT pg_advisory_unlock($1) AS "unlocked!""#, self.key)
            .fetch_one(&mut self.conn)
            .await;
        let _ = self.conn.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> i64 {
        uuid::Uuid::new_v4().as_u64_pair().0 as i64
    }

    /// Given one holder, When a second session tries, Then it gets None;
    /// after release the second succeeds.
    #[tokio::test]
    async fn only_one_session_holds_the_lock() {
        let pool = crate::db::pool::test_pool().await;
        let k = key();
        let first = LeaderLock::try_acquire(&pool, k)
            .await
            .unwrap()
            .expect("first acquires");
        assert!(LeaderLock::try_acquire(&pool, k).await.unwrap().is_none());
        first.release().await;
        let second = LeaderLock::try_acquire(&pool, k).await.unwrap();
        assert!(second.is_some());
        second.unwrap().release().await;
    }

    /// Given a held lock, When the LeaderLock is dropped without release,
    /// Then the lock becomes free (its connection closed, never pooled).
    #[tokio::test]
    async fn dropping_the_lock_frees_it() {
        let pool = crate::db::pool::test_pool().await;
        let k = key();
        drop(LeaderLock::try_acquire(&pool, k).await.unwrap().unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(l) = LeaderLock::try_acquire(&pool, k).await.unwrap() {
                l.release().await;
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "lock still held 5 s after drop"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Given a lock, When pooled connections run other queries, Then none of
    /// them holds the lock (the lock connection is detached from the pool).
    #[tokio::test]
    async fn the_lock_connection_never_returns_to_the_pool() {
        let pool = crate::db::pool::test_pool().await; // max 2
        let k = key();
        let mut lock = LeaderLock::try_acquire(&pool, k).await.unwrap().unwrap();
        for _ in 0..2 {
            let mine: bool = sqlx::query_scalar!(r#"SELECT pg_try_advisory_lock($1) AS "l!""#, k)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(!mine);
        }
        assert!(lock.is_alive().await);
        lock.release().await;
    }

    /// Given a held lock, When its backend is terminated server-side, Then
    /// `is_alive` reports false (leadership is dropped).
    #[tokio::test]
    async fn a_lost_connection_is_reported_dead() {
        let pool = crate::db::pool::test_pool().await;
        let k = key();
        let mut lock = LeaderLock::try_acquire(&pool, k).await.unwrap().unwrap();
        let killed: i64 = sqlx::query_scalar!(
            r#"SELECT count(pg_terminate_backend(pid)) AS "n!" FROM pg_locks
               WHERE locktype = 'advisory' AND granted
                 AND objid = ($1::bigint & 4294967295)::oid"#,
            k
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(killed, 1, "exactly the lock holder is terminated");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while lock.is_alive().await {
            assert!(std::time::Instant::now() < deadline, "still alive");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
