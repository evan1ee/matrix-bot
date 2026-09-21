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
| 3 | `pool.rs`: connect either driver from `DATABASE_URL` | todo |
| 4 | Dialect-aware DDL across the 20 `schema_*.rs` files | todo |
| 5 | Convert 553 query sites, module by module | todo |
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
