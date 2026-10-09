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
- A service method is the unit of transaction. It opens `pool.begin()` only where an invariant needs atomicity and publishes its `SystemEvent` inside that transaction with `repositories::outbox::publish`, so the event commits or rolls back with the state change ([Outbox](#outbox)).
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

Migration `0003_jobs` adds one table, `jobs`, that the job runtime in `backend/src/jobs/` uses ([`architecture.md`](architecture.md#background-jobs)). Only `infrastructure/postgres/repositories/jobs.rs` reads or writes it; the `LISTEN` helper (`job_notifications.rs`) subscribes to its channel, and the scheduler's advisory lock (`advisory.rs`) is a session lock that does not touch the table.

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

## Outbox

Migration `0004_outbox` adds the transactional outbox ([`architecture.md`](architecture.md#events-and-the-outbox)). Only `infrastructure/postgres/repositories/outbox.rs` (and its `LISTEN` helper `outbox_notifications.rs`) touches it.

| Table | Meaning |
|---|---|
| `outbox_events` | one row per `SystemEvent`: `id` `BIGINT` identity, `account_id` `UUID` FK to `accounts` `ON DELETE CASCADE`, `event_type` (the variant name), `payload` `JSONB` (the wire `{type, payload}`), `created_at` |
| `outbox_deliveries` | one row per durable destination: `event_id` FK (`ON DELETE CASCADE`), `destination` (`matrix:<room id>`), `status` (`pending`, `delivered`, `failed`; CHECK), `attempts`, `max_attempts` (default 10), `available_at`, `delivered_at`, `last_error`; `UNIQUE (event_id, destination)` |

Indexes: `idx_outbox_events_account_type_created` (`account_id, event_type, created_at DESC`), `idx_outbox_events_created` (`created_at`, for retention), `idx_outbox_deliveries_pending` (`available_at`) on `status = 'pending'`, `idx_outbox_deliveries_event` (`event_id`).

- **Wake-ups.** The `outbox_notify` trigger (`AFTER INSERT ON outbox_events FOR EACH ROW`) runs `pg_notify('outbox', '<id>:<account uuid>')`. Like `jobs_notify`, it fires only for real inserts and is delivered at commit. The payload carries no event body: the listener loads the row.
- **Claiming.** `claim_due` is one statement with `FOR UPDATE SKIP LOCKED` that increments `attempts` and moves `available_at` 60 s ahead (the lease); a claim that is never recorded reappears after the lease.
- **Retention.** The hourly purge deletes events older than 30 days that have no `pending` delivery; their deliveries go with them (cascade). A pending delivery is never purged, however old, so an outage cannot silently discard an alert. Webhook logs are read from these rows and therefore cover 30 days.
- **Account merge.** `outbox_events` is in `ACCOUNT_ID_FOLLOWS` (like `jobs`): history moves to the surviving account. Deliveries reach it through the event and need no entry of their own.
- **0005 drops `webhook_outbox`.** `0005_drop_webhook_outbox` is a plain `DROP TABLE`; its history is deliberately not migrated (spec O2). Alerts still queued in it at cutover are lost, so drain it (no `pending` rows) before releasing; the rehearsal on a production copy covers 0004 and 0005 (and 0006–0008, below).

## Platform identities and link tokens

Migrations `0006`–`0008` (sub-project 2) replace the Matrix-only link tables with channel-neutral ones ([`architecture.md`](architecture.md#identity-and-linking)). Only `infrastructure/postgres/repositories/identities.rs` and `link_tokens.rs` touch them.

| Table | Meaning |
|---|---|
| `platform_identities` | one row per chat identity bound to an account: `id` `BIGSERIAL`, `account_id` `UUID` FK to `accounts` `ON DELETE CASCADE`, `platform` (`matrix`, `discord`, `telegram`, `email`; CHECK), `platform_user_id` (canonical form: MXID, numeric id, lowercased address), `display_name`, `created_at`, `updated_at`; **`UNIQUE (platform, platform_user_id)`**, index on `account_id` |
| `link_tokens` | one row per minted link token: `token_hash` `TEXT` primary key, `platform`, `platform_user_id`, `display_name`, `conversation` (storage-form address of the direct conversation that asked, where the confirmation goes), `created_at`, `expires_at`, `consumed_at`; partial index on `(platform, platform_user_id) WHERE consumed_at IS NULL` |

- **Link tokens are hash-only, short-lived and single use.** Only the SHA-256 of the token is stored (the token itself exists in the chat reply and the URL and nowhere else), `expires_at` is `created_at` + 10 minutes (`LINK_TOKEN_TTL`), and `consumed_at` is set when the token is claimed or superseded (starting again burns the identity's earlier live token). A claim reads the row with `FOR UPDATE` inside its transaction, so two concurrent claims of one token serialise and only the first can succeed. Rows are not purged on a schedule; an expired or consumed row is inert.
- **Uniqueness is the race guard.** `UNIQUE (platform, platform_user_id)` is what makes "one chat identity, one account" hold under concurrent links and first messages; a lost bind surfaces as `AlreadyLinked`, never as a second row.
- **Account merge.** `platform_identities` is in `ACCOUNT_ID_FOLLOWS`: the absorbed account's identities move to the survivor. `link_tokens` has no `account_id` (it belongs to an identity, not an account) and is in no merge list.
- **`0006_platform_identities`** is plain DDL: both tables and their indexes.
- **`0007_platform_identities_backfill`** starts with a guard that raises, naming the offending `mxid`s, if any `matrix_links` row references a missing account (a hand-edited database); otherwise it copies every `matrix_links` row into `platform_identities` as `matrix` (using `linked_at` for both timestamps) and drops `matrix_links`. It rewrites data, so it is rehearsed on a production copy ([runbook](superpowers/runbooks/postgres-foundation-cutover.md)).
- **`0008_drop_matrix_link_challenges`** drops the web-minted challenge table; pending challenges are not carried over (the `!link <code>` flow no longer exists).

## Knowledge Base and Vector Embeddings (`0011_knowledge_base.sql`)

Migration `0011_knowledge_base` introduces the personal knowledge base and vector search subsystem:

- **pgvector extension:** `CREATE EXTENSION IF NOT EXISTS vector;` enabled on PostgreSQL 18.
- **`notes` table:**
  - `id BIGSERIAL PRIMARY KEY`
  - `owner_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE`
  - `title TEXT NOT NULL`, `content TEXT NOT NULL` (Markdown, the single source of truth), `format TEXT NOT NULL DEFAULT 'markdown'` (only `markdown` is allowed; kept for API compatibility)
  - `source_channel TEXT NOT NULL DEFAULT 'web'`, `source_ref TEXT`
  - `tags TEXT[] NOT NULL DEFAULT '{}'`
  - `is_archived BOOLEAN NOT NULL DEFAULT FALSE`
  - `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`, `updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`
  - Indexes: `idx_notes_owner_updated (owner_id, updated_at DESC)`, `idx_notes_tags USING GIN (tags)`. The full-text and trigram indexes are described under *Notes: derived columns* below.
- **`knowledge_embeddings` table:**
  - `id BIGSERIAL PRIMARY KEY`
  - `owner_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE`
  - `item_type TEXT NOT NULL`, `item_id BIGINT NOT NULL`, `chunk_index INTEGER NOT NULL DEFAULT 0`
  - `content_chunk TEXT NOT NULL`
  - `embedding vector(384) NOT NULL` (sized for `multilingual-e5-small`)
  - `embedding_model TEXT NOT NULL` (added in `0013`): the model that produced the chunk; search only compares vectors of the configured `EMBEDDING_MODEL`, and a chunk of any other model is re-embedded by the backfill
  - `created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()`
  - Unique constraint: `UNIQUE (owner_id, item_type, item_id, chunk_index)`
  - Vector Index: `idx_knowledge_embeddings_vector USING hnsw (embedding vector_cosine_ops) WITH (m = 16, ef_construction = 64)`.
- **Ownership & Account Merge:** Both `notes` and `knowledge_embeddings` are listed in `OWNED_TABLES` (`domain/accounts/mod.rs`), so an account merge safely cascades and reassigns all notes and vector embeddings to the surviving account.

### Notes: derived columns (`0012`–`0014`)

`notes.content` holds the Markdown and nothing else is authoritative. Everything search needs is **derived from it by the service on every write** (`application/knowledge/markdown.rs::derive`) and can be rebuilt from it:

| Column | Meaning |
|---|---|
| `content_hash` | SHA-256 of the UTF-8 body, lowercase hex. The optimistic-concurrency token (`expected_content_hash`) and the indexing skip check. |
| `search_text` | The body as plain text (syntax, HTML and URLs dropped). What full-text and trigram search and the embedder read. |
| `excerpt` | The first 200 characters of `search_text`, whitespace collapsed. What lists show, so a list never loads a body. |
| `note_type` | `note`, `idea`, `fact`, `observation` or `decision` (CHECK constraint). |
| `search_vector` | Generated, stored `tsvector`: `title` (weight A) and `search_text` (weight B) under the `simple` configuration, with a GIN index. `simple` keeps CJK runs as single tokens, so CJK matching goes through the trigram path. |
| `indexed_content_hash`, `indexed_at` | The body hash the stored embedding chunks were built from. The backfill re-embeds a note whose `indexed_content_hash` differs from `content_hash` (a rename clears it too, because the title is part of every chunk). |

`0012` rewrites `format` to `markdown` (guarded, data only), `0013` adds the columns, `pg_trgm` and the indexes, and `0014` makes `content_hash`, `search_text` and `excerpt` NOT NULL and rebuilds the generated column and trigram index over `search_text` alone. `0014` aborts naming up to 20 note ids if any column is still NULL, which means the previous release's backfill did not finish (runbook in `docs/superpowers/HANDOFF.md`).

`pg_trgm` and `vector` come from the `pgvector/pgvector` image's contrib set; the trigram GIN index covers `title || ' ' || search_text`, which is the exact expression the `ILIKE` and `word_similarity` queries use, so a query must keep that expression to use it.

**Rebuild invariant.** Embeddings are derived data. Deleting every row of `knowledge_embeddings` and clearing `notes.indexed_content_hash` loses nothing: `knowledge.backfill_embeddings` re-queues every item and the chunks come back identical (a test pins this). The same path re-tags everything after an `EMBEDDING_MODEL` change.

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
docker run -d --name pg18-test -p 55433:5432 -e POSTGRES_PASSWORD=bot pgvector/pgvector:pg18
export TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
export PG_TEST_URL=$TEST_DATABASE_URL
```

| Variable | Read by |
|---|---|
| `TEST_DATABASE_URL` | every database test (`db::pool::test_pool`) |
| `PG_TEST_URL` | the `#[ignore]`d `live_postgres` tests |
| `DATABASE_URL` | `sqlx migrate run` and `cargo sqlx prepare` only |
| `MIGRATE_TEST_DATABASE_URL` | the production-rehearsal test only: a restored copy, never production |

`test_pool()` creates a fresh schema per test (`t<pid>_<n>`) and returns an **unmigrated** pool; tests call `run_migrations_unlocked` themselves. The schema outlives the test, so a failing test keeps its rows for inspection, but not the next run: `db::test_schemas` does the cleanup.

- On its first `test_pool()` call, each test process takes a session-level advisory lock keyed on its pid, on a dedicated connection that stays open until the process exits. PostgreSQL releases the lock however the process ends, even on a crash or Ctrl-C.
- The process then drops, in the background, every `t<pid>_*` schema whose pid holds no such lock. Schemas from a concurrent `cargo test` stay, because that run still holds its lock.
- `test_pool()` drops a same-named schema before creating one, in case a dead process with the same pid left it behind.

So leftovers are bounded by one run's worth (a few hundred schemas), and the first run after an interrupted one reaps them. The partial-migration tests in `db/migrate.rs` use `test_pool()` too and are covered. The `#[ignore]`d live tests use named schemas (`boot_check`, `trigger_check`) that drop themselves. To inspect a failed test's rows, query its schema before starting another test process. To wipe everything by hand:

```bash
psql "$TEST_DATABASE_URL" -Atc "SELECT format('DROP SCHEMA %I CASCADE;', nspname) FROM pg_namespace WHERE nspname ~ '^t[0-9]+_[0-9]+$'" | psql "$TEST_DATABASE_URL"
```

Fixtures create their owners with `db::pool::seed_account(s)`; a fixture that writes as an id with no `accounts` row fails on the foreign key, exactly as production would. Repository tests cover round-trips and owner isolation; service tests cover transaction atomicity (a NUL byte in a `TEXT` field forces a mid-transaction failure), degradation on a closed pool, and events written in the same transaction (`repositories::outbox::event_types_for` reads them back; a rolled-back change leaves none).

| Symptom | Fix |
|---|---|
| `TEST_DATABASE_URL must point at a PostgreSQL server` | export it and check `docker ps` shows `pg18-test` |
| `could not resize shared memory segment`, `PoolTimedOut`, or a very slow suite | leftover test schemas from before the reaper existed: run any test once to reap them, or use the manual drop above |
| `no cached data for this query` at compile time | `.sqlx/` is stale: regenerate it (see *Adding a migration*) |
| `migration … was previously applied but has been modified` | a migration file changed, often CRLF: restore it; line endings are pinned to LF |
