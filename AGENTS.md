# AGENTS.md

## Overview

`matrix-bot` is a Matrix chat bot with an embedded web UI: a Rust Cargo workspace holds the bot itself (`backend/`) plus a local-inference server for OCR, speech-to-text and text-to-speech (`inference/`), and a React SPA (`frontend/`) builds into the bot's static assets. Data lives in PostgreSQL 18+ (the only application database); matrix-sdk keeps its own SQLite crypto store under `data/`. This file states every hard rule and convention; `docs/ARCHITECTURE.md` is the long-form companion explaining the *why* behind the parts of the design that aren't obvious from the code alone.

## Commands

```bash
cargo check                              # must be warning-free
cargo fmt --all -- --check               # CI blocks on this; `cargo fmt --all` to fix

# Local PostgreSQL for all tests. `DATABASE_URL` is required and must be
# `postgres://`; there is no SQLite fallback for the application database.
# --shm-size=1g: test_pool() creates a fresh schema per test (see db/pool.rs)
# and never drops it, so a long-running local server's default 64MB of shared
# memory has filled up before now.
docker run --rm -d --name bot-pg -p 5432:5432 -e POSTGRES_PASSWORD=bot --shm-size=1g postgres:18
export TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/postgres
export PG_TEST_URL=$TEST_DATABASE_URL
cargo test -p matrix-bot --all-targets   # backend suite (CI's own invocation)

# test_pool() leaves its per-test schemas (named t<pid>_<n>) behind; drop them
# periodically on a long-running local server:
psql "$TEST_DATABASE_URL" -Atc "SELECT format('DROP SCHEMA %I CASCADE;', nspname) FROM pg_namespace WHERE nspname ~ '^t[0-9]+_[0-9]+$'" | psql "$TEST_DATABASE_URL"
cargo test -p matrix-bot-inference --all-targets   # inference crate; not covered by bare `cargo test`
cargo test --package matrix-bot self_verify -- --ignored   # live check: is the running bot verified?
cargo test test_parse_message_exclamation_command   # single test by name
cargo test --package matrix-bot modules::todo::     # one module's tests
cargo run                                # starts bot + API server

# After changing a sqlx::query!/query_as!/query_scalar! call or a migration,
# regenerate the offline cache against a migrated database and commit .sqlx/;
# CI fails on drift (`cargo sqlx prepare --workspace --check`). Prerequisite:
# create a scratch database and run the migrations against it first.
createdb -h localhost -U postgres sqlx_prepare
sqlx migrate run --source backend/migrations --database-url postgres://postgres:bot@localhost:5432/sqlx_prepare
DATABASE_URL=postgres://postgres:bot@localhost:5432/sqlx_prepare cargo sqlx prepare --workspace -- -p matrix-bot --all-targets

# Rehearse a migration against a restored COPY of production and assert
# nothing was stranded — run before shipping any migration, never against
# production itself.
pg_dump -Fc "$PROD_URL" > prod.dump
createdb -h localhost -U postgres rehearsal
pg_restore -h localhost -U postgres -d rehearsal --no-owner prod.dump
MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
  cargo test -p matrix-bot migrates_a_real_database -- --ignored --nocapture

# After any #[utoipa::path] or ToSchema change, regenerate both, and commit
# backend/openapi.json if it changed — CI's drift check fails on any diff.
# (The crate is a lib `matrix_bot` plus two bins, so generate-openapi reuses
# the API types; tests never write openapi.json.)
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run generate-api

# Frontend
npm --prefix frontend run dev        # Vite dev server on :5173
npm --prefix frontend run build      # tsc + vite build -> backend/web/dist
npm --prefix frontend run test:e2e   # Playwright; `npx --prefix ... playwright` skips the config

