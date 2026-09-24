# PostgreSQL-Only Cut & Versioned Migrations — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove SQLite as an application database, move to sqlx 0.9, replace the hand-rolled `schema*.rs` runner with versioned `sqlx::migrate!` migrations that the live PostgreSQL database adopts in place, and stand up the compile-checked-query pipeline (`.sqlx/` offline cache).

**Architecture:** Collapse `DbPool`/`DbTx` to type aliases over `sqlx::PgPool`/`sqlx::Transaction<Postgres>` and shrink the `db_*!`/`tx_*!` macros to a single Postgres arm, so all ~540 call sites keep compiling untouched (they are rewritten to `query!` feature by feature in the next plan). Then upgrade sqlx, then transcribe today's schema into an idempotent `0001_baseline.sql` proved identical to the legacy DDL by a catalog-comparison test, and switch boot to the migrator.

**Tech Stack:** Rust 2021, sqlx 0.7 → 0.9 (postgres, chrono, migrate, macros), PostgreSQL 18+ (CI image `postgres:18`), axum 0.7, matrix-sdk 0.7 (keeps its own SQLite crypto store).

**Spec:** [`docs/superpowers/specs/2026-09-24-postgres-foundation-design.md`](../specs/2026-09-24-postgres-foundation-design.md) (§4, §6 steps 1–3) and roadmap [`SPEC.md`](../../../SPEC.md) §2, §4.3.

## Global Constraints

- The application database is PostgreSQL 18+ only. The **only** SQLite left in the process is matrix-sdk's crypto store (`.sqlite_store(...)` in `backend/src/bot/client.rs`) — do not touch it.
- `DATABASE_URL` is required and must start with `postgres://` or `postgresql://`; anything else refuses to boot with an error naming `DATABASE_URL`.
- Order is forced: **drop the sqlx `sqlite` feature before upgrading to 0.9.** `sqlx-sqlite 0.7` and `matrix-sdk-sqlite 0.7` both link `libsqlite3-sys 0.27`; sqlx 0.9's SQLite driver needs a newer one (and matrix-sdk must move to 0.18, Task 3a) and Cargo cannot resolve both (`links = "sqlite3"`).
- Never edit a migration file after it has been committed to master; add a new one.
- Every task ends with: `cargo fmt --all -- --check`, `cargo check -p matrix-bot --all-targets` warning-free, `TEST_DATABASE_URL=… cargo test -p matrix-bot --all-targets` green, `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`.
- Conventional Commits; end every commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Push to `gitea` (primary) and `origin` (mirror) only when the user asks.
- Local PostgreSQL for all tests (every task):
  ```bash
  docker run --rm -d --name bot-pg -p 5432:5432 -e POSTGRES_PASSWORD=bot postgres:18
  export TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/postgres
  export PG_TEST_URL=$TEST_DATABASE_URL
  ```

### Deviations from the design doc (recorded there in Task 6)

1. Design §6 put the sqlx upgrade before "Postgres-only cut". Reversed, for the `libsqlite3-sys` reason above.
2. `0002_account_uuid` moves out of this plan to **after** the per-feature `query!` rewrite (design step 5). With every query compile-checked, flipping `owner_id` to `UUID` turns every stale bind into a compile error; doing it now would mean hand-auditing ~540 macro call sites and ~100 tests that insert text owners such as `'@a:x'`.
3. Tests keep the existing per-test-schema harness (`db::pool::test_pool`, 110 call sites) instead of `#[sqlx::test]`; it gives the same isolation. It now runs the real migrator with locking disabled.

---

## File Map

| File | Change |
|---|---|
| `backend/Cargo.toml` | Task 1: drop `sqlite` feature. Task 3a: matrix-sdk 0.18. Task 3: sqlx 0.9 |
| `backend/src/db/executor.rs` | Task 1: `DbPool`/`DbTx` become aliases; macros lose the SQLite arm |
| `backend/src/db/dialect.rs` | Task 1: `Dialect` enum removed; keeps only `render_placeholders(sql)` |
| `backend/src/db/dialect_probe.rs` | Task 1: delete |
| `backend/src/db/ddl.rs` | Task 1: PostgreSQL-only; Task 5: delete |
| `backend/src/db/pool.rs` | Task 1: PG-only `connect_pool`, `test_pool`; Task 5: migrator in tests |
| `backend/src/db/backfill.rs` | Task 1: delete SQLite fixture tests; Task 5: delete file |
| `backend/src/db/schema.rs`, `schema_*.rs` | Task 5: delete (replaced by `migrate.rs`) |
| `backend/src/db/migrate.rs` | Task 5: create — `run_migrations`, `migrator()` |
| `backend/src/db/catalog.rs` | Task 4: create (`#[cfg(test)]`) — schema snapshot for comparisons |
| `backend/migrations/0001_baseline.sql` | Task 4: create |
| `backend/src/config.rs` | Task 1: `DATABASE_URL` required + validated |
| `backend/src/accounts/mod.rs` | Task 1: schema-guard tests read `information_schema` |
| `backend/src/api/mcp.rs`, `modules/mcp/service.rs`, `modules/finance/net_worth.rs` + 14 test modules | Task 1: in-memory SQLite → `test_pool()` |
| `backend/src/api/health.rs` | Task 6: first `query_scalar!` |
| `.sqlx/`, `.cargo/config.toml` | Task 6: offline cache + `SQLX_OFFLINE=true` |
| `.github/workflows/ci.yml` | Task 2: one PG backend job; Task 6: `cargo sqlx prepare --check` |
| `docker-compose.yml`, `.env.example` | Task 2: PostgreSQL by default |
| `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` | Tasks 2, 5, 6 |

---

### Task 1: PostgreSQL-only database layer

