# PostgreSQL support — running record

Branch: `feat/postgres-support`, off `cca195e` (`feat/tts-voice-replies`).

**Decision:** support *both* SQLite and PostgreSQL, chosen at boot from
`DATABASE_URL`. SQLite stays the default so the standalone single binary and
`docker run` with a volume keep working with no external service. Tests keep
running on in-memory SQLite by default, with a PostgreSQL pass added in CI.

This is the one place the branch argues with CLAUDE.md's "do not preserve
backward compatibility / no compatibility layers" rule. The reasoning: SQLite
is not an obsolete path here, it is the shipped default that four documented
deployment options depend on. Removing it would delete a product capability,
not a dead one.

## Why not `sqlx::Any`

Measured, not assumed. `sqlx-core-0.7.4/src/any/type_info.rs` defines
`AnyTypeInfoKind` as exactly `Null, Bool, SmallInt, Integer, BigInt, Real,
Double, Text, Blob`. **No chrono.** Seven `modules/*/models.rs` files bind
`DateTime<Utc>` directly, so `Any` would force every timestamp back to
`String`. `schema_todos.rs:35` documents that precise regression as a bug
already fixed once — `due_at` held a rendered display string that could not be
compared or subtracted from. sqlx 0.8 does not change this type set.

## The load-bearing fact

`db/dialect_probe.rs` proves, at compile time, that one
`#[derive(sqlx::FromRow)]` model satisfies `FromRow` for **both** `SqliteRow`
and `PgRow`, including `Option<DateTime<Utc>>` and `bool`. That is why **no
model struct needs to change**. If that probe ever stops compiling, this whole
design is unsound and every model would need a second driver-specific mapping.

## Surface area

| Coupling | Count |
|---|---|
| Files referencing `SqlitePool` | 139 |
| `SqlitePool` occurrences | 707 |
| Query call sites | 553 (882 `.bind()`) |
| DDL lines across `db/schema_*.rs` | 1,410 |
| Tests on in-memory SQLite | 130 sites / 697 tests |
| `.begin()` transaction sites | 8 (5 production, 3 test) |
| `INTEGER PRIMARY KEY` / `AUTOINCREMENT` | 20 |
| `datetime('now')` | 18 |
| `ON CONFLICT` | 14 |
| `last_insert_rowid()` | 6 |
| `strftime` | 5 |
| `INSERT OR IGNORE` | 3 |
| `UPDATE OR IGNORE` · `PRAGMA` · `sqlite_master` · `RAISE(ABORT)` | 2 each |

## Stages

| # | Stage | Status |
|---|---|---|
| 0 | `postgres` driver feature; `dialect_probe` compile gate | **done** — `0c5faed` |
| 1 | `dialect.rs`: `Dialect`, `render_placeholders` | **done** — `0c5faed` |
| 2 | `executor.rs`: `DbPool` enum + the five macros | **done** — `6e39bd3` |
| 3 | `pool.rs`: connect either driver from `DATABASE_URL` | **done** — see below |
| 4 | Dialect-aware DDL across the 20 `schema_*.rs` files | **done** — see below |
| 5 | Convert 553 query sites, module by module | **done** — see below |
| 6 | Convert the 5 production transaction sites | todo |
| 7 | Test harness runs both dialects; CI Postgres service | todo |
| 8 | Docs, `.env.example`, `docker-compose.yml` | todo |

Stage 5 is the bulk. It is deliberately incremental: `DbPool::sqlite_or_err()`
is scaffolding that lets unconverted modules keep compiling and passing while
repositories move one at a time. **It and its test are deleted when the last
caller goes** — that count reaching zero is the completion signal for stage 5.

## Known dialect gaps to resolve in stages 4–6

- `INTEGER PRIMARY KEY AUTOINCREMENT` → `BIGSERIAL PRIMARY KEY`
- `BOOLEAN NOT NULL DEFAULT 0` → `DEFAULT FALSE` (Postgres has a real boolean)
- `datetime('now')` → `NOW()`; `strftime(...)` → `to_char`/`date_trunc`
- `INSERT OR IGNORE` → `INSERT ... ON CONFLICT DO NOTHING`
- `UPDATE OR IGNORE` — no direct equivalent; needs rework per call site.
  Note `accounts/merge.rs` relies on `UPDATE OR IGNORE` *not* suppressing a
  trigger abort (`prevent_portfolio_audit_update`), which is measured
  behaviour recorded in `REFACTOR_PROGRESS.md`. Postgres semantics differ and
  this call site must be re-derived, not translated.