# Local Postgres via Compose. Compose already defaults WEBAUTHN_ORIGIN to
# http://localhost:8080; only override it if your .env sets a different value
# (e.g. the Vite :5173 origin). Add REGISTRATION_OPEN=true to open sign-up —
# compose always passes the variable, and an empty value means closed.
ENVIRONMENT=development WEBAUTHN_ORIGIN=http://localhost:8080 docker compose up -d
```

## Repository Layout

- `backend/` — Rust crate `matrix-bot`: Matrix bot + embedded Axum REST API.
- `inference/` — Rust crate `matrix-bot-inference`: Axum server hosting local ONNX models (OCR, STT, Kokoro TTS).
- `contracts/` — JSON fixtures each crate asserts its wire formats against.
- `frontend/` — React 19 + Vite + Tailwind v4 SPA, built into `backend/web/dist/`.
- `data/` — Matrix crypto store (gitignored); the application database lives in PostgreSQL, not here.
- `docs/` — `ARCHITECTURE.md` (design rationale); `testing-auth-locally.md` covers running passkeys against `localhost` (virtual authenticator, the `!link` DM flow, SQL to skip the step-up and recovery delays); roadmap, reviews.

## Engineering Principles

Design the boundaries carefully, keep the domain deterministic, use Rust's type system to enforce invariants, make concurrency and failure explicit, and keep implementation complexity proportional to the actual problem.

- **Architecture and domain first** — plan business goals and use cases, domain boundaries, crate/module responsibilities, dependency direction, data ownership, request/event flows, and persistence/external-service boundaries before writing code; the architecture should be complete even where the implementation stays restrained. Keep business rules in the domain/application layer, never in Axum handlers, Matrix event handlers, SQL, or UI code. Don't add an abstraction for a single use case — wait for a second genuine one to justify it.
- **Rust-first design** — write idiomatic Rust, not patterns translated from another language. Make invalid states unrepresentable: enums over stringly-typed state, newtypes for identifiers, `Result`/`Option` for fallibility, immutable data by default, exhaustive `match`. Avoid unneeded `Arc<Mutex<_>>`, excess `clone()`, `unwrap()`/`expect()` on runtime paths, global mutable state, and `Box<dyn Trait>` where a concrete type or generic is clearer. A trait earns its place at a real architectural boundary, not for mocking alone.
- **Crate and module boundaries** — see the target layer-first layout under Architecture below for where this is headed. Regardless of layout: domain code never depends on Axum, Matrix SDK, SQLx, or an HTTP client; API and database models never leak into the domain; Matrix events and other external responses are validated and translated into application/domain commands at the boundary; module/crate dependencies stay acyclic; public module APIs stay small.
- **Data boundaries** — keep transport models, validated input, application commands, domain models, and persistence models distinct; never pass a database row or an HTTP DTO through the whole application unchanged. Every boundary crossing validates, normalizes, authorizes, converts, and maps errors. Avoid sharing mutable state across architectural boundaries.
- **Domain and business logic** — keep domain logic deterministic and independent of transport/persistence, expressed as explicit domain commands and results, and reused rather than re-derived in each adapter. Never put business logic in Axum handlers, Matrix handlers, SQL, serialization impls, or background-task closures. Don't mark a domain operation `async` unless it actually does I/O.
- **Error handling** — structure errors by boundary (validation, not-found, unauthorized, forbidden, conflict, database, external-service). Never silently swallow an error, retry unboundedly, or collapse everything into a generic "internal error"; preserve diagnostic context internally and map infrastructure errors to application errors at the boundary, but never leak implementation details to a client. `thiserror` for library/application error types, `anyhow` for application-level propagation.
- **Async and concurrency** — before spawning a task, decide its ownership, cancellation, lifetime, backpressure, ordering, and cleanup. Prefer message passing over shared mutable state; never hold a lock across `.await`; never block a Tokio worker thread directly — use `spawn_blocking`. Bound every queue/channel/worker/concurrent-request count, give every background task a shutdown strategy, every external call a timeout, and every retry a bound that respects idempotency. Don't assume concurrent operations execute in a particular order unless it's explicitly guaranteed.
- **Idempotency and transactions** — before writing a mutation that can be retried, duplicated, or run concurrently (Matrix events, webhooks, background jobs, reminders, AI tool calls, database writes, external requests), decide what happens on double-execution, partial success, a mid-operation crash, and concurrent writers. Size a transaction to the smallest atomic business operation, not to however many statements happen to run together.
- **Security by default** (beyond Security Rules below) — design for multi-tenant isolation even at small scale; every externally supplied value is untrusted. Apply least privilege. Never let a client-supplied id imply ownership — every query enforces the caller's own scope. Guard against resource exhaustion and apply rate limiting on new endpoints. Treat SSRF as a general risk for any outbound request built from user input, not only the AI transport's. Logs must never leak a secret alongside the payloads they already exclude.
- **External services and adapters** — isolate Matrix, LLM providers, MCP, STT/TTS/OCR, web search, and any future external system behind an adapter; the domain describes what it needs, the adapter describes how the external system provides it. Assume they are slow, unavailable, or wrong: explicit timeouts, response validation, rate-limit and cancellation handling, bounded retries, and enough recorded diagnostic context to troubleshoot a failure. Don't leak a provider's own types into the domain, and don't make an external service mandatory for functionality that doesn't need it.
- **AI/agent boundaries** — the LLM is an untrusted orchestration component, not a source of truth; its output reaches application state only through a validated, independently authorized tool call into a deterministic application service, never a direct mutation. Agent retries and tool loops need explicit limits, timeouts, and cancellation; keep agent orchestration out of domain services, and prefer a deterministic tool over letting an agent manipulate infrastructure directly. A destructive operation needs explicit confirmation semantics where appropriate, and well-formed AI-generated data is still not authoritative merely for being well-formed.
- **Observability** — every important operation should be diagnosable via structured logging/tracing carrying a request/task id, user id (when safe), operation name, duration, outcome, retry count, external dependency, and database transaction outcome — never a token, password, API key, session secret, or full sensitive payload. An error should be traceable from the public boundary down to the underlying failure.
- **Testing strategy** — match the test level to what's being verified (unit → domain/application → repository/database → API/integration → end-to-end), and prioritize business invariants, authorization, data isolation, state transitions, transactions, idempotency, concurrency-sensitive paths, error handling, and public contracts over chasing a coverage number. For a bug fix: reproduce it, add a regression test, confirm it fails before the fix, fix it, then run the relevant broader suite.
- **Dependencies** — before adding a crate, check the workspace's existing dependencies first, verify the candidate's current API, documentation, maintenance status, and Rust-version compatibility, weigh compile time/binary size/transitive impact, and add the smallest one necessary. Don't hand-roll what the standard library or an existing workspace dependency already does, and don't add a second dependency for a capability one already in the workspace provides.
- **Focused files and functions** — see File organization below for the line-count rule; functions should have one responsibility, never parsing input, authorizing, executing business logic, running SQL, formatting a response, and calling an external service all at once. Extract responsibilities at architectural boundaries, not into arbitrary helper functions.
- **Comments and ADRs** — comments explain *why*, not what the code already says: non-obvious design decisions, invariants, compatibility constraints, concurrency assumptions, external-service limitations, workarounds, known defects, security considerations. A TODO names an issue and a removal condition — bad: `// TODO: fix this later`; good: `// TODO(#123): Replace polling with Matrix sync notifications once upstream supports it.` Record real architectural decisions as ADRs.
- **Deletion over compatibility** — delete obsolete implementations, dead code, unused dependencies, and obsolete config/database paths rather than keeping deprecated shims, dual-write logic, or dead fallback paths. Public contracts are the one place compatibility is a deliberate, evaluated decision rather than a default.
- **Change discipline** — understand the existing architecture and affected boundaries before changing them, make the smallest correct change, remove what it obsoletes, add or update tests, run the relevant quality gates, and review the final diff for anything unintended. Don't mix unrelated refactoring into a feature or bugfix change.
- Grow the system in layers: start from the smallest version that works end to end, and add each new capability on top of a product that already works — never trade a working product for unfinished complexity.
- Make architectural decisions for the long term; do not accept a stopgap that only works for now and is meant to be replaced later.

