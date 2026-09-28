//! A comparable snapshot of one PostgreSQL schema's structure, for tests that
//! prove two ways of building the schema produce the same result.

use std::collections::BTreeSet;

use crate::db::DbPool;

#[derive(Debug, PartialEq, Eq)]
pub struct SchemaSnapshot {
    pub columns: BTreeSet<String>,
    pub indexes: BTreeSet<String>,
    pub constraints: BTreeSet<String>,
    pub triggers: BTreeSet<String>,
    pub functions: BTreeSet<String>,
}

async fn strings(pool: &DbPool, sql: &str, schema: &str) -> BTreeSet<String> {
    crate::db_scalar_all!(pool, String, sql, schema)
        .expect("catalog query")
        .into_iter()
        // Index and constraint definitions embed the schema name; strip it so
        // two schemas built the same way compare equal.
        .map(|s| s.replace(&format!("{schema}."), ""))
        .collect()
}

pub async fn snapshot(pool: &DbPool, schema: &str) -> SchemaSnapshot {
    SchemaSnapshot {
        columns: strings(
            pool,
            "SELECT table_name || '.' || column_name || ' ' || data_type || \
                    ' null=' || is_nullable || ' default=' || COALESCE(column_default, '') \
             FROM information_schema.columns WHERE table_schema = ? \
               AND table_name <> '_sqlx_migrations'",
            schema,
        )
        .await,
        indexes: strings(
            pool,
            "SELECT indexdef FROM pg_indexes WHERE schemaname = ? \
               AND tablename <> '_sqlx_migrations'",
            schema,
        )
        .await,
        constraints: strings(
            pool,
            "SELECT c.conrelid::regclass::text || ' ' || pg_get_constraintdef(c.oid) \
             FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace \
             WHERE n.nspname = ? AND c.conrelid::regclass::text NOT LIKE '%_sqlx_migrations'",
            schema,
        )
        .await,
        triggers: strings(
            pool,
            "SELECT event_object_table || ' ' || trigger_name || ' ' || action_timing || ' ' || \
                    event_manipulation || ' ' || action_statement \
             FROM information_schema.triggers WHERE trigger_schema = ?",
            schema,
        )
        .await,
        functions: strings(
            pool,
            "SELECT p.proname || ' ' || md5(p.prosrc) FROM pg_proc p \
             JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = ?",
            schema,
        )
        .await,
    }
}

impl SchemaSnapshot {
    /// Lines only in `self` prefixed `-`, only in `other` prefixed `+`.
    pub fn diff(&self, other: &Self) -> String {
        let mut out = String::new();
        for (name, a, b) in [
            ("columns", &self.columns, &other.columns),
            ("indexes", &self.indexes, &other.indexes),
            ("constraints", &self.constraints, &other.constraints),
            ("triggers", &self.triggers, &other.triggers),
            ("functions", &self.functions, &other.functions),
        ] {
            for line in a.difference(b) {
                out.push_str(&format!("- {name}: {line}\n"));
            }
            for line in b.difference(a) {
                out.push_str(&format!("+ {name}: {line}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rehearses the migrations against a restored copy of production.
    ///
    /// ```text
    /// pg_dump -Fc "$PROD_URL" > prod.dump
    /// createdb -h localhost -U postgres rehearsal
    /// pg_restore -h localhost -U postgres -d rehearsal --no-owner prod.dump
    /// MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
    /// PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
    ///   cargo test -p matrix-bot migrates_a_real_database -- --ignored --nocapture
    /// ```
    ///
    /// Never point it at production itself: it applies the migrations.
    #[tokio::test]
    #[ignore = "needs MIGRATE_TEST_DATABASE_URL pointing at a *copy* of production"]
    async fn test_migrates_a_real_database() {
        let url = std::env::var("MIGRATE_TEST_DATABASE_URL")
            .expect("set MIGRATE_TEST_DATABASE_URL to a restored copy of production");
        let copy = crate::db::pool::connect_pool(&url, 2).await.unwrap();

        let tables: Vec<String> = crate::db_scalar_all!(
            &copy,
            String,
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
               AND table_name <> '_sqlx_migrations'"
        )
        .unwrap();
        let mut before = std::collections::BTreeMap::new();
        for t in &tables {
            let n: i64 =
                crate::db_scalar_one!(&copy, _, &format!("SELECT COUNT(*) FROM {t}")).unwrap();
            before.insert(t.clone(), n);
        }

        crate::db::run_migrations(&copy)
            .await
            .expect("migrations apply to production copy");

        for (t, n) in &before {
            let after: i64 =
                crate::db_scalar_one!(&copy, _, &format!("SELECT COUNT(*) FROM {t}")).unwrap();
            assert_eq!(*n, after, "row count changed in {t}");
        }

        // Production must now have exactly the structure a fresh deployment gets.
        let fresh = crate::db::pool::connect_pg_schema("rehearsal_fresh").await;
        crate::db::migrate::run_migrations_unlocked(&fresh).await;
        let prod = snapshot(&copy, "public").await;
        let want = snapshot(&fresh, "rehearsal_fresh").await;
        assert_eq!(
            prod,
            want,
            "production schema drifted from the migrations:\n{}",
            want.diff(&prod)
        );
        crate::db::pool::drop_pg_schema("rehearsal_fresh").await;

        // Every audit chain still verifies.
        let owners: Vec<String> = crate::db_scalar_all!(
            &copy,
            String,
            "SELECT DISTINCT owner_id FROM portfolio_audit_log"
        )
        .unwrap();
        for owner in owners {
            let result = crate::modules::portfolio::audit::verify_audit_chain(&copy, &owner)
                .await
                .unwrap();
            assert!(
                result.is_valid,
                "audit chain broken for {owner}: {result:?}"
            );
        }
    }
}
