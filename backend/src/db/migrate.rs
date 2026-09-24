//! Versioned, forward-only migrations from `backend/migrations/`.

use crate::db::DbPool;

pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}

/// Applied at boot, before any task starts. sqlx serialises concurrent runs
/// with an advisory lock, so two instances booting together are safe.
pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError> {
    tracing::info!("Applying database migrations");
    migrator().run(pool).await
}

/// For tests: each test owns a private schema, so the database-wide advisory
/// lock would only serialise the parallel suite for nothing.
#[cfg(test)]
pub async fn run_migrations_unlocked(pool: &DbPool) {
    let mut m = migrator();
    m.set_locking(false);
    m.run(pool).await.expect("migrations");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_migrations_create_the_schema_and_are_idempotent() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;
        run_migrations_unlocked(&pool).await;

        let applied: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM _sqlx_migrations").unwrap();
        assert_eq!(applied, migrator().iter().count() as i64);

        let todos: i64 = crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM todos").unwrap();
        assert_eq!(todos, 0);
    }

    /// Re-running migrations must not disturb rows that already exist.
    #[tokio::test]
    async fn test_migrations_preserve_existing_data() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        crate::db_execute!(
            &pool,
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', '@a:x', 'a', 'keep me', 'medium', FALSE)"
        )
        .unwrap();

        run_migrations_unlocked(&pool).await;

        let text: String = crate::db_scalar_one!(&pool, _, "SELECT text FROM todos").unwrap();
        assert_eq!(text, "keep me");
    }

    /// The unique index is what stops a user ending up with two categories of
    /// the same name; the repository relies on it rather than a read-then-write.
    #[tokio::test]
    async fn test_category_name_uniqueness_is_enforced_by_the_schema() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        crate::db_execute!(
            &pool,
            "INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')"
        )
        .unwrap();

        let duplicate = crate::db_execute!(
            &pool,
            "INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')"
        );
        assert!(
            duplicate.is_err(),
            "a duplicate name must be rejected by the index"
        );

        // A different owner may reuse the name.
        crate::db_execute!(
            &pool,
            "INSERT INTO finance_categories (owner_id, name) VALUES ('@b:x', 'food')"
        )
        .unwrap();
    }

    /// The audit log must be append-only on PostgreSQL too, not merely have
    /// triggers created. `accounts/merge.rs` depends on the UPDATE guard
    /// firing, so a trigger that exists but does not abort would be worse than
    /// none at all.
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_the_audit_log_is_append_only_on_a_live_postgres() {
        let pool = crate::db::pool::connect_pg_schema("trigger_check").await;
        run_migrations_unlocked(&pool).await;

        crate::db_execute!(
            &pool,
            "INSERT INTO portfolio_audit_log (owner_id, action, amount_minor, currency) \
             VALUES (?, ?, ?, ?)",
            "acct_a",
            "buy",
            1_000_i64,
            "USD"
        )
        .expect("an audit row must be insertable");

        assert!(
            crate::db_execute!(&pool, "UPDATE portfolio_audit_log SET owner_id = 'acct_b'")
                .is_err(),
            "PostgreSQL must refuse to update an audit row"
        );
        assert!(
            crate::db_execute!(&pool, "DELETE FROM portfolio_audit_log").is_err(),
            "PostgreSQL must refuse to delete an audit row"
        );

        crate::db::pool::drop_pg_schema("trigger_check").await;
    }
}