**Files:**
- Modify: `backend/Cargo.toml:36`
- Modify: `backend/src/db/executor.rs` (whole file), `backend/src/db/dialect.rs`, `backend/src/db/ddl.rs`, `backend/src/db/pool.rs`, `backend/src/db/mod.rs`
- Delete: `backend/src/db/dialect_probe.rs`
- Modify: `backend/src/db/schema_portfolio.rs:84`, `backend/src/modules/portfolio/audit.rs:203,269`
- Modify: `backend/src/config.rs:58,432,478`
- Modify tests: `backend/src/accounts/mod.rs` (~l.230–340), `backend/src/db/schema.rs` tests, `backend/src/db/backfill.rs` tests, `backend/src/api/mcp.rs:215-250`, `backend/src/modules/mcp/service.rs:190-250`, `backend/src/modules/finance/net_worth.rs:235-250`, and every other file listed by `grep -rln "sqlite::memory\|sqlite_master\|skip_on_postgres\|PRAGMA\|SqlitePool" backend/src`

**Interfaces:**
- Produces: `pub type DbPool = sqlx::PgPool;` and `pub type DbTx<'c> = sqlx::Transaction<'c, sqlx::Postgres>;` (re-exported from `crate::db`). All `db_*!`/`tx_*!`/`db_ddl!` macros keep their exact call syntax. `pub fn render_placeholders(sql: &str) -> Cow<'_, str>`. `pub async fn connect_pool(database_url: &str, max_connections: u32) -> anyhow::Result<DbPool>`. `#[cfg(test)] pub async fn test_pool() -> DbPool` (unmigrated, fresh schema). `Config::database_url` is validated.
- Removed: `Dialect`, `DbPool::dialect()`, `DbPool::sqlite_or_err()`, `DbTx::dialect()`, `skip_on_postgres`, `dialect_probe`.

- [ ] **Step 1: Write the failing config test**

In `backend/src/config.rs`'s test module add:

```rust
#[test]
fn test_database_url_must_be_postgres() {
    assert!(validate_database_url("postgres://bot:bot@localhost/matrix_bot").is_ok());
    assert!(validate_database_url("postgresql://bot@db/matrix_bot").is_ok());

    let err = validate_database_url("sqlite://data/bot_data.db?mode=rwc").unwrap_err();
    assert!(err.to_string().contains("DATABASE_URL"), "error must name the variable: {err}");
    assert!(validate_database_url("").is_err());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p matrix-bot test_database_url_must_be_postgres`
Expected: FAIL to compile — `cannot find function validate_database_url`.

- [ ] **Step 3: Implement config validation**

In `backend/src/config.rs`, replace the `DATABASE_URL` read at line 58 (`env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://…".to_string())`) so the value is required and validated, and add the function:

```rust
/// The application database is PostgreSQL only; refuse anything else at boot
/// rather than failing on the first query.
pub fn validate_database_url(url: &str) -> anyhow::Result<()> {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Ok(())
    } else {
        anyhow::bail!(
            "DATABASE_URL must be a PostgreSQL URL (postgres://user:pass@host:5432/db); got {:?}",
            url.split('@').next_back().unwrap_or(url)
        )
    }
}
```

At the read site:

```rust
let database_url = env::var("DATABASE_URL")
    .map_err(|_| anyhow::anyhow!("DATABASE_URL is required (a postgres:// URL)"))?;
validate_database_url(&database_url)?;
```

Adapt to the surrounding function's error type if it is not `anyhow::Result` (look at how `Config::from_env` reports other required variables and do the same). In the two test fixtures at lines ~432 and ~478 replace `"sqlite::memory:"` with `"postgres://test@localhost/test"`.

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p matrix-bot test_database_url_must_be_postgres`
Expected: PASS (the crate still compiles with SQLite at this point).

- [ ] **Step 5: Drop the sqlx `sqlite` feature**

`backend/Cargo.toml:36`:

```toml
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "postgres", "chrono", "migrate"] }
```

- [ ] **Step 6: Rewrite `dialect.rs` to PostgreSQL only**

Delete `enum Dialect` and `Dialect::from_url`. Change the signature to `pub fn render_placeholders(sql: &str) -> Cow<'_, str>`; the early return becomes `if !sql.contains('?') { return Cow::Borrowed(sql); }`. Keep the scanner body unchanged. Update the module doc to: "SQL is still written in `?` style during the migration to `query!`; this rewrites it to `$1..$n`. Deleted with the macros once every repository uses `query!`." Update its existing tests: drop the `Dialect` argument, delete the tests that assert SQLite pass-through.

- [ ] **Step 7: Rewrite `executor.rs`**

Replace the `DbPool` enum, its `impl`, the `DbTx` enum and its `impl` with:

```rust
//! The pool and transaction types, and the `?`-style query macros still used by
//! code not yet converted to compile-checked `query!`. The macros are removed
//! once every repository has moved; new code uses `sqlx::query!` directly.

/// The application's PostgreSQL pool.
pub type DbPool = sqlx::PgPool;

/// A PostgreSQL transaction.
pub type DbTx<'c> = sqlx::Transaction<'c, sqlx::Postgres>;

#[doc(hidden)]
pub use crate::db::dialect::render_placeholders as __render;
```

Rewrite every macro to a single arm. The pattern — bind the pool/tx to an explicitly typed reference first (so `pool`, `&pool` and `&&pool` call sites all coerce), then render, then run:

```rust
#[macro_export]
macro_rules! db_fetch_all {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(__p).await
    }};
}

#[macro_export]
macro_rules! db_execute {
    ($pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query(&__sql)$(.bind($bind))*.execute(__p).await.map(|r| r.rows_affected())
    }};
}

#[macro_export]
macro_rules! db_fetch_all_dyn {
    ($pool:expr, $ty:ty, $sql:expr, $dyn:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        let mut __q = sqlx::query_as::<_, $ty>(&__sql);
        for __v in $dyn {
            __q = __q.bind(__v);
        }
        __q$(.bind($bind))*.fetch_all(__p).await
    }};
}

#[macro_export]
macro_rules! tx_execute {
    ($tx:expr, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query(&__sql)$(.bind($bind))*.execute(&mut **__t).await.map(|r| r.rows_affected())
    }};
}
```