## Conventions and Rules

**`BotModule` plugin system** — features are independent modules implementing `BotModule` (`backend/src/modules/mod.rs`), registered in `backend/src/interface/channels/matrix/mod.rs`. Never add command handling to `main.rs` or `interface/channels/matrix/handler.rs`.

**Embedded Axum REST API** — endpoints live in `backend/src/interface/http/`, aggregated via `api_routes()` in `interface/http/routes.rs`. Use `ApiError` (`backend/src/error.rs`) for every response (`{ "error": "...", "code": 400 }`). Every mutation must `broadcaster.send(SystemEvent::…)`, and **every `SystemEvent` variant must return a meaningful `owner_id()`** — the broadcast channel is process-wide and per-owner SSE delivery depends on it; a variant with no owner leaks across users or is never delivered. Cap API payload strings at 500 chars.

**File organization** — no file in `backend/src/` or `inference/src/` exceeds 500 lines of production code (target < 300; `#[cfg(test)]` modules don't count). Split along a real seam, never by line count alone. Feature module shape: `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`, optional `scheduler.rs`.

**`infrastructure/` boundary** — `infrastructure/` never imports from `modules/`, `interface/` or `application/`. `modules/` is what the app does; `infrastructure/` is how it reaches external capabilities. A wire type and a domain type that share fields stay separate, with a `From` impl in the module.

**`contracts/`** — every HTTP wire contract between the two crates lives here as a JSON fixture both sides assert against (or the server side alone, for a server-only format) — a renamed field fails a test, not production.

**Public contracts** — REST endpoints, OpenAPI, Matrix commands/events, MCP interfaces, the database schema, auth behavior, and externally consumed config are explicit contracts. Before changing one: identify its consumers, decide whether the change is breaking, update the schema/source of truth, regenerate derived artifacts (OpenAPI, `contracts/` fixtures, frontend API types), update tests, update documentation, and verify migration/rollback behavior. Never add a compatibility shim for a purely internal refactor — for a public contract, compatibility is an intentional decision, not a side effect.

**Database & SQLx**:
- Treat the database as an infrastructure boundary: convert rows into domain types at the repository edge rather than letting a SQLx row type escape it, keep SQL close to the repository implementation that issues it, prefer compile-time checked queries where practical, and make indexes, constraints, foreign keys, uniqueness, and transaction semantics explicit — a constraint that matters must not depend solely on application code. Never hand-edit the schema outside a migration; review a migration's ordering and rollback implications before applying it; keep a data-rewriting migration separate from plain DDL; never silently change the meaning of historical data.
- New and converted code uses compile-checked `sqlx::query!` / `query_as!` / `query_scalar!`. Builds read the committed `.sqlx/` cache (`SQLX_OFFLINE=true` via `.cargo/config.toml`). After changing a query or a migration, regenerate with `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` against a migrated database and commit `.sqlx/`; CI fails on drift — create a scratch database and run `sqlx migrate run --source backend/migrations` against it first, or `prepare` has nothing to check queries against. The `db_*!`/`tx_*!` macros remain only in code not yet converted — do not add new uses. Those interim macros wrap the runtime-built SQL in `sqlx::AssertSqlSafe`, which is sound only because every value is bound and every dynamic fragment is a compile-time constant or an allow-listed identifier (e.g. `OWNED_TABLES`), never user input; new code uses `query!`.
- **Never put a value into SQL text; always bind it**, even when assembling SQL dynamically — `db_fetch_all_dyn!` (e.g. `modules/bookmarks/repository.rs`) builds the clause with `push_str` but still binds every value as a parameter.
- **Match the column type to the Rust type it is read as**, and check both directions when changing either:

  | Rust | Column |
  |---|---|
  | `i64` | `BIGINT` |
  | `i32` | `INTEGER` (PostgreSQL's `INTEGER` is `i32`, not `i64`) |
  | `f64` | `DOUBLE PRECISION` (`REAL` is `float4`) |
  | `bool` | `BOOLEAN` |
  | `DateTime<Utc>` | `TIMESTAMPTZ` (bare `TIMESTAMP` carries no zone) |
  | `String` holding a timestamp | `TEXT` with a `DEFAULT (NOW()::text)` |

  **Change column types one at a time, against the field that reads them** — three rounds of defects came from blanket `INTEGER`→`BIGINT` / `TIMESTAMP`→`TIMESTAMPTZ` sweeps.
- `TIMESTAMPTZ` holds microseconds, `DateTime<Utc>` holds nanoseconds — build test values with `utils::time::now_at_storage_precision`, not `Utc::now()`.
- Schema changes are forward-only SQL files in `backend/migrations/NNNN_description.sql`, applied at boot by `sqlx::migrate!`. Never edit a committed migration. A data- or type-rewriting migration starts with a guard that names offending rows, and is rehearsed on a production copy (`migrates_a_real_database`) before release.
- Room caching/resolution lives only in `db/rooms.rs` — never couple domain repositories or Matrix channel modules to HTTP interface modules.
- **Any new owner-keyed table must be added to `OWNED_TABLES`** (`accounts/merge.rs`) — a merge rewrites every table that constant lists; one missing from it isn't a compile error, it's a silent data leak on the next account merge.

**Matrix response & formatting** — always reply with `text_html(&plain, &html)`. Use `html_escape()` (`backend/src/utils/html.rs`) for all user-supplied input embedded in HTML — it escapes `<`, `>`, `&`, `"`, `'`; use `strip_html()` for the plain-text fallback, so push notifications don't render raw HTML tags. Quoted-message context (`CommandContext.quoted`): Matrix replies (rich `<mx-reply>` HTML and plain `> quote` fallbacks) are parsed in `interface/channels/matrix/quote.rs` into `QuotedMessage` — AI injects it into prompts via `QuotedMessage::to_context_prompt()`; Todo/Reminder use it as the task/reminder body when none is supplied.

**AI tools & web search** — define tools with the standard OpenAI `type: "function"` schema (`modules/ai/search.rs`); never a provider-specific tool variant. Each domain tool file exports `pub fn tool_definitions() -> Vec<ToolDefinition>`; `definitions.rs` just aggregates them, and tool executors call domain SQLx repositories directly rather than through pass-through layers. Gate web search by intent (`should_enable_web_search()`) — skip it for routine productivity commands. Search backend: Tavily (`TAVILY_API_KEY`) first, falling back to keyless DuckDuckGo scraping if no key is set or on API errors. Execute concurrent tool calls with `futures_util::future::join_all`, not a sequential loop. Long research requests need a generous router timeout (`TimeoutLayer` >= 180s); dedupe immediate retries by checking the conversation's tail message before inserting.

**AI transport configuration** — settings read exactly one env name each: `AI_ENDPOINT` (base URL incl. `/v1`), `AI_API_KEY`, `AI_MODEL`. Per-user base URLs are an SSRF surface: `AI_ALLOW_PRIVATE_BASE_URL` (off by default) is the only way to allow one that resolves to a private host. The AI transport's own proxy chain (`config.rs`) is `AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy` → `HTTP_PROXY`/`http_proxy`. The web-search HTTP client has a **separate, shorter** chain (`modules/ai/search.rs`) — `AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy`, with no `HTTP_PROXY` fallback — don't assume the two share one list. Web-search key: `TAVILY_API_KEY`.

**Frontend design system** (`Design.md`) — monochrome-first: canvas `bg-white dark:bg-neutral-950`, borders `border-neutral-200 dark:border-neutral-800`, text `text-neutral-900 dark:text-neutral-100`/`text-neutral-500 dark:text-neutral-400`; color only for meaningful state, always paired with a non-color cue. Geist Sans for prose/controls, Geist Mono for identifiers (code, paths, timestamps, currency codes); sentence-case headings, no all-caps overlines. `AppShell` already provides the outer gutters (`px-6 md:px-8 py-6`) — module roots are `<div className="space-y-6">` (or the full-height chat/map variant `flex-1 flex flex-col min-h-0 space-y-4`), never redundant padding or a centering container. No decorative gradients, glow, glassmorphism, stripes, textures, or card-in-card nesting; no pulsing badges or simulated animation — transitions are `transition-colors duration-150`. Stat Strip pattern (`grid grid-cols-2 sm:grid-cols-4 gap-3`) with uppercase mono labels and tabular numbers; tables own the full content width, numbers right-align. Every control has visible labels, clear focus outlines, and full keyboard access (`Escape` closes modals, `Enter` submits single-line forms).

**Timezone-aware rendering** — always pass the zone explicitly when formatting an instant (`formatDateTimeInZone` and friends); never use a bare `toLocaleString`, which silently uses the browser's zone instead of the user's saved one.

## Module Implementation Checklist

When creating or enhancing a feature module:

1. **Backend directory**: `backend/src/modules/<feature_name>/` with `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`.
2. **Database migration**: a new `backend/migrations/NNNN_<feature>.sql`; if the table is owner-keyed, add it to `OWNED_TABLES` in `accounts/merge.rs` or a merge silently drops it.
3. **Repository**: SQLx query binding CRUD methods in `repository.rs`.
4. **Command handler**: parse Matrix commands, enforce length limits (max 500 chars), format HTML output.
5. **REST API**: handlers in `backend/src/interface/http/<feature_name>.rs` returning `Result<_, ApiError>`, registered in `backend/src/interface/http/routes.rs::api_routes()`. If a route is sensitive, add `VerifiedUser` to the handler **and** add it to the `SENSITIVE` list in `backend/src/interface/http/mod.rs::step_up_tests` — that test only checks routes already listed, so it won't catch a new one on its own.
6. **SystemEvents**: variants in `backend/src/events.rs` with `owner_id()`, broadcast on every state change.
7. **BotModule registration**: implement `BotModule` in `mod.rs`, register in `backend/src/interface/channels/matrix/mod.rs`.
8. **OpenAPI & types**: update `ApiDoc` in `backend/src/interface/http/openapi.rs`, then run the OpenAPI regeneration commands above.
9. **Frontend**: `frontend/src/modules/<feature_name>/` exporting `<feature>Module: FrontendModule`; register in `frontend/src/App.tsx`; nav item in `Sidebar.tsx`; label in `AppShell.tsx`.
10. **Tests**: backend unit tests in `repository.rs`/`handler.rs`/`interface/http/<feature_name>.rs` using `crate::db::pool::test_pool()` + `crate::db::migrate::run_migrations_unlocked`; mock routes in `frontend/e2e/fake-api.ts`; Playwright spec in `frontend/e2e/<feature_name>.spec.ts`.

## Architecture

One- to three-sentence summaries; full rationale is in `docs/ARCHITECTURE.md`.

- **Single process, two front doors** — `main.rs` boots one `Config`/`Database`/`EventBroadcaster` and spawns the Axum API server and the Matrix sync loop, coupled through shared handles (bot token, Matrix client, pending merges). [Details](docs/ARCHITECTURE.md#single-process-two-front-doors).
- **Backend module plugin system** — features are `BotModule`s dispatched in registration order by `ModuleRegistry`; background work runs through `spawn_background_workers`. [Details](docs/ARCHITECTURE.md#backend-module-plugin-system).
- **Event flow (SSE)** — every mutation broadcasts a `SystemEvent`, delivered per-owner via `owner_id()`. [Details](docs/ARCHITECTURE.md#event-flow-sse).
- **Matrix session and crypto store** — never keep the crypto store across a fresh login, and never pin a device ID on one, or peers see broken encryption or a silent key rotation. [Details](docs/ARCHITECTURE.md#matrix-session-and-crypto-store).
- **Verification** — `ensure_self_verified` must run on every boot; never treat `Recovery::recover()`'s `Ok` as "verified". [Details](docs/ARCHITECTURE.md#verification-self-and-peer).
- **Accounts and ownership** — `owner_id` on every table is an `accounts.id`, resolved from a Matrix link or a web session, never derived any other way. A merge rewrites every table in `OWNED_TABLES`, and sessions never follow a merge — a session belongs to the credential that opened it. [Details](docs/ARCHITECTURE.md#accounts-one-owner-several-ways-in).
- **Auth: passkeys and sessions** — passkeys are the only way to start a browser session; a sensitive route needs `VerifiedUser` **and** an entry in `step_up_tests`'s `SENSITIVE` list (`interface/http/mod.rs`) — the test checks only routes already listed, so both are required. An MCP token never satisfies step-up. [Details](docs/ARCHITECTURE.md#auth-passkeys-and-cookie-sessions).
- **Linking and recovery** — `!link` must be handled before the sender is resolved (never move it into a `BotModule`), or the claim provisions an empty account first. `!recover` is time-fenced; cancelling it deliberately takes `AuthUser`, not `VerifiedUser`. `WEB_ONLY` refuses `!export`/`!sessions`/`!recoverycodes`/`!unlink`; `!passkey add` enrolls only an account's first passkey. [Details](docs/ARCHITECTURE.md#linking-recovery-and-the-matrix-ceiling).
- **Per-user settings** — timezone/currency, read by every other module; a missing row means defaults, never an error. [Details](docs/ARCHITECTURE.md#per-user-settings-timezone-default-currency).
- **Voice replies** — `!say` synthesises through `infrastructure/inference/tts`; the reply is Ogg Opus (not WAV) so clients draw a voice bubble. [Details](docs/ARCHITECTURE.md#voice-replies-say).
- **Two crates, services and inference** — `backend/` and `inference/` are separate crates joined by `backend/src/infrastructure/inference/` and `contracts/`; `default-members = ["backend"]` keeps `cargo run`/bare `cargo test` scoped to the bot. [Details](docs/ARCHITECTURE.md#two-crates-services-and-inference).
- **Portfolio** — cash is a hard precondition for buying, never for selling; the holding write and the cash write happen in one transaction. [Details](docs/ARCHITECTURE.md#portfolio-investment-holdings).
- **Persistence and migrations** — `db/migrate.rs::run_migrations` runs `sqlx::migrate!` over versioned SQL files in `backend/migrations/`; `0001_baseline.sql` is an idempotent adoption of the live database's shape, and every migration after it is forward-only. [Details](docs/ARCHITECTURE.md#persistence-and-migrations).
- **PostgreSQL only** — `DATABASE_URL` is required and must be `postgres://`; matrix-sdk keeps its own SQLite crypto store under `data/`. [Details](docs/ARCHITECTURE.md#postgresql-only).
- **Target layer-first layout (in progress)** — the design target is `domain/ → application/ → interface/{http,chat,channels,mcp}, llm/, jobs/, events/ → infrastructure/{postgres,inference}`, with `interface/` playing the role a generic `api/` + `adapters/` split would; `domain` depends on nothing but std/chrono/uuid/serde/thiserror. The former `api/`, `bot/` and `services/` directories are now `interface/http/`, `interface/channels/matrix/` and `infrastructure/inference/`; `domain/` and `application/` roots arrive next. The move off today's `modules/` layout happens feature by feature, so the `BotModule` plugin rules above remain the real rules for every feature that hasn't moved yet. [Details](docs/ARCHITECTURE.md#target-layer-first-layout).
- **Frontend module mirror** — the SPA mirrors the backend's module shape; `core/session.ts` is the single source of "who's signed in" since the cookie is unreadable by the page. [Details](docs/ARCHITECTURE.md#frontend-module-mirror).

## Testing and Verification

```bash
cargo fmt --all -- --check                        # CI: backend + inference jobs both block on this
cargo test -p matrix-bot --all-targets             # CI: backend job
cargo test -p matrix-bot-inference --all-targets   # CI: inference job
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # CI's drift check: regenerate, commit the file if it changed, then this must exit 0
npm --prefix frontend run build                    # CI: frontend job
npm --prefix frontend run test:e2e                 # not CI-gated yet; still required before shipping a UI change
```

CI (`.github/workflows/ci.yml`) runs three jobs: `backend` (format, tests, `live_postgres --ignored` tests against a real PostgreSQL 18, OpenAPI drift — all blocking), `inference` (format + tests), `frontend` (build only). See "Commands" above to run the Postgres suite locally.

Never commit code that fails compilation, formatting, or tests.

**Quality gates for changed code** — `cargo fmt --all -- --check` and the test commands above (which also compile every target, so there's no separate CI `cargo check` step) are required, and CI-enforced except for `npm --prefix frontend run test:e2e`, which is required before shipping a UI change but not yet run in CI. `cargo clippy -p matrix-bot --all-targets -- -D warnings` (and `-p matrix-bot-inference`) is a required gate for any Rust you touch but is **not yet enforced in CI** — a backlog of 102 warnings (101 `matrix-bot`, 1 `matrix-bot-inference`) predates this rule and is tracked as follow-up; don't add new ones in files you change, and don't silence one with `#[allow]` without documenting why it exists, why it's safe, its scope, and when it can be removed. For a release-sensitive change, also run `cargo build -p matrix-bot --release`.

**Checklist by change type** — beyond the standard fmt/check/test/clippy pass:
- *Schema change*: add `backend/migrations/NNNN_description.sql`, run the backend suite, and if it rewrites or retypes existing data, rehearse it against a restored production copy with `migrates_a_real_database` (see Commands) before shipping.
- *API/OpenAPI change*: update the handler/model, regenerate `backend/openapi.json` and the frontend client (`npm --prefix frontend run generate-api`), update `contracts/` fixtures and tests, and re-check auth/step-up behavior on the route.
- *AI/agent change*: validate tool arguments, verify authorization independently of the model, and test malformed model output, tool failure, timeouts, cancellation, retry limits, and duplicate tool calls; verify destructive operations and confirm domain invariants cannot be bypassed.
- *Before submitting*: `git diff --check`, review `git status`/`git diff` for unrelated changes, generated artifacts, debug code, or secrets.

## Security Rules

- Sensitive API routes require `VerifiedUser` **and** an entry in `step_up_tests`'s `SENSITIVE` list in `interface/http/mod.rs` — the test only checks routes already listed there, so adding `VerifiedUser` alone isn't enough to be caught by CI (see Architecture: Auth). An MCP token never satisfies step-up.
- Never put a value into SQL text — always bind it, even in dynamically assembled queries (see Conventions: Database & SQLx).
- Escape all user input embedded in HTML with `html_escape()`.
- Every `SystemEvent` variant must return a meaningful `owner_id()`, or it leaks across users or is never delivered.
- Matrix has no route to a privileged action. `WEB_ONLY` in `modules/account/mod.rs` refuses `!export`, `!sessions`, `!recoverycodes` and `!unlink` with a pointer to the web app. `!passkey add` enrolls only an account's **first** passkey: both the bot and `EnrollmentRepository::validate_and_consume` refuse once one exists, because otherwise Matrix alone could skip the `!recover` fence.
- `!link` is refused outside a DM, and **the code is burned either way** — a public-room code is readable by anyone.
- Only the SHA-256 of a session cookie is stored server-side.
- CSRF is enforced twice: `SameSite=Strict` plus an `Origin` check on every cookie-authenticated mutation. A missing `Origin` on such a mutation is refused; a request carrying an explicit `Authorization: Bearer` credential is exempt.
- `Config::validate` refuses to boot production on localhost WebAuthn defaults.
- API payload strings are capped at 500 chars.
- Any new owner-keyed table must be added to `OWNED_TABLES` (`accounts/merge.rs`), or an account merge silently leaves its rows behind.

## Gotchas

- Cutting over an existing production deployment to this branch (PostgreSQL-only, versioned migrations, matrix-sdk 0.18) follows `docs/runbooks/postgres-foundation-cutover.md` — production rehearsal, pre-boot steps, first-boot watch list, and rollback.
- `DATABASE_URL` has no default and must be `postgres://`; `interface/http/mod.rs` probes `web/dist` then `backend/web/dist`. The latter assumes `cargo run` from the repo root.
- Bare `cargo run` resolves to the bot only via `default-members = ["backend"]` + `default-run = "matrix-bot"` in `backend/Cargo.toml` — keep both if you add a workspace member or a third binary.
- `frontend/vite.config.ts` proxies `/api` to `:8080`, matching `API_PORT`'s default. Change one, change the other.
- Windows: a cold `openssl-sys` build needs a full Perl on `PATH` first (e.g. Strawberry or scoop `perl`) — Git for Windows' bundled Perl lacks `Locale::Maketext::Simple`.
- Windows: loading the real Kokoro TTS model in a test needs sherpa's own `onnxruntime.dll` copied into `target/debug/deps/` — see that test's own comment in `inference/src/kokoro.rs` for why a version mismatch there dies with an access violation rather than a clean error.
- `.env` is read from the current working directory only (`main.rs` loads `./.env` via `dotenvy::from_path`, never dotenvy's own upward parent-directory search) — always run the bot binary from the repo root, and note that a git worktree checkout has its own `.env`, distinct from the main checkout's.

## Definition of Done

A change is not complete merely because it compiles. Before considering it complete:

- [ ] Architecture and dependency direction remain correct; domain logic stays transport/infrastructure-independent.
- [ ] Input validation, authorization, and data isolation are enforced.
- [ ] Errors are handled explicitly; concurrency and cancellation behavior are understood.
- [ ] Database transactions and invariants are correct.
- [ ] Public contracts remain intentional and documented.
- [ ] Tests cover the important behavior; no obsolete implementation remains.
- [ ] Logs carry sufficient diagnostic context without sensitive data.
- [ ] `cargo fmt`, `cargo check`, and `cargo test` pass; `cargo clippy` passes on the code you touched.
- [ ] Documentation, ADRs, and technical-debt references are updated where necessary.

## Commits and PRs

Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`; `!` for a breaking change, e.g. `refactor!:`).

Two remotes: `gitea` (`git.mbm.one`) is the primary forge — push and open pull requests there; `origin` (GitHub) is a mirror.
