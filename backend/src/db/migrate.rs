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

        let owner = crate::domain::accounts::AccountId::from_seed("acct_a");
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'a', 'active') ON CONFLICT DO NOTHING",
            owner.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', $1, 'a', 'keep me', 'medium', FALSE)",
            owner.as_uuid()
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

        let owner_a = crate::domain::accounts::AccountId::from_seed("acct_a");
        let owner_b = crate::domain::accounts::AccountId::from_seed("acct_b");
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'a', 'active'), ($2, 'b', 'active') ON CONFLICT DO NOTHING",
            owner_a.as_uuid(),
            owner_b.as_uuid(),
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO finance_categories (owner_id, name) VALUES ($1, 'food')",
            owner_a.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let duplicate = sqlx::query!(
            "INSERT INTO finance_categories (owner_id, name) VALUES ($1, 'food')",
            owner_a.as_uuid()
        )
        .execute(&pool)
        .await;
        assert!(
            duplicate.is_err(),
            "a duplicate name must be rejected by the index"
        );

        // A different owner may reuse the name.
        sqlx::query!(
            "INSERT INTO finance_categories (owner_id, name) VALUES ($1, 'food')",
            owner_b.as_uuid()
        )
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

        let owner_a = crate::domain::accounts::AccountId::from_seed("acct_a");
        let owner_b = crate::domain::accounts::AccountId::from_seed("acct_b");
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'a', 'active'), ($2, 'b', 'active') ON CONFLICT DO NOTHING",
            owner_a.as_uuid(),
            owner_b.as_uuid(),
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO portfolio_audit_log (owner_id, action, amount_minor, currency) \
             VALUES ($1, $2, $3, $4)",
            owner_a.as_uuid(),
            "buy",
            1_000_i64,
            "USD"
        )
        .execute(&pool)
        .await
        .expect("an audit row must be insertable");

        assert!(
            sqlx::query!(
                "UPDATE portfolio_audit_log SET owner_id = $1",
                owner_b.as_uuid()
            )
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

    #[tokio::test]
    async fn test_all_account_and_owner_id_columns_are_uuid() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        let tables_and_cols = [
            ("accounts", "id"),
            ("user_settings", "owner_id"),
            ("webhooks", "owner_id"),
            ("webhook_outbox", "owner_id"),
            ("ai_user_configs", "owner_id"),
            ("ai_conversations", "owner_id"),
            ("portfolio_holdings", "owner_id"),
            ("portfolio_cash_balances", "owner_id"),
            ("portfolio_daily_snapshots", "owner_id"),
            ("portfolio_price_alerts", "owner_id"),
            ("portfolio_audit_log", "owner_id"),
            ("footprints", "owner_id"),
            ("habits", "owner_id"),
            ("habit_logs", "owner_id"),
            ("life_day_logs", "owner_id"),
            ("life_profiles", "owner_id"),
            ("bookmarks", "owner_id"),
            ("todos", "owner_id"),
            ("reminders", "owner_id"),
            ("finance_categories", "owner_id"),
            ("transactions", "owner_id"),
            ("finance_shares", "owner_id"),
            ("recurring_bills", "owner_id"),
            ("bot_message_targets", "owner_id"),
            ("matrix_links", "account_id"),
            ("webauthn_credentials", "account_id"),
            ("sessions", "account_id"),
            ("matrix_link_challenges", "account_id"),
            ("passkey_enrollment_tokens", "account_id"),
            ("recovery_codes", "account_id"),
            ("recovery_requests", "account_id"),
            ("security_events", "account_id"),
            ("web_users", "account_id"),
            ("mcp_tokens", "account_id"),
            ("account_rooms", "account_id"),
            ("jobs", "account_id"),
        ];

        for (table, col) in tables_and_cols {
            let data_type: Option<String> = sqlx::query_scalar!(
                "SELECT data_type FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
                table,
                col
            )
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("failed checking {table}.{col}: {e}"));

            assert_eq!(
                data_type.as_deref(),
                Some("uuid"),
                "{table}.{col} was expected to have type uuid, found {:?}",
                data_type
            );
        }
    }

    /// The migrator restricted to versions up to `version`, so a test can build
    /// the schema a production database had *before* a later migration.
    fn migrator_up_to(version: i64) -> sqlx::migrate::Migrator {
        let mut m = migrator();
        let kept: Vec<_> = m
            .migrations
            .iter()
            .filter(|migration| migration.version <= version)
            .cloned()
            .collect();
        m.migrations = std::borrow::Cow::Owned(kept);
        m.set_locking(false);
        m
    }

    const LEGACY_A: &str = "acct_26ab01c0f4c19b7d854f46dca154ee89";
    const LEGACY_A_UUID: &str = "26ab01c0-f4c1-9b7d-854f-46dca154ee89";
    const LEGACY_B: &str = "acct_00000000000000000000000000000002";

    /// 0002 rewrites the type of every account-keyed column of a live database.
    ///
    /// Runtime `sqlx::query` on purpose: the compile-checked macros only see the
    /// *current* schema, and this test needs the pre-0002 (TEXT) one.
    #[tokio::test]
    async fn test_0002_converts_legacy_text_ids_in_place() {
        let pool = crate::db::pool::test_pool().await;
        migrator_up_to(1).run(&pool).await.expect("baseline");

        for (id, name) in [(LEGACY_A, "a"), (LEGACY_B, "b")] {
            sqlx::query("INSERT INTO accounts (id, display_name) VALUES ($1, $2)")
                .bind(id)
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text) \
             VALUES ('web', $1, 'a', 'keep me')",
        )
        .bind(LEGACY_A)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO matrix_links (mxid, account_id) VALUES ('@a:x', $1)")
            .bind(LEGACY_A)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sessions (id_hash, account_id, created_at, last_seen_at, last_verified_at) \
             VALUES ('h', $1, NOW(), NOW(), NOW())",
        )
        .bind(LEGACY_A)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO web_users (username, password_hash, account_id) \
             VALUES ('linked', 'x', $1), ('loose', 'x', NULL)",
        )
        .bind(LEGACY_A)
        .execute(&pool)
        .await
        .unwrap();

        migrator_up_to(i64::MAX).run(&pool).await.expect("0002");

        for (label, query) in [
            ("todos.owner_id", "SELECT owner_id::text FROM todos"),
            (
                "matrix_links.account_id",
                "SELECT account_id::text FROM matrix_links",
            ),
            (
                "sessions.account_id",
                "SELECT account_id::text FROM sessions",
            ),
        ] {
            let value: String = sqlx::query_scalar(query).fetch_one(&pool).await.unwrap();
            assert_eq!(value, LEGACY_A_UUID, "{label}");
        }
        let linked: Vec<Option<String>> =
            sqlx::query_scalar("SELECT account_id::text FROM web_users ORDER BY username")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(linked, vec![Some(LEGACY_A_UUID.to_string()), None]);

        // The new foreign key refuses an owner that has no account...
        let stray = sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text) \
             VALUES ('web', gen_random_uuid(), 'x', 'x')",
        )
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(
            stray.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("23503")
        );

        // ...and deleting an account takes its rows with it, but only unlinks a
        // web login, which is a credential rather than owned data.
        sqlx::query("DELETE FROM accounts WHERE id = $1::uuid")
            .bind(LEGACY_A_UUID)
            .execute(&pool)
            .await
            .unwrap();
        for (table, query) in [
            ("todos", "SELECT COUNT(*) FROM todos"),
            ("matrix_links", "SELECT COUNT(*) FROM matrix_links"),
            ("sessions", "SELECT COUNT(*) FROM sessions"),
        ] {
            let left: i64 = sqlx::query_scalar(query).fetch_one(&pool).await.unwrap();
            assert_eq!(left, 0, "{table} must cascade");
        }
        let logins: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM web_users WHERE account_id IS NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(logins, 2);
    }

    /// A row 0002 cannot convert must abort the whole migration, naming what is
    /// wrong, and leave the database exactly as it was.
    #[tokio::test]
    async fn test_0002_refuses_data_it_cannot_convert_and_changes_nothing() {
        for (setup, expected) in [
            (
                "INSERT INTO accounts (id, display_name) VALUES ('@evan:mbm.one', 'legacy')",
                "accounts.id contains non-UUID",
            ),
            (
                "INSERT INTO todos (room_id, owner_id, sender_name, text) \
                 VALUES ('web', 'acct_00000000000000000000000000000009', 'x', 'orphan')",
                "todos column owner_id contains 1 orphaned",
            ),
        ] {
            let pool = crate::db::pool::test_pool().await;
            migrator_up_to(1).run(&pool).await.expect("baseline");
            sqlx::query(setup).execute(&pool).await.unwrap();

            let err = migrator_up_to(i64::MAX).run(&pool).await.unwrap_err();
            assert!(err.to_string().contains(expected), "{err}");

            let id_type: String = sqlx::query_scalar(
                "SELECT data_type FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = 'accounts' \
                   AND column_name = 'id'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(id_type, "text", "a refused migration must roll back");
        }
    }

    /// No table may keep a TEXT `owner_id` / `account_id`, or lack the foreign
    /// key, because the migration's own table list forgot it. Reads the catalog
    /// rather than repeating that list.
    #[tokio::test]
    async fn test_every_owner_and_account_column_is_a_uuid_with_a_foreign_key() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        let columns: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT table_name::text, column_name::text, data_type::text \
             FROM information_schema.columns \
             WHERE table_schema = current_schema() \
               AND column_name IN ('owner_id', 'account_id') ORDER BY 1, 2",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(columns.len() >= 34, "found only {} columns", columns.len());

        for (table, column, data_type) in &columns {
            assert_eq!(data_type, "uuid", "{table}.{column}");
            let referenced: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pg_constraint con \
                 JOIN pg_class rel ON rel.oid = con.conrelid \
                 JOIN pg_attribute att ON att.attrelid = rel.oid AND att.attnum = ANY (con.conkey) \
                 WHERE con.contype = 'f' AND con.confrelid = 'accounts'::regclass \
                   AND rel.relnamespace = current_schema()::regnamespace \
                   AND rel.relname = $1 AND att.attname = $2",
            )
            .bind(table)
            .bind(column)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(
                referenced > 0,
                "{table}.{column} has no foreign key to accounts(id)"
            );
        }
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

    /// Given a migrated schema, When a job row is inserted twice with the same
    /// (job_type, dedupe_key), Then the second insert is a no-op; and an
    /// un-keyed job never conflicts. Runtime `sqlx::query` on purpose: this
    /// pins the DDL itself, not a repository.
    #[tokio::test]
    async fn test_0003_jobs_table_dedupes_on_type_and_key() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations_unlocked(&pool).await;

        for _ in 0..2 {
            sqlx::query(
                "INSERT INTO jobs (job_type, dedupe_key) VALUES ('t', 'k') \
                 ON CONFLICT (job_type, dedupe_key) DO NOTHING",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO jobs (job_type) VALUES ('t')")
                .execute(&pool)
                .await
                .unwrap();
        }
        let (keyed, unkeyed): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE dedupe_key IS NOT NULL), \
                    COUNT(*) FILTER (WHERE dedupe_key IS NULL) FROM jobs",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((keyed, unkeyed), (1, 2));

        let defaults: (String, i32, i32, i32) = sqlx::query_as(
            "SELECT status, attempts, max_attempts, backoff_seconds FROM jobs LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(defaults, ("pending".to_string(), 0, 3, 10));

        let bad = sqlx::query("INSERT INTO jobs (job_type, status) VALUES ('t', 'bogus')")
            .execute(&pool)
            .await;
        assert!(bad.is_err(), "the status CHECK must reject unknown states");
    }
}