Apply the same shape to the rest, keeping each macro's existing fetch method:
`db_fetch_one` (`query_as` + `fetch_one`), `db_fetch_optional` (`fetch_optional`), `db_scalar_all` / `db_scalar_one` / `db_scalar_optional` (`sqlx::query_scalar::<_, $ty>` + `fetch_all`/`fetch_one`/`fetch_optional` — copy the `$ty` handling from the current arms), `db_fetch_one_dyn` (dyn + `fetch_one`), `tx_fetch_all` / `tx_fetch_one` / `tx_fetch_optional` / `tx_scalar_one` / `tx_scalar_all` / `tx_scalar_optional` (tx shape above with `&mut **__t`). Delete the `sqlite_or_err` test and any test in this file that builds `DbPool::Sqlite`; port the remaining tests to `crate::db::pool::test_pool().await`.

If a call site fails to coerce into `&DbPool` (compile error "expected `&PgPool`, found …"), fix the call site to pass a reference; do not change the macro.

- [ ] **Step 8: Make `ddl.rs` PostgreSQL-only**

- `render(sql)`: drop the `dialect` parameter; `{{PK}}` → `BIGSERIAL PRIMARY KEY`, `{{NOW_TEXT}}` → `(NOW()::text)`.
- `drop_trigger`, `audit_update_trigger`, `audit_delete_trigger`, `drop_audit_update_trigger`, `audit_log_immutability`: drop the `dialect` parameter, keep only the PostgreSQL SQL they currently return for `Dialect::Postgres`.
- `db_ddl!`: `let __p: &$crate::db::DbPool = $pool; sqlx::raw_sql(&$crate::db::ddl::render($sql)).execute(__p).await.map(|_| ())`.
- Update callers: `db/schema_portfolio.rs:84` → `ddl::audit_log_immutability()`, `modules/portfolio/audit.rs:203` → `drop_audit_update_trigger()`, `:269` → `audit_update_trigger()`.
- Delete every test in `ddl.rs` that uses `SqlitePool` or asserts SQLite output; keep the PostgreSQL-output assertions.

- [ ] **Step 9: Rewrite `pool.rs`**

```rust
use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use tracing::info;

use crate::db::executor::DbPool;
use crate::db::schema;

#[derive(Clone, Debug)]
pub struct Database {
    pub pool: DbPool,
}

/// Opens the PostgreSQL pool. Split from `Database::connect` so tests can get a
/// pool without running migrations.
pub async fn connect_pool(database_url: &str, max_connections: u32) -> Result<DbPool> {
    crate::config::validate_database_url(database_url)?;
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(database_url)
        .await
        .context("Failed to connect to PostgreSQL")
}
```

`test_pool()`: delete the SQLite fallback; replace the `let Ok(base) = … else { return …Sqlite… }` with

```rust
let base = std::env::var("TEST_DATABASE_URL").expect(
    "TEST_DATABASE_URL must point at a PostgreSQL server, e.g. \
     postgres://postgres:bot@localhost:5432/postgres (see AGENTS.md)",
);
```

Keep the per-test schema logic. Delete `skip_on_postgres`. Keep `pg_test_url`, `connect_pg_schema`, `drop_pg_schema`. `Database::connect` loses the "Connecting to database at {url}" log of the full URL (it contains the password) — log only `"Connecting to PostgreSQL"`. In `pool.rs` tests delete `test_in_memory_connect_runs_migrations` and `test_creates_the_database_directory_if_missing`; add:

```rust
#[tokio::test]
async fn test_connect_pool_refuses_sqlite() {
    let err = connect_pool("sqlite::memory:", 1).await.unwrap_err();
    assert!(err.to_string().contains("DATABASE_URL"));
}
```

- [ ] **Step 10: Delete `dialect_probe.rs`**

Remove the file and `pub mod dialect_probe;` from `db/mod.rs`. In `db/mod.rs` remove `pub use dialect::Dialect;`.

- [ ] **Step 11: Port the SQLite-specific tests**

Run `grep -rn "sqlite::memory\|sqlite_master\|skip_on_postgres\|PRAGMA\|SqlitePool\|DbPool::Sqlite" backend/src` and handle each hit:

- Pools built as `DbPool::Sqlite(SqlitePoolOptions…)` or `SqlitePool::connect("sqlite::memory:")` → `crate::db::pool::test_pool().await`, followed by the same `run_migrations` call the test made before.
- `accounts/mod.rs` guard tests (`test_owned_tables_and_exceptions_match_the_schema`, `test_account_id_tables_are_all_classified`): remove the `skip_on_postgres` guard and derive tables from the test's own schema:

```rust
async fn tables_with_column(pool: &DbPool, column: &str) -> BTreeSet<String> {
    crate::db_scalar_all!(
        pool,
        String,
        "SELECT table_name FROM information_schema.columns \
         WHERE table_schema = current_schema() AND column_name = ?",
        column
    )
    .unwrap()
    .into_iter()
    .collect()
}
```

Use `tables_with_column(&pool, "owner_id")` / `"account_id"` in place of the `sqlite_master` + `PRAGMA table_info` loop; the assertions stay the same.
- `db/schema.rs` tests `test_creates_every_table` and `test_migrations_are_idempotent`: `table_names` becomes `SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema() ORDER BY table_name`; drop the skip guards.
- `db/backfill.rs`: delete the tests that hand-build a *legacy SQLite* database; keep any test that runs through `test_pool()`. Delete `test_migrates_a_real_database_without_stranding_rows` (its PostgreSQL replacement arrives in Task 5).
- Tests asserting SQLite-only behaviour that has no PostgreSQL meaning (e.g. WAL/pragmas): delete them.

- [ ] **Step 12: Verify the SQLite driver is gone**

Run: `cargo tree -p matrix-bot -i sqlx-sqlite`
Expected: `error: package ID specification 'sqlx-sqlite' did not match any packages` (or empty output).
Run: `cargo tree -p matrix-bot -i libsqlite3-sys`
Expected: only the `rusqlite ← matrix-sdk-sqlite` path.
Run: `grep -rn "Sqlite\|sqlite" backend/src --include=*.rs | grep -v "sqlite_store"`
Expected: no hits except comments you have reworded; fix any remaining.

- [ ] **Step 13: Full suite on PostgreSQL**

Run: `cargo check -p matrix-bot --all-targets && cargo test -p matrix-bot --all-targets`
Expected: warning-free, all pass. Also run `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1` — all pass.

- [ ] **Step 14: Commit**

