# Feature Layering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move every feature out of `modules/` into `domain/` → `application/` → `infrastructure/postgres/` with adapters in `interface/`, rewriting every `db_*!`/`tx_*!` call as compile-checked `sqlx::query!`, then delete the interim macro layer and `modules/`.

**Architecture:** One feature per task (or one coupled batch). Each task follows the **Feature Migration Contract** below: pure domain types and invariants; an application service per feature that owns transactions and event publication; a concrete PostgreSQL repository with private row structs; HTTP DTOs, Matrix command handlers, MCP and LLM-tool adapters that call the service. The public HTTP contract is frozen: `backend/openapi.json` must not change.

**Tech Stack:** Rust 2021, sqlx 0.9 (`query!`/`query_as!`/`query_scalar!`, offline `.sqlx/` cache), PostgreSQL 18, axum 0.7, utoipa, matrix-sdk 0.18.

**Spec:** [`docs/superpowers/specs/2026-09-24-postgres-foundation-design.md`](../specs/2026-09-24-postgres-foundation-design.md) §3, §3.1–§3.3, §6 (feature migration step). Project rules: `AGENTS.md` (Engineering Principles, Definition of Done). Survey of every feature (sizes, macro counts, tables, coupling, business-rule locations, transactions, adapters): [`2026-09-25-feature-survey.md`](2026-09-25-feature-survey.md).

## Global Constraints

- **No behaviour change.** Every existing test keeps passing (tests move with their code; counts may only grow). No new features.
- **Public HTTP contract frozen:** after every task `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json` passes with **no diff**. DTO names, field names, optionality and enums stay identical.
- **Matrix command syntax and replies unchanged** (same prefixes, same `text_html` output).
- **Compile-checked SQL:** converted code uses `sqlx::query!` / `query_as!` / `query_scalar!` only; `.sqlx/` regenerated and committed per task; no new `db_*!`/`tx_*!`/`sqlx::query(` calls.
- **Dependency rule** (spec §3.1, enforced by `backend/tests/architecture.rs`): `domain` imports nothing but std/chrono/uuid/serde/thiserror; `application` never imports axum/matrix-sdk/reqwest/utoipa/interface; `infrastructure` never imports application/interface/modules.
- **Files ≤ 500 lines of production code**; a file that would exceed it is split along a real seam as part of the move (survey lists the current offenders).
- **Every `SystemEvent` still carries a meaningful `owner_id()`**; application services publish after commit using the existing `EventBroadcaster` (the outbox plan replaces the mechanism later).
- **`OWNED_TABLES` stays complete** — moving a table's code never removes it from the list.
- Every task ends with: `cargo fmt --all -- --check`; `cargo check -p matrix-bot --all-targets` (warning-free); `cargo test -p matrix-bot --all-targets`; `cargo test -p matrix-bot --test architecture`; `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1`; `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets` against a migrated scratch DB; OpenAPI no-diff; clippy on touched files adds no warnings.
- Never run the bot binary; never contact external services. Local test DB `postgres://postgres:bot@localhost:55433/postgres`; `CARGO_TARGET_DIR=D:/01_Data/code/matrix-hello-bot/target`.

## Decisions (controller rulings for this plan)

