//! PostgreSQL adapters shared across interface boundaries.
//!
//! Provides repositories, read models, and dynamic table-level merge operations.

pub mod job_notifications;
pub mod merge;
pub mod read_models;
pub mod repositories;

use sqlx::PgPool;

/// A trivial round-trip query: succeeds only if the pool can still reach the
/// database. The caller (the readiness probe) is expected to bound this with
/// a timeout of its own — a hung connection must not hang `/health/ready`.
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT 1 AS "ok!""#)
        .fetch_one(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ping_succeeds_against_a_live_pool() {
        let pool = crate::db::pool::test_pool().await;

        assert!(ping(&pool).await.is_ok());
    }

    #[tokio::test]
    async fn ping_fails_against_a_closed_pool() {
        let pool = crate::db::pool::test_pool().await;
        pool.close().await;

        assert!(ping(&pool).await.is_err());
    }
}