```bash
git add -A backend/
git commit -m "refactor!: make PostgreSQL the only application database

DbPool and DbTx become aliases over PgPool and Transaction<Postgres>; the
query macros keep their call syntax with a single PostgreSQL arm. The sqlx
sqlite feature is dropped. DATABASE_URL is required and must be a
postgres:// URL. Tests always run on PostgreSQL via TEST_DATABASE_URL.
matrix-sdk's own SQLite crypto store is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Deployment, CI and docs follow the cut

**Files:**
- Modify: `.github/workflows/ci.yml` (jobs `backend`, `backend-postgres`)
- Modify: `docker-compose.yml:15-20,55-80`, `.env.example` (DATABASE_URL lines)
- Modify: `AGENTS.md` (Overview, Commands, Database & SQLx, Gotchas, Testing), `docs/ARCHITECTURE.md` ("Two databases" section)

**Interfaces:**
- Consumes: Task 1's `TEST_DATABASE_URL` requirement.
- Produces: a single `backend` CI job running on PostgreSQL 18 (Task 6 adds a step to it).

- [ ] **Step 1: Merge the CI jobs**

In `ci.yml`, delete the `backend` job and rename `backend-postgres` to `backend` (`name: Backend CI (PostgreSQL)`). In it:
- Service image `postgres:18` (the supported version).
- Add `components: rustfmt` to the toolchain step.
- Steps after rust-cache, in order:

```yaml
      - name: Format check
        run: cargo fmt --all -- --check

      - name: Run backend tests & check targets
        run: cargo test -p matrix-bot --all-targets

      - name: Run the live PostgreSQL tests
        run: cargo test --package matrix-bot live_postgres -- --ignored --test-threads=1

      - name: Verify OpenAPI spec is up-to-date
        run: |
          cargo run --bin generate-openapi
          git diff --exit-code backend/openapi.json
