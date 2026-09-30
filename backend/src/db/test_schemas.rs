//! Keeps the per-test schemas [`test_pool`](super::pool::test_pool) creates
//! from piling up on the test server.
//!
//! A schema is named `t<pid>_<n>` after the test process that created it. That
//! process holds a session-level advisory lock keyed on its pid for as long as
//! it lives, on a connection of its own; the server releases the lock when the
//! process exits, however it exits. So "is the owner still running?" is a
//! question PostgreSQL answers from `pg_locks`, with no platform-specific pid
//! probing, and it stays correct when the server runs in a container.
//!
//! Each process reaps the schemas of owners that no longer hold their lock once
//! it has taken its own. A schema therefore outlives the run that made it —
//! a failing test still leaves its rows to inspect — but only until the next
//! test process starts, which bounds the leftovers to one run's worth.

use sqlx::{Connection, PgConnection, Postgres, QueryBuilder};
use std::sync::OnceLock;

/// First half of the two-key advisory lock; the second half is the pid. Any
/// fixed value works as long as nothing else in the test database uses it.
const LOCK_CLASS: i32 = 0x7473_6368; // "tsch"

pub fn schema_name(pid: u32, n: u64) -> String {
    format!("t{pid}_{n}")
}

/// Takes this process's liveness lock on first call and keeps it until the
/// process exits, then reaps orphaned schemas in the background. Later calls
/// return at once.
///
/// The lock lives on a dedicated thread with its own runtime: a connection
/// opened inside a `#[tokio::test]` dies with that test's runtime, which would
/// release the lock while sibling tests still use their schemas.
pub fn hold_liveness_lock(database_url: &str) {
    static HELD: OnceLock<()> = OnceLock::new();
    HELD.get_or_init(|| {
        let (ready, locked) = std::sync::mpsc::channel::<Result<(), String>>();
        let url = database_url.to_owned();
        std::thread::Builder::new()
            .name("test-schema-lock".into())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("test-schema-lock runtime")
                    .block_on(hold_and_reap(url, ready));
            })
            .expect("spawn test-schema-lock thread");
        locked
            .recv()
            .expect("test-schema-lock thread exited before locking")
            .unwrap_or_else(|e| panic!("test schema liveness lock: {e}"));
    });
}

async fn hold_and_reap(url: String, ready: std::sync::mpsc::Sender<Result<(), String>>) {
    let mut conn = match PgConnection::connect(&url).await {
        Ok(conn) => conn,
        Err(e) => {
            let _ = ready.send(Err(format!("connect: {e}")));
            return;
        }
    };
    let locked = match try_lock(&mut conn, std::process::id()).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "another live test process already holds pid {}'s lock",
            std::process::id()
        )),
        Err(e) => Err(format!("lock: {e}")),
    };
    let failed = locked.is_err();
    let _ = ready.send(locked);
    if failed {
        return;
    }

    if let Err(e) = reap_orphaned(&mut conn).await {
        eprintln!("test schema cleanup: could not list orphaned schemas: {e}");
    }
    // `conn` stays in scope, and with it the lock, until the process exits.
    std::future::pending::<()>().await;
}

/// Takes the liveness lock for `pid` on `conn`, for as long as `conn` stays open.
pub async fn try_lock(conn: &mut PgConnection, pid: u32) -> sqlx::Result<bool> {
    // The pid is carried bit-for-bit in an int4; `pg_locks.objid` shows it back
    // as the unsigned value, which is what `reap_orphaned` compares against.
    QueryBuilder::<Postgres>::new("SELECT pg_try_advisory_lock(")
        .push_bind(LOCK_CLASS)
        .push(", ")
        .push_bind(pid as i32)
        .push(")")
        .build_query_scalar::<bool>()
        .fetch_one(conn)
        .await
}