- `RAISE(ABORT)` triggers → plpgsql trigger functions
- Best-effort `ALTER TABLE ADD COLUMN` (errors swallowed with `let _ =`) →
  `ADD COLUMN IF NOT EXISTS`, which Postgres supports and SQLite does not
- `PRAGMA` / `sqlite_master` introspection → `information_schema`
- `last_insert_rowid()` → `RETURNING id` (works on both; pinned by
  `test_returning_yields_the_new_id_without_last_insert_rowid`)

## Verification standard

Every stage: `cargo test --workspace` green, `cargo fmt --all -- --check`
clean, and any guard test mutation-tested before it is trusted. The
`render_placeholders` scanner was mutation-tested — disabling its
literal-skipping arm fails exactly the three tests written to catch it.

Baseline at stage 2: **697 passed, 0 failed, 8 ignored.**
Baseline at stage 3: **699 passed, 0 failed, 9 ignored.**
Baseline at stage 4: **709 passed, 0 failed, 11 ignored**, plus 3 live
PostgreSQL tests passing against PostgreSQL 18.6.
Baseline at stage 5: **711 passed, 0 failed, 12 ignored**, plus 4 live
PostgreSQL tests including a real repository round-trip.

## Stage 3 notes

`connect_pool` opens either driver. SQLite keeps its directory creation, WAL
journal mode and busy timeout; PostgreSQL has none of those, because the server
owns its storage.

`Database.pool` is now a `DbPool`, but **no repository signature changed**.
`Database::sqlite_pool()` returns `&SqlitePool`, so all 406 `pool: &SqlitePool`
signatures stayed as they are and only the ~495 call sites reaching `db.pool`
moved to `db.sqlite_pool()`. That is what keeps stage 5 incremental: a module
converts by changing its own signatures to `&DbPool` and its bodies to the
macros, with everything else still compiling.

`Database::connect` **refuses a Postgres URL at boot** with an explanation.
Until stages 4 and 5 land, the DDL is SQLite-only and most repositories still
reach `sqlite_pool()`, so booting on Postgres would create no tables and then
fail per query. Refusing is the honest behaviour, and it also makes
`sqlite_pool()`'s `expect` unreachable by construction. The guard, the
scaffolding accessor and their tests are deleted in the same commit as stage 5.

**Not yet verified against a real server.** The unit tests prove a Postgres URL
is *routed* to the Postgres driver and that a SQLite URL is not, but Docker
Desktop was not running on the development machine, so
`test_connect_pool_connects_to_a_live_postgres` has never been executed. It is
`#[ignore]`d and carries the exact command:

```bash
docker run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=bot --name bot-pg postgres:18
PG_TEST_URL=postgres://postgres:bot@127.0.0.1:55432/postgres   cargo test --package matrix-bot connects_to_a_live_postgres -- --ignored
```

Run it before trusting stage 3.

## Stage 4 notes

The whole schema now creates on a real PostgreSQL. `create_schema` was split out
of `run_migrations` so it can be verified before `backfill` converts, and
`test_the_whole_schema_creates_on_a_live_postgres` runs it twice to prove
idempotence — which matters more on Postgres, since it has no
`CREATE TRIGGER IF NOT EXISTS`.

Most DDL stayed written once. `db/ddl.rs` substitutes **named tokens**
(`{{PK}}`, `{{NOW_TEXT}}`) rather than pattern-matching SQL, so an unhandled
construct leaves an unsubstituted token and fails loudly instead of being
mistranslated. The audit-log triggers are written out per dialect, because
PostgreSQL needs a `plpgsql` function plus a row trigger where SQLite needs one
statement. `db/ddl.rs` is now the single source for those, so the schema and
`absorb_ledger`'s drop/restore pair cannot drift — which the old
`AUDIT_UPDATE_TRIGGER` comment warned about.

The `ALTER TABLE ... ADD COLUMN` statements needed nothing: their errors were
already discarded with `let _ =`, and Postgres's duplicate-column error is
discarded the same way.

