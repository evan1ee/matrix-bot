# Database

PostgreSQL 18 is the only application database. This document covers the schema rules, the migration mechanism, the SQLx conventions and the local test setup. Design rationale for the foundation is in `superpowers/specs/2026-09-24-postgres-foundation-design.md`; the layer model is in [`architecture.md`](architecture.md).

## PostgreSQL only

PostgreSQL is the only application database: `DATABASE_URL` is required and must be `postgres://` or `postgresql://`, and the process refuses to boot otherwise. The one exception is matrix-sdk's own crypto store, which stays SQLite under `data/` regardless of `DATABASE_URL` — that store is matrix-sdk's, not this application's, and is out of scope for this cut. See `superpowers/specs/2026-09-24-postgres-foundation-design.md` for the decision and its rationale.

## Where SQL lives

```text
application/<f>   ──►  infrastructure/postgres/repositories/<f>.rs   ──►  PostgreSQL
(transactions)          (query!/query_as!/query_scalar! only)
                        infrastructure/postgres/read_models/          (cross-feature reads)
                        infrastructure/postgres/merge.rs              (allow-listed dynamic SQL)
```

- A repository is a set of `pub async fn`s taking `conn: impl sqlx::PgExecutor<'_>`, so a service passes either the pool or `&mut *tx`. Rows land in private `…Row` structs and leave as domain types (`TryFrom<Row>` when a row can violate an invariant or needs parsing, `From<Row>` only when every row is valid). A SQLx row type never escapes the repository.
- A service method is the unit of transaction. It opens `pool.begin()` only where an invariant needs atomicity and publishes its `SystemEvent` after commit.
- Cross-feature reads (a daily activity timeline, net worth) go through `infrastructure/postgres/read_models`, never through another feature's repository.
- `interface/` and `application/` run no SQL. `AssertSqlSafe` is allowed only in `infrastructure/postgres/merge.rs`, where table names come from an allow-list and every value is bound.
- **Never put a value into SQL text.** Bind it, even in `sqlx::QueryBuilder`. Optional filters use `($1::text IS NULL OR col = $1)` inside `query!`.
- Compile-checked queries read the committed `.sqlx/` cache (`SQLX_OFFLINE=true` via `.cargo/config.toml`), so builds need no database.

## Column types

Match the column type to the Rust type it is read as, and check both directions when changing either.

