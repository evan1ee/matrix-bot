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
            sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM _sqlx_migrations"#)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(applied, migrator().iter().count() as i64);

        let todos: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM todos"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(todos, 0);
    }

    /// Re-running migrations must not disturb rows that already exist.
    #[tokio::test]
    async fn test_migrations_preserve_existing_data() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        sqlx::query!(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', '@a:x', 'a', 'keep me', 'medium', FALSE)"
        )
        .execute(&pool)
        .await
        .unwrap();

        run_migrations_unlocked(&pool).await;

        let text: String = sqlx::query_scalar!("SELECT text FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(text, "keep me");
    }

    /// The unique index is what stops a user ending up with two categories of
    /// the same name; the repository relies on it rather than a read-then-write.
    #[tokio::test]
    async fn test_category_name_uniqueness_is_enforced_by_the_schema() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        sqlx::query!("INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')")
            .execute(&pool)
            .await
            .unwrap();

        let duplicate =
            sqlx::query!("INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')")
                .execute(&pool)
                .await;
        assert!(
            duplicate.is_err(),
            "a duplicate name must be rejected by the index"
        );

        // A different owner may reuse the name.
        sqlx::query!("INSERT INTO finance_categories (owner_id, name) VALUES ('@b:x', 'food')")
            .execute(&pool)
            .await
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

        sqlx::query!(
            "INSERT INTO portfolio_audit_log (owner_id, action, amount_minor, currency) \
             VALUES ($1, $2, $3, $4)",
            "acct_a",
            "buy",
            1_000_i64,
            "USD"
        )
        .execute(&pool)
        .await
        .expect("an audit row must be insertable");

        assert!(
            sqlx::query!("UPDATE portfolio_audit_log SET owner_id = 'acct_b'")
                .execute(&pool)
                .await
                .is_err(),
            "PostgreSQL must refuse to update an audit row"
        );
        assert!(
            sqlx::query!("DELETE FROM portfolio_audit_log")
                .execute(&pool)
                .await
                .is_err(),
            "PostgreSQL must refuse to delete an audit row"
        );

        crate::db::pool::drop_pg_schema("trigger_check").await;
    }

    /// sqlx hashes the raw bytes of each migration file to detect drift. A
    /// CRLF checkout would record a different checksum than an LF checkout
    /// of the same migration, producing `VersionMismatch` at boot on
    /// whichever platform didn't create the applied row. `.gitattributes`
    /// pins these files to LF; this test catches a future migration that
    /// slips past that (e.g. added with `core.autocrlf` misconfigured).
    #[test]
    fn test_no_migration_file_contains_a_carriage_return() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).expect("read migrations dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read migration file");
            assert!(
                !bytes.contains(&b'\r'),
                "{} contains a carriage return; migrations must be LF-only \
                 (see .gitattributes)",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "expected at least one migration file to check");
    }
}
