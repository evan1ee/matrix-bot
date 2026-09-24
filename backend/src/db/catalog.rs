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

    /// The baseline migration must build exactly what the legacy
    /// `schema*.rs` runner builds, or the live database — built by the legacy
    /// runner — would not match what a fresh deployment gets.
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_live_postgres_baseline_matches_legacy_schema() {
        let legacy = crate::db::pool::connect_pg_schema("baseline_legacy").await;
        crate::db::schema::create_schema(&legacy).await.unwrap();

        let fresh = crate::db::pool::connect_pg_schema("baseline_fresh").await;
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.set_locking(false);
        migrator.run(&fresh).await.unwrap();

        let a = snapshot(&legacy, "baseline_legacy").await;
        let b = snapshot(&fresh, "baseline_fresh").await;
        assert!(
            a.columns.len() > 100,
            "snapshot looks empty: {}",
            a.columns.len()
        );
        assert_eq!(a, b, "baseline differs from legacy schema:\n{}", a.diff(&b));

        // Applying the baseline over an already-built legacy schema (the live
        // database's situation) must succeed and change nothing.
        migrator.run(&legacy).await.unwrap();
        let a2 = snapshot(&legacy, "baseline_legacy").await;
        assert_eq!(
            a,
            a2,
            "baseline is not a no-op on a legacy database:\n{}",
            a.diff(&a2)
        );

        crate::db::pool::drop_pg_schema("baseline_legacy").await;
        crate::db::pool::drop_pg_schema("baseline_fresh").await;
    }
}
