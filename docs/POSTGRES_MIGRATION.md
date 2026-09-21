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
| 6 | Convert the 5 production transaction sites | **done** — see below |
| 7 | Test harness runs both dialects; CI Postgres service | **done** — see below |
| 8 | Docs, `.env.example`, `docker-compose.yml` | todo |

Stages 3–6 are done and **the scaffolding is gone**: `DbPool::sqlite_or_err()`,
`Database::sqlite_pool()` and the boot-time refusal of a Postgres URL were all
deleted in stage 6. Pointing `DATABASE_URL` at PostgreSQL now works.

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
Baseline at stage 6: **713 passed, 0 failed, 14 ignored**, plus 6 live
PostgreSQL tests including a full boot. `cargo check` is warning-free.

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

## Stage 6 notes

**PostgreSQL is now a supported target.** `Database::connect` no longer refuses
it, `sqlite_or_err()` and `sqlite_pool()` are deleted, and
`test_the_bot_boots_against_a_live_postgres` runs the whole boot path —
`Database::connect`, every migration, `backfill`, then an ordinary write and
read. That test is the claim reduced to something that either passes or does
not.

`DbTx` mirrors `DbPool`: an enum over `Transaction<'_, Sqlite>` and
`Transaction<'_, Postgres>`, with a `tx_*` macro family. Those two are different
types with no shared trait covering both `commit` and use as an executor, which
is why transactions could not ride along with stage 5.

### Replacing `UPDATE OR IGNORE`

This was the piece flagged from the start, and it needed re-deriving rather than
translating. SQLite's `UPDATE OR IGNORE` moves the rows it can and silently
skips those that would violate a unique constraint. PostgreSQL has no
equivalent, and it aborts the whole transaction on a failed statement unless a
savepoint catches it.

`accounts::merge::move_rows` attempts the bulk update inside a savepoint — one
round trip, the common case — and only on a unique violation rolls back and
moves rows one at a time, each in its own savepoint. An all-or-nothing retry
would have been wrong in a way that loses data: if `from` has "food" and
"travel" while `into` already has "food", **"travel" must still transfer**.

Nothing there enumerates which tables have which constraints; the database
reports the collision. That was deliberate — the comment being replaced named
four unique-constrained tables when the schema actually has seven, so an
enumeration had already drifted once.

`row_key` does name one column per table for the per-row fallback, and
`test_every_merged_table_has_the_row_key_this_names` checks each name by
selecting it against a migrated database rather than repeating the list.
Both guards were mutation-tested: flattening `row_key` to `"id"` fails the
guard, and making the fallback all-or-nothing fails the collision test.

`INSERT OR IGNORE` is likewise SQLite-only and became `ON CONFLICT DO NOTHING`,
which both accept.

### Also fixed here

A boolean-as-integer literal my stage 5 sweep missed: it caught `is_direct = 0`
in comparisons but not `VALUES (?, ?, ?, 0)` in inserts. Two such sites, plus
two in test fixtures.

`cargo fix` was a mistake worth recording: run to clear unused imports, it
removed `use sqlx::SqlitePool` from 76 files where only the `#[cfg(test)]`
module used it, breaking the test build. The right fix was to move those imports
*into* the test modules, which is where they belong.

## Stage 7 notes

`db::pool::test_pool()` is now the single place a test gets a database. It
returns in-memory SQLite by default — so `cargo test` stays instant and needs
nothing installed — and, when `TEST_DATABASE_URL` is set, a PostgreSQL pool
scoped to a schema of its own.

The per-test schema is not decoration. The suite runs in parallel and
`search_path` is a property of a connection, not a statement, so tests sharing
one schema would see each other's tables. The name carries the process id so two
concurrent `cargo test` runs cannot collide either. Schemas are left behind
deliberately: the server is a throwaway CI container, and a test that failed is
easier to look at with its rows still there.

The pool comes back **unmigrated**, exactly as the inline
`SqlitePool::connect` did, so every test still runs whichever schema it wants
and nothing about the 84 call sites changed beyond the one expression.

CI gains a `backend-postgres` job running the same suite against `postgres:18`,
plus the `#[ignore]`d live tests that need a server. The SQLite job stays the
fast default. Both run on every pull request, because the whole argument for
this job is that SQLite accepts things Postgres rejects — `completed = 0` for a
boolean, bare `TIMESTAMP` for a `DateTime<Utc>`, `INTEGER` where an `i64` is
read — and each of those was a real defect found only by running against a
server.

One self-inflicted bug worth recording: the script that rewrote 84 test setups
to call `test_pool()` also rewrote the SQLite fallback *inside `test_pool`
itself*, producing infinite recursion. `rustc` caught it as "recursion in an
async fn requires boxing". A bulk rewrite has to exclude its own target.

### Where the PostgreSQL suite actually stands

**594 of 719 pass; 125 fail.** SQLite remains 719/719. The CI job is
`continue-on-error: true` until that reaches zero — visible and reporting, not
blocking. Leaving it out until it were green would be worse: this job is what
found every dialect defect listed below.

Production defects it found and that are now fixed:

| Defect | Why SQLite hid it |
|---|---|
| `randomblob(n)` in 14 places minting account ids, session tokens, recovery and link codes, MCP and webhook tokens | SQLite-only function. Now `utils::random`, so a token's entropy no longer depends on the deployed database |
| `briefing_enabled` and `is_active` declared `BIGINT` but read as `bool` | SQLite stores a boolean as an integer; PostgreSQL refuses. **Introduced by my own stage 4 `INTEGER` → `BIGINT` sweep** |
| `next_due_at` declared `TIMESTAMPTZ` but read as `String` | Same cause: the stage 4 `TIMESTAMP` → `TIMESTAMPTZ` sweep assumed every timestamp column is a `DateTime<Utc>`; this one is not |

The two schema errors above are worth dwelling on: a blanket type sweep is
exactly the kind of change that looks safe on a dynamically typed database and
is not. Both were caught here and nowhere else.

Remaining failures, by cause:

| Cause | Distinct errors | Kind |
|---|---|---|
| Test fixtures inserting `0`/`1` into a `BOOLEAN` column | 13 | test-only |
| `next_due_at` bound as text in fixtures | 9 | test-only |
| Fixtures creating tables with `AUTOINCREMENT` | 6 | test-only |
| Guard tests reading `sqlite_master` | 4 | SQLite-only by nature; should skip on Postgres |
| Fixtures calling `datetime()` | 2 | test-only |
| One `syntax error at or near "$3"` | 1 | **unexplained — investigate before trusting the renderer** |

Most of the 125 cascade from those ~36 root errors. The one that deserves a
look before anything else is the `$3` syntax error: every other entry is a
fixture written in SQLite dialect, but that one could mean
`render_placeholders` emits invalid SQL for some statement shape, which would
be a production bug rather than a test one.

### Also worth recording

The first PostgreSQL run showed **265 failures from "could not resize shared
memory segment"** — Docker's default 64 MB `/dev/shm`, not a code problem at
all. `--shm-size=1g` on the container fixed it. Without checking, that would
have read as "PostgreSQL does not work".