/// Drops every `t<pid>_<n>` schema whose pid holds no liveness lock, and
/// returns the ones it dropped. A schema that fails to drop — typically one a
/// concurrent reaper got to first — is reported and skipped.
pub async fn reap_orphaned(conn: &mut PgConnection) -> sqlx::Result<Vec<String>> {
    let orphans: Vec<String> = QueryBuilder::<Postgres>::new(
        "SELECT n.nspname::text FROM pg_namespace n \
         WHERE n.nspname ~ '^t[0-9]+_[0-9]+$' \
           AND NOT EXISTS ( \
             SELECT 1 FROM pg_locks l \
             WHERE l.locktype = 'advisory' AND l.objsubid = 2 AND l.granted \
               AND l.classid::bigint = ",
    )
    .push_bind(i64::from(LOCK_CLASS))
    .push(
        " AND l.objid::bigint = substring(n.nspname FROM '^t([0-9]+)_')::bigint) \
         ORDER BY 1",
    )
    .build_query_scalar()
    .fetch_all(&mut *conn)
    .await?;

    let mut dropped = Vec::with_capacity(orphans.len());
    for schema in orphans {
        match drop_schema(&mut *conn, &schema).await {
            Ok(()) => dropped.push(schema),
            Err(e) => eprintln!("test schema cleanup: could not drop {schema}: {e}"),
        }
    }
    Ok(dropped)
}

/// `DROP SCHEMA IF EXISTS … CASCADE`. An identifier cannot be bound, so the
/// name is checked against the `t<pid>_<n>` shape before it reaches the SQL.
pub async fn drop_schema<'c, E>(conn: E, schema: &str) -> sqlx::Result<()>
where
    E: sqlx::Executor<'c, Database = Postgres>,
{
    assert!(is_test_schema(schema), "not a test schema name: {schema:?}");
    QueryBuilder::<Postgres>::new("DROP SCHEMA IF EXISTS ")
        .push(schema)
        .push(" CASCADE")
        .build()
        .execute(conn)
        .await
        .map(drop)
}

fn is_test_schema(name: &str) -> bool {
    let Some((pid, n)) = name.strip_prefix('t').and_then(|rest| rest.split_once('_')) else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits(pid) && digits(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn schema_exists(conn: &mut PgConnection, schema: &str) -> bool {
        QueryBuilder::<Postgres>::new("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = ")
            .push_bind(schema)
            .push(")")
            .build_query_scalar::<bool>()
            .fetch_one(conn)
            .await
            .unwrap()
    }

    async fn create_schema(conn: &mut PgConnection, schema: &str) {
        drop_schema(&mut *conn, schema).await.unwrap();
        QueryBuilder::<Postgres>::new("CREATE SCHEMA ")
            .push(schema)
            .build()
            .execute(conn)
            .await
            .unwrap();
    }

    #[test]
    fn test_only_test_pool_schema_names_are_recognised() {
        assert!(is_test_schema(&schema_name(1234, 0)));
        assert!(is_test_schema("t4294967295_17"));
        for name in [
            "public",
            "t_1",
            "t1_",
            "t1",
            "tx_1",
            "t1_1; DROP",
            "boot_check",
        ] {
            assert!(!is_test_schema(name), "{name}");
        }
    }

    /// Given schemas left by a live process, a dead process and this one, when
    /// the reaper runs, only the dead process's schema is dropped — and it is
    /// dropped as soon as its owner lets go of the lock.
    #[tokio::test]
    async fn test_reaper_drops_only_schemas_whose_process_is_gone() {
        // A pid no real test process has; above i32::MAX, so this also covers
        // the bit-cast into the int4 lock key.
        const OTHER_PID: u32 = u32::MAX - 7;

        let own_pool = crate::db::pool::test_pool().await;
        let own: String = QueryBuilder::<Postgres>::new("SELECT current_schema()::text")
            .build_query_scalar()
            .fetch_one(&own_pool)
            .await
            .unwrap();

        let url = crate::db::pool::test_database_url();
        let mut reaper = PgConnection::connect(&url).await.unwrap();
        let mut other = PgConnection::connect(&url).await.unwrap();
        assert!(
            try_lock(&mut other, OTHER_PID).await.unwrap(),
            "the fake pid's lock must be free"
        );
        let others = schema_name(OTHER_PID, u64::from(std::process::id()));
        create_schema(&mut reaper, &others).await;

        reap_orphaned(&mut reaper).await.unwrap();
        assert!(schema_exists(&mut reaper, &own).await, "own schema reaped");
        assert!(
            schema_exists(&mut reaper, &others).await,
            "a live process's schema was reaped"
        );

        // The "other process" exits: closing its session releases the lock,
        // though the server finishes tearing the backend down asynchronously.
        other.close().await.unwrap();
        let mut reaped = false;
        for _ in 0..50 {
            reap_orphaned(&mut reaper).await.unwrap();
            if !schema_exists(&mut reaper, &others).await {
                reaped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(reaped, "a dead process's schema survived the reaper");
        assert!(schema_exists(&mut reaper, &own).await, "own schema reaped");
    }
}