- **D-A `accounts/merge.rs` dynamic SQL:** it iterates `OWNED_TABLES` building `SELECT/UPDATE/DELETE … {table} …`. It moves to `infrastructure/postgres/merge.rs` and stays dynamic: identifiers come only from the `OWNED_TABLES`/`ACCOUNT_ID_*` constants, each checked with an `assert!`-style allow-list guard before formatting, values always bound, SQL wrapped in `sqlx::AssertSqlSafe` **only in that file**. Task 14 adds an architecture rule that `AssertSqlSafe` appears nowhere else. (A 20-arm `match` over table names would be less reviewable and no safer.)
- **D-B Transport/persistence split:** domain types derive neither `sqlx::FromRow` nor `utoipa::ToSchema`. Repositories map private `Row` structs into domain types; HTTP DTOs (keeping the **exact** current schema names and fields) are built from domain types with `From` impls in `interface/http`.
- **D-C Matrix handlers:** each feature's `BotModule` implementation moves to `interface/chat/<feature>.rs` and calls the application service. `BotModule`/`ModuleRegistry` move to `interface/chat/mod.rs` in Task 14, when `modules/` is deleted.
- **D-D Cross-feature reads:** a read model that aggregates several features' tables (`life`, `briefing`, `finance` net worth) lives in `infrastructure/postgres/read_models/<name>.rs` as `query!` SQL; it is the only place allowed to join across features' tables. Writes always go through the owning feature's service.
- **D-E Coupled batches:** todo + reminder + recurring_bills + due_reminder move together (shared `source_kind`/`source_id` due-reminder linkage).
- **D-F Service wiring:** services are constructed once in `app.rs` into an `AppServices` struct (`Arc`-free where the service is cheap to clone: it holds a `PgPool` and an `EventBroadcaster`, both clone-cheap) and passed to `ApiState` and to the Matrix `ModuleContext`. Adapters never build their own services. Task 1 creates `AppServices` with the rooms service; each later task adds its field.
- **D-G Interface SQL allow-list:** `backend/tests/architecture.rs` holds `INTERFACE_SQL_ALLOW_LIST` (the interface files that still run SQL). Each task removes its feature's files from the list; the test fails if a listed file no longer offends, so the list only shrinks. Task 14 deletes the list.
- **D-H Chat context types:** `QuotedMessage` (today `interface/channels/matrix/quote.rs`) is platform-neutral chat context. The first task whose application service needs it (Task 8) moves the type to `domain/chat.rs` (parsing stays in the Matrix adapter), per spec §3.3's `ChatRequest.quoted`. `StageReporter`/`notify` stay adapter-side: application code receives what it needs as plain data or a narrow trait defined in `application/`.

---

## Feature Migration Contract

Every task applies this contract to its feature(s). Task 1 (rooms) establishes it in code; later tasks follow the patterns Task 1 committed (read `domain/rooms`, `application/rooms`, `infrastructure/postgres/repositories/rooms.rs`, `interface/chat/rooms.rs` first).

**Target files per feature `<f>`:**
```
backend/src/domain/<f>/mod.rs            entities, value objects, validated commands, DomainError variants
backend/src/application/<f>/mod.rs       <F>Service { pool: PgPool, events: EventBroadcaster } + use-case methods
backend/src/infrastructure/postgres/repositories/<f>.rs   query! functions over impl PgExecutor
backend/src/interface/http/<f>.rs        handlers + DTOs (moved/adapted in place)
backend/src/interface/chat/<f>.rs        BotModule impl (Matrix commands), calls the service
```
(Split any of these into a directory with submodules when it would exceed 500 lines.)