### Four defects the live test caught that SQLite never would

1. **`INTEGER` is `int4` on PostgreSQL.** Every model reads `i64`, so 58 columns
   would have failed with `Rust type i64 (as SQL type INT8) is not compatible
   with SQL type INT4` — and `amount_minor` would have capped money at about
   2.1 billion minor units. All widened to `BIGINT`, which is a no-op on SQLite
   (INTEGER affinity). Pinned by
   `test_sqlite_round_trips_a_bigint_beyond_int4_range`.
2. **`SET search_path` does not survive a pool.** It applies to one session, and
   a pool hands successive queries to different connections, so
   `matrix_links` was created on one and looked for from another. The search
   path now travels in the connection options.
3. **PostgreSQL refuses multiple commands in a prepared statement.** Four DDL
   blocks bundle a `CREATE TABLE` with its indexes. `db_ddl!` now uses
   `sqlx::raw_sql` (simple query protocol); DDL has no bind parameters, so
   nothing is lost.
4. **Temporaries died inside the macros.** `db_ddl!(pool, &format!(..))` dropped
   the `format!` result mid-statement. The macros now bind the SQL expression in
   a `let` first, which extends the temporary to the end of the block. This
   would have recurred at stage 5 call sites.

### Running the live tests

```bash
docker run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=bot --name bot-pg postgres:18
```

```bash
PG_TEST_URL=postgres://postgres:bot@127.0.0.1:55432/postgres cargo test --package matrix-bot live_postgres -- --ignored --test-threads=1
```

Each test works in a throwaway schema it creates and drops, so the tests neither
depend on nor leave behind state.

## Stage 5 notes

All 553 query sites now go through the dual-dialect macros, and a **real
repository round-trips on PostgreSQL** — insert with `RETURNING id`, read back,
filter, update. Compiling only proved the call sites type-check; that test is
what proves the SQL they send is accepted.

The macro set grew to nine: `db_fetch_all/one/optional`,
`db_scalar_all/one/optional`, `db_execute`, and `db_fetch_all_dyn`/
`db_fetch_one_dyn` for the handful of list endpoints whose bind count depends on
which optional filters a request carries. Passing `_` as the row type expands to
`query_as::<_, _>`, which is how the 81 sites that never had a turbofish
converted without inventing types for them.

Two idioms had no portable form and were rewritten rather than translated:

- `last_insert_rowid()` is SQLite-only. The five inserts that used it now use
  `RETURNING id` with `db_scalar_one!`, which both databases support.
- Raw `row.get::<T, _>("col")` cannot be written once, because `SqliteRow` and
  `PgRow` are different types. `db/rooms.rs` selects a typed tuple instead.

Transactions are deliberately **not** converted — `Transaction<Sqlite>` and
`Transaction<Postgres>` are different types, and that is stage 6. The ten
remaining `sqlite_or_err()` callers are exactly those sites (`accounts/merge.rs`,
`modules/portfolio/trade.rs`, `db/rooms.rs`) plus one `PRAGMA table_info` guard
test. That count reaching zero is still the completion signal.

### Three more defects only the live server found

1. **`completed = 0` is not a boolean on PostgreSQL.** SQLite accepts `0`/`1`
   for a `BOOLEAN`; Postgres reports `column "completed" is of type boolean but
   expression is of type integer`. Fixed across nine boolean columns, in both
   comparisons and `INSERT` literals.
2. **`TIMESTAMP` is "without time zone" on PostgreSQL** and will not decode into
   `DateTime<Utc>`. The schema now says `TIMESTAMPTZ`, which SQLite gives the
   same NUMERIC affinity, so existing databases are unaffected. Pinned by
   `test_sqlite_round_trips_a_timestamptz_into_datetime_utc`.
3. **A `sed` replacement corrupted the guard itself.** `DbPool::Sqlite(pool) =>
   Ok(pool)` inside `sqlite_or_err` was rewritten to a catch-all arm `pool =>
   Ok(pool)`, which would have made the Postgres refusal unreachable and
   returned the wrong type. The compiler caught it; a looser signature would not
   have. Two call sites were also corrupted into `api_&state.db.pool` by `sed`
   treating `&` in the replacement as "the whole match".