| Rust | Column |
|---|---|
| `i64` | `BIGINT` |
| `i32` | `INTEGER` (PostgreSQL's `INTEGER` is `i32`, not `i64`) |
| `f64` | `DOUBLE PRECISION` (`REAL` is `float4`) |
| `bool` | `BOOLEAN` |
| `DateTime<Utc>` | `TIMESTAMPTZ` (bare `TIMESTAMP` carries no zone) |
| `AccountId` | `UUID` |
| `String` holding a timestamp | `TEXT` with a `DEFAULT (NOW()::text)` |

Change column types one at a time, against the field that reads them; three rounds of defects came from blanket `INTEGER`→`BIGINT` and `TIMESTAMP`→`TIMESTAMPTZ` sweeps. `TIMESTAMPTZ` holds microseconds while `DateTime<Utc>` holds nanoseconds, so tests build values with `utils::time::now_at_storage_precision`, not `Utc::now()`. A constraint that matters (uniqueness, foreign key, ownership) lives in the schema, never only in application code. Monetary amounts and quantities are integer minor units; there is no floating-point money anywhere.

## Account ids

`accounts.id`, every `owner_id` and every `account_id` column is a native `UUID` with a foreign key to `accounts` (migration `0002_account_uuid`: one transaction, with a guard that names the offending table, column and value and rolls back on any non-conforming row). In Rust the type is `domain::accounts::AccountId`; it serialises and displays as `acct_<32 hex>`, so the HTTP contract and the portfolio audit-hash chain are unchanged (a pinned-digest test guards historical chains). Ids are typed `AccountId` end to end and there is no implicit conversion from a string; untrusted text (HTTP bodies, `--owner`) goes through `FromStr` and is rejected when malformed. Fixtures use `AccountId::from_seed`.

**Any new owner-keyed table must be added to `OWNED_TABLES`** (`domain/accounts/mod.rs`). An account merge rewrites exactly the tables that constant lists; a missing one is not a compile error, it is a silent data leak on the next merge. A catalog test asserts every `owner_id`/`account_id` column is `UUID` with a foreign key. Tables keyed by `account_id` are classified in `ACCOUNT_ID_FOLLOWS` (rows move with the absorbed account), `ACCOUNT_ID_DISCARDED` or `ACCOUNT_ID_UNTOUCHED` instead.

## Job queue

Migration `0003_jobs` adds one table, `jobs`, that the job runtime in `backend/src/jobs/` uses ([`architecture.md`](architecture.md#background-jobs)). Only `infrastructure/postgres/repositories/jobs.rs` (and the leader lock and `LISTEN` helpers beside it) touch it.

| Column | Meaning |
|---|---|
| `id` | `BIGINT` identity |
| `account_id` | nullable `UUID` FK to `accounts`, `ON DELETE CASCADE`; `NULL` for global jobs such as `portfolio.check_alerts` |
| `job_type`, `payload` | e.g. `reminder.fire`; `JSONB` carrying row ids and dates, never an owner to trust |
| `status` | `pending`, `running`, `completed`, `failed`, `cancelled` (CHECK) |
| `priority`, `available_at` | dequeue order: priority descending, then `available_at` ascending; a job is claimable once `available_at <= NOW()` |
| `attempts`, `max_attempts`, `backoff_seconds` | `attempts` is incremented by `claim`; retry delay is `backoff_seconds · 2^(attempts-1)`, computed by `domain::jobs::backoff_delay` and capped at one day |
| `locked_by`, `locked_at`, `heartbeat_at`, `started_at`, `completed_at` | ownership and liveness of a running job |
| `dedupe_key` | with `job_type`, unique (`uq_jobs_dedupe`); enqueue is `ON CONFLICT DO NOTHING`, and a `NULL` key never conflicts |
| `error` | last failure, truncated to 2,000 characters |

Partial indexes serve the three hot queries: `idx_jobs_dequeue` (`priority DESC, available_at`) on `status = 'pending'`, `idx_jobs_running` (`heartbeat_at`) on `status = 'running'`, and `idx_jobs_finished` (`completed_at`) on the terminal statuses; `idx_jobs_account` covers the account merge.

- **Claiming** is one `UPDATE ... WHERE id = (SELECT ... FOR UPDATE SKIP LOCKED)`, so two workers, or two instances, never get the same job.
- **Guarded transitions.** Every change after `claim` (`heartbeat`, `complete`, `retry`, `defer`, `fail`, `release`) has `WHERE locked_by = $worker AND status = 'running'` and returns whether it applied. A worker whose job the reaper already requeued therefore cannot overwrite the new owner's state.
- **Wake-ups.** The `jobs_notify` trigger (`AFTER INSERT ... FOR EACH ROW`) runs `pg_notify('jobs', '')`. A trigger fires only for rows really inserted, so a deduplicated enqueue sends nothing, and PostgreSQL delivers it at commit, so a worker never wakes to an uncommitted job.
- **Account merge.** `jobs` is in `ACCOUNT_ID_FOLLOWS` (`domain/accounts`): pending work moves to the surviving account, and handlers resolve their subject from the job row.
- **Retention.** Finished jobs are purged after 7 days; the `failed` ones are the operator's record of what did not deliver until then.
- **Connections.** `JOB_WORKERS` workers, one `LISTEN` connection and the HTTP handlers must fit within `DATABASE_MAX_CONNECTIONS`. The scheduler's advisory-lock connection is detached from the pool (the pool opens a replacement) and does not count against it.

## Migrations

`db/migrate.rs::run_migrations` is the only migration mechanism: `sqlx::migrate!("./migrations")` applies every file in `backend/migrations/` in order, tracked in `_sqlx_migrations`, serialised across concurrent boots by sqlx's own advisory lock. Migrations are forward-only — a committed migration is never edited; a mistake is fixed by a new migration, not a rewritten old one.

`0001_baseline.sql` is the one migration that looks unlike the rest: it is idempotent DDL (`CREATE TABLE IF NOT EXISTS`, deterministic `ADD COLUMN IF NOT EXISTS`), because its job is to *adopt* the shape the old hand-rolled `db/schema.rs` runner had already built on the live database, not to build the database itself. It reproduces that runner's schema column-for-column, index-for-index, trigger-for-trigger, but deliberately omits data statements — a fresh database has no rows to touch, and the live database already ran them — including the legacy `owner_id` backfill, the credential-less `active` → `unclaimed` demotion, and the `sender_mxid` → `owner_id` rename on `todos`, `reminders`, `transactions` and `recurring_bills` (a rename is also unrepeatable: it would fail on a column that no longer exists). This makes adoption a **precondition, not an automatic step**: the live database must already run current master (pre-branch) once before upgrading, so its schema already equals what `0001_baseline.sql` describes; upgrading straight from an older schema skips the rename and backfill and leaves missing the columns/rows `0001` assumes exist. The production rehearsal test (`migrates_a_real_database`, below) is the release gate that catches drift from this precondition before it reaches production. Every migration after `0001` is a plain forward-only SQL file with no such idempotence requirement, because there is no second adoption to perform.

Because a migration runs against a real, populated database exactly once, a data- or type-rewriting migration is rehearsed before release: restore a `pg_dump` of production locally and run `cargo test -p matrix-bot migrates_a_real_database -- --ignored` (see `catalog.rs`), which applies the migrations to the copy, asserts every table's row count is unchanged, asserts the resulting schema matches what a fresh database gets from the same migrations, and re-verifies every portfolio audit chain. Never point it at production itself — it mutates whatever it is pointed at.

## Adding a migration

1. Create `backend/migrations/NNNN_description.sql` with the next free number (check the directory and any numbers reserved in the design spec first).
2. A data- or type-rewriting migration starts with a guard that names offending rows and is kept separate from plain DDL. Never silently change the meaning of historical data.
3. Regenerate the query cache against a migrated scratch database and commit `.sqlx/` (CI fails on drift):

   ```bash
   createdb -h localhost -U postgres sqlx_prepare
   sqlx migrate run --source backend/migrations --database-url postgres://postgres:bot@localhost:5432/sqlx_prepare
   DATABASE_URL=postgres://postgres:bot@localhost:5432/sqlx_prepare \
     cargo sqlx prepare --workspace -- -p matrix-bot --all-targets
   cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets   # what CI runs
   ```

4. Add the table to `OWNED_TABLES` if it is owner-keyed.
5. Rehearse a rewriting migration on a production copy (below) before release.

`.gitattributes` pins `backend/migrations/*.sql` and `.sqlx/` to LF: a migration's checksum covers its bytes, and a CRLF checkout fails `sqlx migrate` against an existing database.

## Rehearsing against a production copy

Never point this at production itself; the test mutates whatever it is given.

```bash
pg_dump -Fc "$PROD_URL" > prod.dump
createdb -h localhost -U postgres rehearsal
pg_restore -h localhost -U postgres -d rehearsal --no-owner prod.dump
MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
  cargo test -p matrix-bot migrates_a_real_database -- --ignored --nocapture
```

The full cutover procedure for the PostgreSQL foundation branch (pre-boot backups, first-boot watch list, rollback) is `superpowers/runbooks/postgres-foundation-cutover.md`.

## Test databases

Tests need a throwaway PostgreSQL server, never a real one:

```bash
docker run -d --name pg18-test --shm-size=1g -p 55433:5432 -e POSTGRES_PASSWORD=bot postgres:18
export TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
export PG_TEST_URL=$TEST_DATABASE_URL
```

| Variable | Read by |
|---|---|
| `TEST_DATABASE_URL` | every database test (`db::pool::test_pool`) |
| `PG_TEST_URL` | the `#[ignore]`d `live_postgres` tests |
| `DATABASE_URL` | `sqlx migrate run` and `cargo sqlx prepare` only |
| `MIGRATE_TEST_DATABASE_URL` | the production-rehearsal test only: a restored copy, never production |

`test_pool()` creates a fresh schema per test (`t<pid>_<n>`) and returns an **unmigrated** pool; tests call `run_migrations_unlocked` themselves. Schemas are deliberately never dropped so a failing test keeps its rows, which is why the container needs `--shm-size=1g`. Recreate the container between long sessions, or drop the leftovers:

```bash
psql "$TEST_DATABASE_URL" -Atc "SELECT format('DROP SCHEMA %I CASCADE;', nspname) FROM pg_namespace WHERE nspname ~ '^t[0-9]+_[0-9]+$'" | psql "$TEST_DATABASE_URL"
```

Fixtures create their owners with `db::pool::seed_account(s)`; a fixture that writes as an id with no `accounts` row fails on the foreign key, exactly as production would. Repository tests cover round-trips and owner isolation; service tests cover transaction atomicity (a NUL byte in a `TEXT` field forces a mid-transaction failure), degradation on a closed pool, and events emitted only after commit.

| Symptom | Fix |
|---|---|
| `TEST_DATABASE_URL must point at a PostgreSQL server` | export it and check `docker ps` shows `pg18-test` |
| `could not resize shared memory segment` | too many leftover test schemas: recreate the container with `--shm-size=1g` |
| `no cached data for this query` at compile time | `.sqlx/` is stale: regenerate it (see *Adding a migration*) |
| `migration … was previously applied but has been modified` | a migration file changed, often CRLF: restore it; line endings are pinned to LF |