**Shapes:**
```rust
// domain/<f>/mod.rs — pure; synchronous; no I/O
pub struct Bookmark { pub id: BookmarkId, pub owner: AccountId, pub url: Url, /* … */ }
pub struct NewBookmark { /* validated fields */ }
impl NewBookmark {
    /// Validates and normalises input (500-char caps, URL parsing, …).
    pub fn parse(raw: RawBookmark) -> Result<Self, DomainError> { /* … */ }
}

// infrastructure/postgres/repositories/<f>.rs — the only place with SQL for <f>
struct BookmarkRow { id: i64, owner_id: String, url: String, /* … */ }
impl TryFrom<BookmarkRow> for Bookmark { /* … */ }

pub async fn insert(conn: impl sqlx::PgExecutor<'_>, owner: &AccountId, b: &NewBookmark)
    -> Result<Bookmark, sqlx::Error>
{
    let row = sqlx::query_as!(BookmarkRow,
        "INSERT INTO bookmarks (owner_id, url, …) VALUES ($1, $2, …) RETURNING id, owner_id, url, …",
        owner.as_str(), b.url.as_str(), /* … */)
        .fetch_one(conn).await?;
    row.try_into().map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

// application/<f>/mod.rs — unit of transaction; publishes after commit
impl BookmarkService {
    pub async fn create(&self, owner: &AccountId, raw: RawBookmark) -> Result<Bookmark, AppError> {
        let new = NewBookmark::parse(raw)?;                 // DomainError -> AppError::Validation
        let saved = repositories::bookmarks::insert(&self.pool, owner, &new).await?;
        self.events.send(SystemEvent::BookmarkCreated { owner_id: owner.to_string(), /* … */ });
        Ok(saved)
    }
}
```
- `AccountId` newtype over the current `String` owner id lives in `domain/accounts` (created in Task 3; Tasks 1–2 use `&str` and are updated by Task 3's mechanical pass). It is **not** the UUID migration — the column type stays TEXT here.
- `AppError` (in `application/mod.rs`, created in Task 1): `Validation`, `NotFound`, `Forbidden`, `Conflict`, `Database(sqlx::Error)`, `External(String)`; `interface/http` maps it to `ApiError` with the **same status codes and error strings the handlers return today**.
- Use a transaction only where the invariant needs atomicity (the survey lists every current `begin()` and what it protects — keep exactly those boundaries).
- Tests: domain invariants as plain `#[test]` in `domain/<f>`; repository tests with `test_pool()` + `run_migrations_unlocked` in the repository file; service tests for transaction/event behaviour; existing handler tests move with their handlers.
- LLM tools (`modules/ai/tools/<f>.rs`) and MCP tools that touch `<f>` are repointed at `<F>Service` in the same task.
- Delete `modules/<f>/` when empty; update `ModuleRegistry` registration to the new `interface/chat/<f>.rs` type.
- AGENTS.md: after Task 1, rewrite the "Module Implementation Checklist" to this contract (paths, the openapi no-diff rule, `query!`, ≤500 lines).
- Every task: add the feature's service to `AppServices` (D-F) and remove its interface files from `INTERFACE_SQL_ALLOW_LIST` (D-G) — the architecture test enforces the shrink.

---

### Task 1: rooms — establish the contract (template)
Scope: `db/rooms.rs` (`RoomRepository`, room resolution: `get_cached_rooms`, `save_cached_rooms`, `resolve_user_default_room`, `resolve_target_room`), `modules/rooms/mod.rs`, `interface/http/rooms.rs`. Tables: `account_rooms` (+ reads `user_settings.dev_room_id`, `matrix_links` via `AccountRepository::linked_mxids` — keep those as calls into their owners' code for now). Creates `application/mod.rs` with `AppError`, `infrastructure/postgres/mod.rs` + `repositories/mod.rs`, `interface/chat/mod.rs` (initially just `pub mod rooms;`).
- [ ] Failing tests first: domain (`JoinedRoom` construction, target-resolution rules as pure functions over a room list where possible — the priority order in `resolve_user_default_room` is a business rule and belongs in domain), repository round-trip with `query!`.
- [ ] Implement per the contract; `save_cached_rooms` keeps its transaction (delete + inserts atomically).
- [ ] Create `AppServices` in `app.rs` (D-F) with `rooms`; thread it into `ApiState` and `ModuleContext`; remove rooms' files from `INTERFACE_SQL_ALLOW_LIST` (D-G).
- [ ] Verify (global list); openapi no diff.
- [ ] Rewrite AGENTS.md "Module Implementation Checklist" to the contract.
- [ ] Commit `refactor(rooms): move rooms into the layered layout with compile-checked queries`.

### Task 2: settings
Scope: `modules/settings/*`, `interface/http/settings.rs`, AI tool `modules/ai/tools/settings.rs`. Break the survey's `settings → finance/todo` coupling by having settings expose what they need rather than importing them (or invert the call); document the choice in the report. `user_settings` rules: a missing row means defaults, never an error (AGENTS.md) — make that a domain rule with a test.
- [ ] TDD → implement → verify → commit `refactor(settings): …`.

### Task 3: accounts core + AccountId
Scope: `backend/src/accounts/*` except `merge.rs` (repository, principal, credentials, recovery, sessions, security events), `modules/account/mod.rs` (the `WEB_ONLY` command gate and `!passkey add` first-passkey rule move to domain/application unchanged), `interface/http/account/*`, `interface/http/auth/*` handler call sites. Introduce `domain::accounts::AccountId` (newtype over the current `acct_…` string) and thread it through Tasks 1–2's services in a mechanical pass. Security rules (AGENTS.md Security Rules) must remain enforced exactly — list each and the test that proves it in the report.
- [ ] TDD → implement → verify → commit `refactor(accounts): …`.

### Task 4: accounts merge (decision D-A)
Scope: `accounts/merge.rs` → `application/accounts/merge.rs` (orchestration, preview, transaction) + `infrastructure/postgres/merge.rs` (dynamic SQL per D-A, with the allow-list guard and tests proving a non-allow-listed identifier panics/errs). Keep `portfolio_audit_log`'s dedicated `absorb_ledger` routine and its trigger suspension semantics exactly.
- [ ] TDD → implement → verify → commit `refactor(accounts): isolate merge's dynamic SQL behind an allow-list`.

### Task 5: bookmarks
Scope: `modules/bookmarks/*` (split `metadata.rs` 516 lines), its HTTP, AI tool, MCP use. The two `db_fetch_all_dyn!` list endpoints become `query!` with optional-filter SQL (`($1::text IS NULL OR col = $1)` style) — no string-built SQL.
- [ ] TDD → implement → verify → commit.

### Task 6: footprints + habits
Two leaf features, one task (small, same shape). Footprints' one `_dyn` list query → optional-filter `query!`.
- [ ] TDD → implement → verify → commit.

### Task 7: webhook
Scope: `modules/webhook/*` (+ scheduler: stays a `BotModule` background worker for now; the job-queue plan replaces it), `interface/http/webhooks.rs`, `webhooks_telegram.rs`. Keep the SSRF protections exactly.
- [ ] TDD → implement → verify → commit.

### Task 8: todo + reminder + recurring_bills + due_reminder (decision D-E)
One coupled batch. Split `todo/repository.rs`, `todo/service.rs`, `reminder/repository.rs` (691) along real seams. The todo dynamic column-list SQL becomes explicit `query!` variants. Due-reminder sweep behaviour must be identical — its tests are the guard.
- [ ] TDD → implement → verify → commit (may be several commits, one per feature, each green).

### Task 9: finance
Scope: `modules/finance/*` (split `repository.rs` 624, `handler.rs` 632), analytics, net worth (read model per D-D), categories, shares, HTTP (`finance*.rs`), AI tool.
- [ ] TDD → implement → verify → commit.

### Task 10: portfolio
Scope: `modules/portfolio/*` (split `repository.rs` 845, `service.rs`, `price_feed.rs`, `audit.rs`), scheduler (stays a worker), HTTP (`portfolio*.rs`), AI tool. Invariants that must be domain rules with tests: cash is a hard precondition for buying, never for selling; the holding write and the cash write in one transaction; audit log append-only and hash-chained (`compute_entry_hash` input unchanged — an existing chain must still verify).
- [ ] TDD → implement → verify → commit.

### Task 11: life + journal + briefing (read models)
`life`'s cross-feature UNION queries become `infrastructure/postgres/read_models/life.rs` `query!` SQL (D-D); journal and briefing call life/other services or read models. Read the SQL closely: the survey found CTE names (`activity_counts`, `all_dates`, `user_activities`) that are not tables.
- [ ] TDD → implement → verify → commit.

### Task 12: ocr + voice + tts + core + account command module
Adapter-only features (no tables): move their `BotModule`s to `interface/chat/`, route through the services they use (ai/settings), keep inference calls in `infrastructure/inference`.
- [ ] Implement → verify → commit.

### Task 13: ai + mcp
Scope: `modules/ai/*` (~7 250 lines; split `handler.rs` 1 209), `modules/ai/tools/*` (every tool already repointed at services by earlier tasks — finish the rest), `modules/mcp/*`, `interface/http/ai.rs`, `mcp.rs`. Target: `llm/` (spec §3: tool registry, bounded tool loop, prompts) — tool names already `<ns>_<action>`; the loop keeps its current cap (5 rounds) as `MAX_TOOL_ROUNDS`. AI conversation persistence → `application/ai` + repository. Per AGENTS.md AI/agent rules: tool arguments validated in the service, authorization independent of the model.
- [ ] TDD → implement → verify → commit (several commits).

### Task 14: remove the interim layer
- Delete `db/executor.rs` macros and `db/dialect.rs` (`render_placeholders`); `crate::db` keeps only pool/migrate/catalog.
- Move `BotModule`, `ModuleRegistry`, `BackgroundWorkers` from `modules/mod.rs` to `interface/chat/mod.rs`; delete `modules/`.
- Architecture test additions: no `db_*!`/`tx_*!` anywhere; `AssertSqlSafe` only in `infrastructure/postgres/merge.rs`; no `sqlx::` in `interface/` or `application/` other than `PgPool`/`Transaction`/`Error` types; delete `INTERFACE_SQL_ALLOW_LIST` (must be empty by now). Before adding needles, make the scanner strip string literals as well as `//` comments (today `line.split("//")` truncates at URL literals, e.g. in bookmarks, hiding anything after them on that line).
- AGENTS.md / ARCHITECTURE.md: remove every mention of the `?`-style macros and `modules/`; the Database & SQLx rules state `query!` only.
- [ ] Verify → commit `refactor!: remove the interim query macros and modules/`.