```

Keep the existing container/service comments (they explain the `postgres` hostname); delete sentences that compare against "the SQLite job".

- [ ] **Step 2: Compose defaults to PostgreSQL**

In `docker-compose.yml`:
- Bot env: `- DATABASE_URL=${DATABASE_URL:-postgres://${POSTGRES_USER:-bot}:${POSTGRES_PASSWORD:-bot}@postgres:5432/${POSTGRES_DB:-matrix_bot}}`; replace the "SQLite by default" comment with "PostgreSQL is the only application database. `data/` still holds matrix-sdk's crypto store."
- `depends_on.postgres`: `condition: service_healthy`, remove `required: false`.
- `postgres` service: remove `profiles: [postgres]` and the "Only started with `--profile postgres`" comment.

Update `.env.example`'s `DATABASE_URL` to a `postgres://` example and remove any SQLite example.

- [ ] **Step 3: Validate compose**

Run: `docker compose config -q`
Expected: exit 0, no output.

- [ ] **Step 4: Update AGENTS.md and ARCHITECTURE.md**

AGENTS.md:
- Overview: "Data lives in PostgreSQL 18+ (the only application database); matrix-sdk keeps its own SQLite crypto store under `data/`."
- Commands: replace the backend test line with the `docker run … postgres:18` + `export TEST_DATABASE_URL=…` block from this plan's Global Constraints, then `cargo test -p matrix-bot --all-targets`. Remove "Run the suite against PostgreSQL instead of in-memory SQLite" wording. Change the compose command to drop `--profile postgres` and the `DATABASE_URL=` prefix.
- Database & SQLx: keep the macro list for now but replace "dual-dialect macros … rewrites it to `$1..$n` for Postgres" with "`?`-style macros (PostgreSQL only; being replaced by compile-checked `query!`)"; delete the "SQLite pools: WAL…" line. Leave the portability and type tables unchanged in this task — Task 6 rewrites them when `query!` becomes the rule.
- Gotchas: `DATABASE_URL` has no default and must be `postgres://`.
- Testing: CI runs four jobs → three (`backend`, `inference`, `frontend`), `backend` on PostgreSQL 18.

ARCHITECTURE.md "Two databases": replace with a short section "PostgreSQL only" stating the decision, the matrix-sdk crypto-store exception, and linking the foundation design doc.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml docker-compose.yml .env.example AGENTS.md docs/ARCHITECTURE.md
git commit -m "ci: run the backend suite on PostgreSQL 18 only

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3a: Upgrade matrix-sdk 0.7 → 0.18 (unblocks sqlx 0.9)

**Why:** Cargo checks `links = "sqlite3"` conflicts even for optional, non-activated dependencies. sqlx 0.9.0 (via its optional `sqlx-sqlite`, also reachable through `sqlx-macros-core`) requires `libsqlite3-sys >=0.30.1, <0.38`; matrix-sdk-sqlite 0.7.1 → rusqlite 0.30 → `libsqlite3-sys ^0.27`. matrix-sdk-sqlite 0.14–0.18 use rusqlite `^0.37` → `libsqlite3-sys ^0.35` (compatible); 0.19 moves to rusqlite 0.40 → `^0.38` (not). Target: **matrix-sdk 0.18**, the newest compatible release (user decision: sqlx 0.9).

**Files:**
- Modify: `backend/Cargo.toml` (matrix-sdk line; remove the direct `libsqlite3-sys` dependency and its comment block added in Task 1)
- Modify: every file the compiler flags — expected concentration: `backend/src/bot/{client,verification,stage,quote,reaction,reaction_targets,reply,notify,media,mod}.rs`, `backend/src/modules/rooms/mod.rs`, `backend/src/modules/ai/alert.rs`, `backend/src/modules/tts/mod.rs`, `backend/src/api/auth/types.rs`, and single uses in `modules/{voice,ocr,reminder,webhook}`
- Read first: `docs/ARCHITECTURE.md` sections "Matrix session and crypto store" and "Verification" — their invariants must hold after the upgrade

**Interfaces:**
- Consumes: nothing from Tasks 1–2 beyond the build.
- Produces: `matrix-sdk = { version = "0.18", features = ["e2e-encryption", "bundled-sqlite"] }` (or the 0.18 feature names that give an E2EE client with a bundled SQLite store); `cargo tree -p matrix-bot -i libsqlite3-sys` shows `libsqlite3-sys 0.35.x` only under `rusqlite ← matrix-sdk-sqlite`. Task 3 relies on exactly this.

**Invariants that must survive (from ARCHITECTURE.md / AGENTS.md):**
- The crypto store stays at the same on-disk path; matrix-sdk-sqlite migrates its own schema in place when opened by the newer version (0.7 → 0.18 crosses several store schema versions; the SDK applies them in order). Never delete or re-create it as part of the upgrade.
- Never keep the crypto store across a fresh login; never pin a device ID on a fresh login (whatever the code does today about this must be preserved with the 0.18 API).
- `ensure_self_verified` runs on every boot; `Recovery::recover()` returning `Ok` is still not treated as "verified".
- Replies still use `text_html(plain, html)`; voice replies are still sent as Ogg Opus voice messages with MSC3245 metadata.
- `!link` is still handled before sender resolution.

- [ ] **Step 1: Bump and see the damage**

Edit `backend/Cargo.toml`: `matrix-sdk = { version = "0.18", features = ["e2e-encryption", "bundled-sqlite"] }` — check matrix-sdk 0.18's `Cargo.toml` feature list (`~/.cargo/registry/src/*/matrix-sdk-0.18.*/Cargo.toml` after `cargo fetch`) and use the names that exist; delete the direct `libsqlite3-sys` dependency and its comment. Run:
`cargo update -p matrix-sdk && cargo tree -p matrix-bot -i libsqlite3-sys`
Expected: resolves, `libsqlite3-sys v0.35.x` via `rusqlite ← matrix-sdk-sqlite`.
Run: `cargo check -p matrix-bot --all-targets 2>&1 | grep -c "^error"` and record the count in the report.

- [ ] **Step 2: Port the API changes**

Fix compile errors file by file. Consult the matrix-sdk CHANGELOG entries for every release 0.8 through 0.18 (`~/.cargo/registry/src/*/matrix-sdk-0.18.*/CHANGELOG.md`) and the ruma version it pulls in. Rules: preserve behaviour exactly; no compatibility shims; when an API was removed without a direct replacement, pick the documented successor and name it in the report. If an invariant above cannot be kept with the 0.18 API, stop and report NEEDS_CONTEXT.

- [ ] **Step 3: Verify tests and the build**

Run: `cargo fmt --all -- --check && cargo check -p matrix-bot --all-targets && cargo test -p matrix-bot --all-targets && cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1 && cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
Expected: all green, warning-free. Also `cargo check -p matrix-bot-inference` still builds (it must not depend on matrix-sdk; confirm nothing changed there).

- [ ] **Step 4: Live boot check against a COPY of the crypto store**

Never point the upgraded binary at the real `data/` store first. Copy it:
```bash
cp -r ../../../data /tmp/matrix-store-copy   # the main checkout's data/ directory
```
Point the store path at the copy (use the env var or constant `bot/client.rs` reads; if the path is a hard-coded constant, run from a scratch directory containing the copied `data/`), boot with the real Matrix credentials from the main checkout's `.env` and a throwaway `DATABASE_URL` database, and confirm in the log: login restored (no fresh login), store opened and migrated, `ensure_self_verified` reports verified, sync loop running. Stop the process. If credentials are not available in this environment, record that the live check was skipped and why — the controller will run it with the user before merge.

- [ ] **Step 5: Commit**

```bash
git add backend/Cargo.toml Cargo.lock backend/src
git commit -m "chore!: upgrade matrix-sdk to 0.18

matrix-sdk-sqlite 0.18 uses rusqlite 0.37 / libsqlite3-sys 0.35, the newest
line compatible with sqlx 0.9's sqlite links requirement. The crypto store
is migrated in place by the SDK on first open.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Upgrade sqlx to 0.9

**Files:**
- Modify: `backend/Cargo.toml:36`, `Cargo.lock`
- Modify: whatever the compiler flags (expected: `backend/src/db/executor.rs` macros, a few `Executor`/`Transaction` call sites)

**Interfaces:**
- Consumes: Task 1 (no `sqlite` feature) and Task 3a (matrix-sdk 0.18, so `libsqlite3-sys` is 0.35.x — inside sqlx 0.9's optional `sqlx-sqlite` range `>=0.30.1, <0.38`).
- Produces: sqlx 0.9 with features `runtime-tokio`, `tls-rustls`, `postgres`, `chrono`, `uuid`, `json`, `migrate`, `macros` for Tasks 4–6 and the next plan.

- [ ] **Step 1: Bump**

```toml
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "tls-rustls-ring-webpki", "postgres", "chrono", "uuid", "json", "migrate", "macros"] }
```

Run: `cargo update -p sqlx && cargo tree -p matrix-bot -i libsqlite3-sys`
Expected: resolves; `libsqlite3-sys` appears only under `matrix-sdk-sqlite`. If resolution fails on `links = "sqlite3"`, a transitive `sqlx-sqlite` has crept back — run `cargo tree -p matrix-bot -i sqlx-sqlite` and remove the feature that pulls it.

- [ ] **Step 2: Fix compile errors**

Run: `cargo check -p matrix-bot --all-targets 2>&1 | head -80`

Known 0.9 changes and their fixes (confirm against sqlx's CHANGELOG for 0.8.x and 0.9.0):
- **`SqlSafeStr`:** `query`, `query_as`, `query_scalar` and `raw_sql` accept a `&'static str` or a value wrapped in `sqlx::AssertSqlSafe(...)`. The `?`-style macros build SQL at runtime (`__render` returns `Cow<str>`), so wrap it once inside each macro: `sqlx::query_as::<_, $ty>(sqlx::AssertSqlSafe(__sql.into_owned()))`. Every value is still bound, never interpolated — the wrapper asserts exactly that. Do the same in `db_ddl!` and any direct `sqlx::query(&format!(..))` call site the compiler flags.
- MSRV is Rust 1.94.
- `Transaction` no longer derefs to a connection usable as `&mut *tx` in some positions → use `&mut **tx` (the macros already do) or `tx.as_mut()`.
- `sqlx::raw_sql(...)`: covered by the `SqlSafeStr` item above.
- `query_as::<_, (T,)>` tuple `FromRow` unchanged.

Fix each error at its site; do not add compatibility shims.

- [ ] **Step 3: Full verification**

Run: `cargo fmt --all -- --check && cargo check -p matrix-bot --all-targets && cargo test -p matrix-bot --all-targets && cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1 && cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
Expected: all green, warning-free.

- [ ] **Step 4: Commit**

```bash
git add backend/Cargo.toml Cargo.lock backend/src
git commit -m "chore: upgrade sqlx to 0.9

Fixes RUSTSEC-2024-0363 and enables the uuid, json and macros features the
compile-checked query rewrite needs.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Baseline migration, proved identical to the legacy schema

**Files:**
- Create: `backend/migrations/0001_baseline.sql`
- Create: `backend/src/db/catalog.rs` (`#[cfg(test)]` only)
- Modify: `backend/src/db/mod.rs` (add `#[cfg(test)] pub mod catalog;`)
- Test: `backend/src/db/catalog.rs` (equivalence test lives with the helper)

**Interfaces:**
- Consumes: `crate::db::pool::{connect_pg_schema, drop_pg_schema, test_pool}`, `crate::db::schema::create_schema(pool)` (legacy, still present).
- Produces:
  - `#[cfg(test)] pub async fn snapshot(pool: &DbPool, schema: &str) -> SchemaSnapshot` where `pub struct SchemaSnapshot { pub columns: BTreeSet<String>, pub indexes: BTreeSet<String>, pub constraints: BTreeSet<String>, pub triggers: BTreeSet<String>, pub functions: BTreeSet<String> }`, `#[derive(Debug, PartialEq, Eq)]`, plus `pub fn diff(&self, other: &Self) -> String` (human-readable, empty when equal).
  - `backend/migrations/0001_baseline.sql`, consumed by Task 5's `sqlx::migrate!("./migrations")`.

- [ ] **Step 1: Write the catalog snapshot helper**

`backend/src/db/catalog.rs`:

```rust
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
```

- [ ] **Step 2: Write the failing equivalence test**

Append to `catalog.rs`:

```rust
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
        assert!(a.columns.len() > 100, "snapshot looks empty: {}", a.columns.len());
        assert_eq!(a, b, "baseline differs from legacy schema:\n{}", a.diff(&b));

        // Applying the baseline over an already-built legacy schema (the live
        // database's situation) must succeed and change nothing.
        migrator.run(&legacy).await.unwrap();
        let a2 = snapshot(&legacy, "baseline_legacy").await;
        assert_eq!(a, a2, "baseline is not a no-op on a legacy database:\n{}", a.diff(&a2));

        crate::db::pool::drop_pg_schema("baseline_legacy").await;
        crate::db::pool::drop_pg_schema("baseline_fresh").await;
    }
}
```

Name contains `live_postgres` so CI's ignored-test step runs it.

- [ ] **Step 3: Run it to verify it fails**

Create an empty `backend/migrations/0001_baseline.sql` (so `migrate!` compiles), register `#[cfg(test)] pub mod catalog;` in `db/mod.rs`, then:
Run: `cargo test -p matrix-bot live_postgres_baseline -- --ignored`
Expected: FAIL at `assert_eq!(a, b, …)` with every legacy column listed as `-`.

- [ ] **Step 4: Transcribe the baseline**

Fill `0001_baseline.sql` by walking `db::schema::create_schema` **in its call order** (`schema_auth`, `schema_settings`, `schema_webhooks`, `schema_ai`, `schema_portfolio`, `schema_footprints`, `schema_habits`, `schema_mcp`, `schema_life`, `schema_bookmarks`, `schema_rooms`, `schema_todos`, `schema_reminders`, `schema_finance`, `schema_bills`, `schema_reactions`) and copying every DDL statement, applying exactly these rules:

1. Header comment: `-- Baseline: the schema as built by the pre-migration schema*.rs runner (master @ <short sha>). Idempotent so a database built by that runner adopts it in place. Never edit; add a new migration.`
2. `{{PK}}` → `BIGSERIAL PRIMARY KEY`; `{{NOW_TEXT}}` → `(NOW()::text)`.
3. `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` / `CREATE UNIQUE INDEX IF NOT EXISTS`: copy verbatim.
4. Best-effort `ALTER TABLE t ADD COLUMN c …` (errors ignored with `let _ =`): write `ALTER TABLE t ADD COLUMN IF NOT EXISTS c …`. This keeps an older database upgradable and is a no-op on a fresh one.
5. `ALTER TABLE … RENAME COLUMN sender_mxid TO owner_id` (in `schema.rs`): omit — every database this project ever built has already been renamed, and a migration cannot swallow the error.
6. Any other best-effort `ALTER` (drop column, alter type, etc.): express it idempotently with PostgreSQL guards (`DROP COLUMN IF EXISTS`, or a `DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.columns WHERE …) THEN … END IF; END $$;` block). If it cannot be made idempotent, stop and ask.
7. Audit-log triggers: copy the PostgreSQL SQL that `ddl::audit_log_immutability()` returns, writing functions as `CREATE OR REPLACE FUNCTION` and each trigger as `DROP TRIGGER IF EXISTS <name> ON portfolio_audit_log;` followed by `CREATE TRIGGER …`.
8. Data statements (`UPDATE`, `INSERT`, `DELETE`) inside `schema_*.rs`: omit (a fresh database has no rows; the live one already ran them). List each omitted statement in a comment at the end of the file so a reviewer can check.
9. No `BEGIN`/`COMMIT` — sqlx wraps each migration in a transaction.

- [ ] **Step 5: Run the equivalence test until it passes**

Run: `cargo test -p matrix-bot live_postgres_baseline -- --ignored`
Expected: PASS. On failure, the message lists `-`/`+` lines; fix the SQL (never the snapshot helper, unless a difference is purely cosmetic such as a schema-qualified name the `replace` missed).

- [ ] **Step 6: Commit**

```bash
git add backend/migrations/0001_baseline.sql backend/src/db/catalog.rs backend/src/db/mod.rs
git commit -m "feat: add idempotent baseline migration matching the legacy schema

A catalog-snapshot test proves 0001_baseline builds exactly what the
schema*.rs runner builds, and that applying it over a legacy-built schema
is a no-op, so the live database can adopt it in place.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Boot on the migrator; delete the legacy schema runner

**Files:**
- Create: `backend/src/db/migrate.rs`
- Delete: `backend/src/db/schema.rs`, `backend/src/db/schema_*.rs` (16 files), `backend/src/db/ddl.rs`, `backend/src/db/backfill.rs`
- Modify: `backend/src/db/mod.rs`, `backend/src/db/pool.rs`, `backend/src/db/catalog.rs` (equivalence test becomes a rehearsal test), every caller of `crate::db::schema::run_migrations` (≈110, mechanical)
- Modify: `AGENTS.md` (Commands: migrate command; Database & SQLx: migration rules), `docs/ARCHITECTURE.md` ("Persistence and migrations")

**Interfaces:**
- Consumes: `0001_baseline.sql`; `catalog::snapshot`; `modules::portfolio::audit::verify_audit_chain(pool: &DbPool, owner_id: &str) -> anyhow::Result<AuditVerificationResult>`.
- Produces: `pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError>` in `crate::db::migrate` (re-exported as `crate::db::run_migrations`); `#[cfg(test)] pub async fn run_migrations_unlocked(pool: &DbPool)`.

- [ ] **Step 1: Write the failing test**

`backend/src/db/migrate.rs`:

```rust
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
}
```

Register `pub mod migrate;` and `pub use migrate::run_migrations;` in `db/mod.rs`.

- [ ] **Step 2: Run it to verify it passes against the baseline**

Run: `cargo test -p matrix-bot test_migrations_create_the_schema_and_are_idempotent`
Expected: PASS (it depends only on Task 4's file).

- [ ] **Step 3: Switch boot and tests to the migrator**

- `pool.rs` `Database::connect`: replace `schema::run_migrations(&pool)` with `crate::db::migrate::run_migrations(&pool).await.context("Failed to apply database migrations")?;`.
- Mechanically rewrite every test call site:
  ```bash
  grep -rln "db::schema::run_migrations\|schema::run_migrations" backend/src \
    | xargs sed -i 's/crate::db::schema::run_migrations(&\([a-z_.]*\)).await.unwrap()/crate::db::migrate::run_migrations_unlocked(\&\1).await/g'
  ```
  Then `grep -rn "schema::run_migrations" backend/src` and fix the remaining variants (`.await?`, `.expect(..)`, non-test callers) by hand: test code → `run_migrations_unlocked(&pool).await`; production code → `crate::db::run_migrations(&pool).await`.
- Move the tests still worth keeping from `schema.rs` into `migrate.rs`'s test module, switching them to `run_migrations_unlocked`: `test_migrations_preserve_existing_data`, `test_category_name_uniqueness_is_enforced_by_the_schema`, and the audit-log trigger tests (`live_postgres` ones included). Drop `test_creates_every_table`/`test_migrations_are_idempotent` (covered by Step 1's test) and `test_the_whole_schema_creates_on_a_live_postgres` (covered by migrations running at all).

- [ ] **Step 4: Replace the equivalence test with the production rehearsal test**

The legacy runner is about to be deleted, so in `catalog.rs` delete `test_live_postgres_baseline_matches_legacy_schema` and add:

```rust
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

    crate::db::run_migrations(&copy).await.expect("migrations apply to production copy");

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
    assert_eq!(prod, want, "production schema drifted from the migrations:\n{}", want.diff(&prod));
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
        assert!(result.is_valid(), "audit chain broken for {owner}: {result:?}");
    }
}
```

Check `AuditVerificationResult`'s actual field/method for validity (`grep -n "pub struct AuditVerificationResult" -A10 backend/src/modules/portfolio/audit.rs`) and use it in place of `is_valid()` if that method does not exist.

- [ ] **Step 5: Delete the legacy runner**

```bash
git rm backend/src/db/schema.rs backend/src/db/schema_*.rs backend/src/db/ddl.rs backend/src/db/backfill.rs
```

Remove their `pub mod` lines from `db/mod.rs`. Fix remaining references (`grep -rn "db::schema\|db::ddl\|db_ddl!\|backfill" backend/src`). `modules/portfolio/audit.rs` used `ddl::drop_audit_update_trigger()` / `audit_update_trigger()` to suspend the guard during a merge: inline those two SQL strings (the PostgreSQL text from Task 1) as `const` in `audit.rs` next to their use, executed with `tx_execute!`/`db_execute!`.

- [ ] **Step 6: Full verification**

Run: `cargo fmt --all -- --check && cargo check -p matrix-bot --all-targets && cargo test -p matrix-bot --all-targets && cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1 && cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
Expected: all green.

Boot check against a fresh database:
```bash
docker exec bot-pg createdb -U postgres bootcheck
DATABASE_URL=postgres://postgres:bot@localhost:5432/bootcheck MATRIX_HOMESERVER= cargo run
```
Stop it (Ctrl-C) once the log shows "Applying database migrations" followed by the API listening line. Then confirm one applied migration:
`docker exec bot-pg psql -U postgres -d bootcheck -c "SELECT version, description, success FROM _sqlx_migrations"` → one row, `success = t`.

- [ ] **Step 7: Rehearse on a production copy (release gate)**

Ask the user for a `pg_dump` of production (or for them to run it), restore it locally as in the test's doc comment, and run `test_migrates_a_real_database`. Record the result (pass, or the diff) in the commit message body. Do not merge Task 5 without a passing rehearsal. See `docs/runbooks/postgres-foundation-cutover.md` for the full durable procedure (this rehearsal plus the pre-boot, first-boot and rollback steps).

- [ ] **Step 8: Docs**

AGENTS.md:
- Commands: replace the `MIGRATE_TEST_DB=/tmp/copy.db …` block with the `pg_dump`/`pg_restore`/`MIGRATE_TEST_DATABASE_URL` block from Step 4.
- Database & SQLx: replace "Migrate via `CREATE TABLE IF NOT EXISTS` + best-effort `ALTER TABLE` in `db/schema_*.rs` … No `sqlx migrate` directory." with: "Schema changes are forward-only SQL files in `backend/migrations/NNNN_description.sql`, applied at boot by `sqlx::migrate!`. Never edit a committed migration. A data- or type-rewriting migration starts with a guard that names offending rows, and is rehearsed on a production copy (`migrates_a_real_database`) before release."
- Module Implementation Checklist item 2: "Database migration: a new `backend/migrations/NNNN_<feature>.sql`".
- Architecture bullet "Persistence and migrations": update the one-liner to match.

ARCHITECTURE.md "Persistence and migrations": rewrite to describe `sqlx::migrate!`, the idempotent baseline and why it is idempotent (adoption of the live DB), and the rehearsal gate.

- [ ] **Step 9: Commit**

```bash
git add -A backend/ AGENTS.md docs/ARCHITECTURE.md
git commit -m "refactor!: apply schema through versioned sqlx migrations

Boot runs sqlx::migrate! over backend/migrations. The schema*.rs runner,
ddl.rs and the one-shot backfill are deleted; the live database adopts
0001_baseline in place. Release note: run current master once before
upgrading so the live schema equals the baseline.

Rehearsal on production copy: <result>

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Compile-checked query pipeline

**Files:**
- Create: `.cargo/config.toml` (or append if it exists), `.sqlx/` (generated)
- Modify: `backend/src/api/health.rs:24`
- Modify: `.github/workflows/ci.yml` (`backend` job), `AGENTS.md`, `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md`

**Interfaces:**
- Consumes: Task 3 (`macros` feature), Task 5 (migrations define the schema `cargo sqlx prepare` checks against).
- Produces: the `query!` workflow every later feature change uses — `SQLX_OFFLINE=true` builds from `.sqlx/`, `cargo sqlx prepare --workspace` regenerates it, CI fails on drift.

- [ ] **Step 1: Install the CLI**

Run: `cargo install sqlx-cli --version ^0.9 --no-default-features --features rustls,postgres --locked`
Expected: `cargo sqlx --version` prints `0.9.x`.

- [ ] **Step 2: Force offline builds by default**

`.cargo/config.toml`:

```toml
[env]
# Compile-checked queries read the committed .sqlx/ cache, never a live
# database: a developer's .env DATABASE_URL must not be queried at build time.
# `cargo sqlx prepare` overrides this for its own run.
SQLX_OFFLINE = "true"
```

- [ ] **Step 3: Convert the first query**

`backend/src/api/health.rs:24`:

```rust
let db_ok = sqlx::query_scalar!(r#"SELECT 1 AS "ok!""#)
    .fetch_one(&state.db.pool)
    .await
    .is_ok();
```

Run: `cargo check -p matrix-bot`
Expected: FAIL — `SQLX_OFFLINE=true but there is no cached data for this query`.

- [ ] **Step 4: Generate the cache**

```bash
docker exec bot-pg createdb -U postgres sqlx_prepare
DATABASE_URL=postgres://postgres:bot@localhost:5432/sqlx_prepare sqlx migrate run --source backend/migrations
DATABASE_URL=postgres://postgres:bot@localhost:5432/sqlx_prepare cargo sqlx prepare --workspace -- -p matrix-bot --all-targets
```

Expected: `.sqlx/query-<hash>.json` created. Run `cargo check -p matrix-bot --all-targets` — PASS.

- [ ] **Step 5: CI drift check**

In the `backend` job, before "Run backend tests & check targets", add:

```yaml
      - name: Install sqlx-cli
        run: cargo install sqlx-cli --version ^0.9 --no-default-features --features rustls,postgres --locked

      # The committed .sqlx/ cache must match both the queries and the schema.
      - name: Verify the sqlx offline cache
        env:
          DATABASE_URL: postgres://postgres:bot@postgres:5432/postgres
        run: |
          sqlx migrate run --source backend/migrations
          cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets
```

Also set `SQLX_OFFLINE: "true"` in the `release.yml` build env if it builds the backend without `.cargo/config.toml` in scope (check; `.cargo/config.toml` at the repo root already applies to builds run from the repo).

- [ ] **Step 6: Docs**

AGENTS.md, Database & SQLx:
- Replace "Never call `sqlx::query*` directly. Use the dual-dialect macros…" and "Compile-time `query!` macros stay banned…" with: "New and converted code uses compile-checked `sqlx::query!` / `query_as!` / `query_scalar!`. Builds read the committed `.sqlx/` cache (`SQLX_OFFLINE=true` via `.cargo/config.toml`). After changing a query or a migration, regenerate with `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` against a migrated database and commit `.sqlx/`; CI fails on drift. The `db_*!`/`tx_*!` macros remain only in code not yet converted — do not add new uses."
- Delete the "Keep SQL portable" table and the "Match the column type to the Rust type" table's SQLite rationale; keep the Rust↔PostgreSQL type table (it still applies and `query!` now enforces it).
- Commands: add the prepare command.

Design doc (`2026-09-24-postgres-foundation-design.md`): in §6, swap steps 1 and 2 and add the `libsqlite3-sys` reason; move `0002_account_uuid` to a new step after step 5 with the compiler-guided reason; in §4.3 note the test harness stays `test_pool()` + `run_migrations_unlocked`. Renumber migration filenames in §3 so jobs/outbox are `0002`/`0003` and the account UUID migration follows them. Mirror the filename change in `SPEC.md` §13's tree.

- [ ] **Step 7: Full verification and commit**

Run: `cargo fmt --all -- --check && cargo check -p matrix-bot --all-targets && cargo test -p matrix-bot --all-targets && cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
Expected: all green.

```bash
git add .cargo/config.toml .sqlx backend/src/api/health.rs .github/workflows/ci.yml AGENTS.md SPEC.md docs/superpowers/specs/2026-09-24-postgres-foundation-design.md
git commit -m "build: compile-check queries against a committed sqlx offline cache

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## What comes next (separate plans)

1. **Layer skeleton** (design step 4): `app.rs`, `domain/ application/ interface/ llm/ jobs/ events/ infrastructure/`, architecture test, `/health/live` + `/health/ready`, graceful shutdown.
2. **Feature migration** (design step 5): one feature per change into the layers with `query!`; the final change deletes `executor.rs`, `dialect.rs` and the macros.
3. **Account UUID** (`owner_id` → `UUID` + FKs, `AccountId`), compiler-guided, rehearsed on production.
4. **Job queue** (step 6), **Outbox** (step 7).
