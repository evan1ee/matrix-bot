# AGENTS.md

Rules for coding agents working in this repository. This file says **how to work and what must not be done**; it is not project documentation. Design rationale and reference material live in `docs/`, and this file only points there.

| Need | Read |
|---|---|
| Architecture decisions, layers, Matrix/accounts/settings design | [`docs/architecture.md`](docs/architecture.md) |
| Database conventions, migrations, SQLx, test databases | [`docs/database.md`](docs/database.md) |
| REST/SSE contract, authentication, webhooks, OpenAPI workflow | [`docs/api.md`](docs/api.md) |
| LLM, tool calling, agent limits, MCP | [`docs/llm.md`](docs/llm.md) |
| Plans, designs, progress, hand-off state, runbooks | [`docs/superpowers/`](docs/superpowers/) — start with [`HANDOFF.md`](docs/superpowers/HANDOFF.md) |
| Frontend design system | [`Design.md`](Design.md) |
| Product roadmap (all phases) | [`SPEC.md`](SPEC.md) |
| Setup, deployment, command reference | [`README.md`](README.md) |

> **Refactor in progress.** This branch is mid-way through the layer-first PostgreSQL refactor. Before any change, read [`docs/superpowers/HANDOFF.md`](docs/superpowers/HANDOFF.md): status, next actions, settled decisions and hard safety rules.

```text
                          AGENTS.md
                               │
        ┌──────────────────────┼──────────────────────┐
        ▼                      ▼                      ▼
   Architecture            Workflow              Constraints
 how code is organised   how to develop        what must not be done
        └──────────────────────┼──────────────────────┘
                               ▼
              Coding agent → Implementation → Verification
```

## 1. Project context

`matrix-bot` is a personal AI/life workspace built around Matrix. One Rust Cargo workspace and one React SPA:

- `backend/` — crate `matrix-bot`: Matrix bot, embedded Axum REST/SSE API, MCP server, LLM integration, domain features (todos, reminders, finance, portfolio, bills, habits, bookmarks, life/journal, footprints, webhooks, settings, accounts and passkey auth).
- `inference/` — crate `matrix-bot-inference`: Axum server hosting local ONNX models for OCR, speech-to-text and text-to-speech.
- `frontend/` — React 19 + Vite + Tailwind v4 SPA, built into `backend/web/dist/`.
- `contracts/` — JSON fixtures each crate asserts its wire formats against.
- PostgreSQL 18 is the only application database; matrix-sdk keeps its own SQLite crypto store under `data/`.

The backend is the source of truth for application state. It is a **modular monolith**: do not introduce microservices, message queues, event buses, caches or distributed infrastructure without a demonstrated requirement. Grow in layers: start from the smallest version that works end to end, add each capability on top of a product that already works, and never trade a working product for unfinished complexity. Make architectural decisions for the long term; do not accept a stopgap that is meant to be replaced later.

## 2. The five rules

1. **Inspect before editing.** Read the affected module, its neighbours, its tests and the relevant doc first.
2. **Business logic belongs in `application/` and `domain/`.** Never in Axum handlers, Matrix handlers, SQL, serialization, MCP handlers, background-task closures or React components.
3. **All interfaces share the same application capabilities.** Matrix, REST, MCP, AI tools and the web UI call one service; no interface re-derives a rule.
4. **Avoid speculative abstractions and infrastructure.** A trait, factory, registry or worker needs a current second use or a real boundary.
5. **Never claim completion without verification.** Run the gates in §8 and report their actual results.

```text
                    ┌── Matrix (BotModule)
                    ├── REST API + SSE
 LLM / User ───────►├── MCP
                    ├── Web (SPA over REST)
                    └── AI tools
                           │
                    application  (<F>Service)
                           │
                        domain
                           │
                       PostgreSQL
```

## 3. Architecture

### 3.1 Layers

Layer-first layout under `backend/src/`; dependency direction runs one way, enforced by `backend/tests/architecture.rs`.

```text
domain/<f>/                  entities, value objects, invariants, domain errors (pure, synchronous)
application/<f>/             <F>Service: use cases, transactions, events after commit
                             application/mod.rs: AppError, AppServices
interface/http/              Axum handlers + DTOs (OpenAPI types)
interface/chat/              BotModule command handlers per feature
interface/channels/matrix/   matrix-sdk client, sync loop, E2EE, verification
interface/mcp/               MCP protocol adapters
llm/                         prompts, tool loop, tool registry (llm/tools/)
infrastructure/postgres/     repositories/<f>.rs, read_models/, merge.rs, ping
infrastructure/inference/    HTTP clients: OCR, STT, TTS, LLM transport
infrastructure/web/          outbound web feeds, SSRF guard
app.rs / main.rs             composition roots
db/                          pool, migration runner, test pools and the schema catalog test
accounts/                    legacy facades over the accounts repositories (being retired; do not extend)
```

| Layer | Owns | May not import |
|---|---|---|
| `domain` | rules and invariants | anything but `std`, `chrono`, `uuid`, `serde`, `thiserror`; no Axum, matrix-sdk, sqlx, reqwest, utoipa, React, MCP |
| `application` | use cases, transactions | axum, matrix-sdk, reqwest, utoipa, `interface` |
| `interface`, `llm` | protocol translation, AI orchestration | SQL of any kind outside `#[cfg(test)]` (the architecture test enforces this for `interface/` and `application/`; keep `llm/` to it too) |
| `infrastructure` | external capabilities | `application`, `interface` |

- Before adding a module, check whether an existing one already owns the responsibility. Do not create new top-level layers without a clear reason and an ADR.
- **Interface layer** parses input, authenticates, resolves the caller's scope, validates protocol-level shape, calls a service, formats the reply. **Application layer** orchestrates, owns transaction boundaries, coordinates repositories and external services. **Domain** holds pure rules; if a feature is simple CRUD with no meaningful rules, keep the design simple rather than inventing domain abstractions. **Infrastructure** implements how PostgreSQL, Matrix and external services are reached.
- Keep transport models (DTOs), validated input, application commands, domain models and persistence rows distinct. Never pass a database row or HTTP DTO through the whole application unchanged; every boundary crossing validates, normalizes, authorizes, converts and maps errors. A wire type and a domain type that share fields stay separate, with a `From` impl.
- `AppServices` is built once per process by a composition root and passed to adapters as a parameter (`ApiState.services`, `ModuleContext.services`, `handle_command(ctx, services, …)`, the AI turn). No adapter constructs a service.
- `infrastructure/` never imports from `interface/` or `application/`.

### 3.2 Avoid premature abstraction

Prefer a concrete implementation over `Trait → Factory → Registry → Provider → Concrete`. Abstract only for a second real implementation, testability at a real boundary, dependency inversion, an external-service boundary or a significant domain boundary. Repositories are plain functions over `impl sqlx::PgExecutor`, not traits. There is deliberately no per-vendor LLM provider enum. Do not add an abstraction for a single use case; wait for the second one.

### 3.3 Rust-first design

Write idiomatic Rust, not patterns translated from another language. Make invalid states unrepresentable: enums over stringly-typed state, newtypes for identifiers (`AccountId`), `Result`/`Option` for fallibility, immutable data by default, exhaustive `match`. Avoid needless `Arc<Mutex<_>>`, excess `clone()`, `unwrap()`/`expect()` on runtime paths, global mutable state and `Box<dyn Trait>` where a concrete type or generic is clearer. Do not mark a domain operation `async` unless it does I/O.

### 3.4 Plugin and feature shape

Chat features are independent `BotModule`s (`interface/chat/mod.rs`), registered in `interface/channels/matrix/mod.rs`; registration order matters. Never add command handling to `main.rs` or `interface/channels/matrix/handler.rs`. `!link` is handled before the sender is resolved and must never move into a `BotModule`.

## 4. Workflow

### 4.1 Feature development

1. **Understand.** Inspect the module, domain objects, repositories, API endpoints, Matrix handlers, schema and tests. Do not start coding immediately.
2. **Plan.** For a non-trivial feature identify the interaction path (interface → service → domain → repository → PostgreSQL), data changes, API changes, migration, authentication/authorization, tests and compatibility. **Plans and specs go in `docs/superpowers/`**: designs in `specs/`, plans in `plans/`, named `YYYY-MM-DD-topic.md`. A design needs the user's approval before its plan; a plan exists before code.
3. **Implement vertically**: one feature at a time through `domain → application → persistence → API/chat → tests`, not one architectural layer across every feature. Start with a failing test.
4. **Verify** with the gates in §8.
5. **Record progress** in `docs/superpowers/HANDOFF.md` in the same commit as the work, so another agent can take over: status table, rulings (`Ruling: <decision> — <why> — <cost if wrong>`), next actions. Progress lives in git, never in an ignored scratch folder.
6. **Report** (§10).

Decide ambiguities yourself using the design doc as authority and write the decision down. Stop and ask the user only for irreversible actions, security-sensitive actions, anything outside the repository (push, merge, PR, production data, a live Matrix login), or a plan so broken every path forward is a guess.

### 4.2 Feature template

Rooms is the canonical pattern: read `domain/rooms`, `application/rooms`, `infrastructure/postgres/repositories/rooms.rs`, `interface/http/rooms.rs` and `interface/chat/rooms.rs` before starting. For a feature `<f>`:

1. **Domain** `domain/<f>/mod.rs` — entities, validated commands, business rules as synchronous functions unit-tested in the same file. Private fields behind constructors that enforce invariants. No `sqlx::FromRow`, no `utoipa::ToSchema`. A feature error enum (`thiserror`) with `From<…> for AppError` (→ `Validation`) in `application/<f>`. A rule once expressed as SQL becomes a pure function over rows the service fetched (small bounded inputs only); keep tie-breaks exact (first wins, strict `>`, never `max_by`).
2. **Migration** if the schema changes ([`docs/database.md`](docs/database.md)); owner-keyed tables go into `OWNED_TABLES`.
3. **Repository** `infrastructure/postgres/repositories/<f>.rs` — `pub async fn`s over `impl sqlx::PgExecutor<'_>`, compile-checked macros only, private `…Row` structs converted at the edge, round-trip and owner-isolation tests.
4. **Service** `application/<f>/mod.rs` — `<F>Service { pool, events }`, `Clone`, returns `Result<_, AppError>`; opens a transaction only where an invariant needs atomicity; publishes its `SystemEvent` after commit; tests cover mid-transaction failure and events.
5. **Wiring** — add the service to `AppServices` and build it in `AppServices::new`.
6. **HTTP** `interface/http/<f>.rs` — handlers returning `Result<_, ApiError>`, DTOs with `From<&domain>`; register in `routes.rs::api_routes()`; a sensitive route also needs `VerifiedUser` **and** a `SENSITIVE` entry (§7).
7. **Chat** `interface/chat/<f>.rs` — the `BotModule`; cap inputs at 500 chars, call `services.<f>`, reply with `text_html`, turn failures into replies only via `interface::chat::error_reply`. LLM and MCP tools touching `<f>` call `services.<f>` in the same change.
8. **Frontend** (new features only) — `frontend/src/modules/<f>/` exporting `<f>Module: FrontendModule`; register in `App.tsx`; nav item in `Sidebar.tsx`; label in `AppShell.tsx`; mock routes in `frontend/e2e/fake-api.ts`; Playwright spec `frontend/e2e/<f>.spec.ts`.
9. **Clean up** — delete what the change obsoleted; no file over 500 production lines.

### 4.3 Engineering principles

- **Boundaries.** Business rules never live in handlers, SQL, serialization impls or background closures. Adapters translate; services decide.
- **Idempotency and transactions.** Before writing a mutation that can be retried, duplicated or run concurrently (Matrix events, webhooks, jobs, reminders, AI tool calls), decide what happens on double execution, partial success, a mid-operation crash and concurrent writers. Size a transaction to the smallest atomic business operation. Read-modify-write goes through one service method that locks the row (`FOR UPDATE`) in a transaction.
- **Async.** Before spawning a task decide its ownership, cancellation, lifetime, backpressure, ordering and cleanup. Prefer message passing to shared mutable state; never hold a lock across `.await`; never block a Tokio worker (`spawn_blocking`). Bound every queue, channel, worker and concurrent-request count; give every external call a timeout and every retry a bound that respects idempotency. Anything long-lived gets a child `CancellationToken` and joins `app::run`'s shutdown — never a detached task.
- **External services** (Matrix, LLM, MCP, STT/TTS/OCR, web search) sit behind adapters: explicit timeouts, response validation, rate-limit and cancellation handling, bounded retries, recorded diagnostic context. Do not leak a provider's types into the domain, and do not make an external service mandatory for functionality that does not need it.
- **Error handling.** Typed errors per boundary (validation, not-found, unauthorized, forbidden, conflict, database, external). `thiserror` for library/application types, `anyhow` for propagation. Never swallow an error, retry unboundedly, or collapse everything into "internal error". Keep diagnostic context internally; never show a client SQL errors, stack traces, internal paths, secrets or authentication internals.
- **Observability.** Important operations are diagnosable through structured tracing with operation name, duration, outcome, retry count and external dependency, plus a task/request id and user id when safe. Never log a password, session token, recovery code, API key, private key or full sensitive payload.
- **Dependencies.** Check the workspace's existing crates first; verify a candidate's current API, maintenance and Rust-version compatibility; weigh compile time and transitive impact; add the smallest one necessary. Do not hand-roll what std or an existing dependency does, and do not add a second crate for a capability one already provides.
- **Performance.** Correctness → maintainability → measurement → optimization. Do not add caching, queues, background workers, extra pools, distributed locks or microservices for hypothetical scale; identify a measured bottleneck first.
- **Deletion over compatibility.** Delete obsolete implementations, dead code, unused dependencies and dead config paths rather than keeping shims or dual-write logic. Compatibility is a deliberate decision only for public contracts.
- **Comments.** Explain *why*: invariants, compatibility constraints, concurrency assumptions, workarounds, security considerations. A TODO names an issue and a removal condition (`// TODO(#123): … once upstream supports it`), never "fix later". Record real architectural decisions as ADRs.
- **Focused files and functions.** No file in `backend/src/` or `inference/src/` exceeds 500 lines of production code (target < 300; `#[cfg(test)]` modules do not count). Split along a real responsibility seam, never by line count alone. A function has one responsibility; do not parse, authorize, execute business logic, run SQL, format a response and call an external service in one function.

## 5. Interface rules

### 5.1 REST API

Handlers follow `Request → parse/validate → service → response DTO`. Full conventions: [`docs/api.md`](docs/api.md).

- Endpoints live in `interface/http/`, aggregated by `api_routes()`. Return `ApiError` (`{ "error": "...", "code": 400 }`) for every response; map `AppError` through `app_error.rs`; `Database`/`External` become an opaque 500 with the detail logged.
- Cap API payload strings at 500 characters. Apply rate limiting to new unauthenticated or expensive endpoints.
- **Every mutation publishes a `SystemEvent` and every variant returns a meaningful `owner_id()`.** The broadcast channel is process-wide; a variant without an owner leaks across users or is never delivered.
- A change that must not alter the contract must not change `backend/openapi.json`: keep DTO names, fields, optionality, enums, status codes and error strings.

### 5.2 Matrix

Matrix is an interface, not the owner of business logic. A handler receives the event, parses the command, resolves the account, invokes a service and formats the result.

- Always reply with `text_html(&plain, &html)`. Escape every user-supplied value embedded in HTML with `html_escape()` (`backend/src/utils/html.rs`); build the plain-text fallback with `strip_html()`.
- Quoted-message context (`CommandContext.quoted`) is parsed once in `interface/channels/matrix/quote.rs` into `QuotedMessage`; AI injects it via `to_context_prompt()`, Todo/Reminder use it as the body when none is supplied.
- Never keep the crypto store across a fresh login and never pin a device ID on one. `ensure_self_verified` must run on every boot; never treat `Recovery::recover()`'s `Ok` as "verified". See [`docs/architecture.md`](docs/architecture.md).
- `WEB_ONLY` in `interface/chat/account.rs` refuses `!export`, `!sessions`, `!recoverycodes` and `!unlink` with a pointer to the web app. `!passkey add` enrolls only an account's first passkey. `!link` is refused outside a DM and the code is burned either way.

### 5.3 LLM, agents and MCP

The LLM is an untrusted orchestration component. Its output reaches state only through `LLM → tool selection → application capability → domain validation → repository → PostgreSQL`. Details: [`docs/llm.md`](docs/llm.md).

- Define tools with the OpenAI `type: "function"` schema, never a provider-specific variant. Each `llm/tools/<domain>.rs` exports `tool_definitions()`; `definitions.rs` only aggregates; executors call services, never repositories through pass-through layers, and never take an owner from the model.
- MCP tools expose the same registry; no business logic inside MCP handlers. An MCP token never satisfies step-up.
- Agent loops have explicit iteration, timeout, concurrency and cancellation limits and per-call authorization. Do not introduce an agent framework where a controlled loop suffices.
- Gate web search by intent (`should_enable_web_search()`). Settings read exactly one env name each (`AI_ENDPOINT`, `AI_API_KEY`, `AI_MODEL`); per-user base URLs are an SSRF surface guarded by `AI_ALLOW_PRIVATE_BASE_URL`.
- A destructive AI operation needs explicit confirmation semantics where appropriate; well-formed AI output is not authoritative merely for being well-formed.

### 5.4 Frontend

The SPA mirrors the backend module shape (`frontend/src/modules/<f>/`, `core/registry.ts`). Follow [`Design.md`](Design.md): monochrome-first, Geist Sans for prose and Geist Mono for identifiers, sentence-case headings, no decorative gradients/glow/glassmorphism/card-in-card, `transition-colors duration-150`, visible labels and focus outlines, keyboard access (`Escape` closes modals, `Enter` submits single-line forms). `AppShell` provides the outer gutters; module roots are `<div className="space-y-6">`. All API calls go through `core/apiClient.ts`. Always pass the zone explicitly when formatting an instant (`formatDateTimeInZone` and friends); a bare `toLocaleString` silently uses the browser's zone instead of the user's saved one.

## 6. Database rules

PostgreSQL is the production database; the reference is [`docs/database.md`](docs/database.md).

- Schema changes are forward-only SQL files `backend/migrations/NNNN_description.sql`, applied at boot by `sqlx::migrate!`. Never edit a committed migration and never change the schema outside one. A data- or type-rewriting migration starts with a guard that names offending rows, is separate from plain DDL, and is rehearsed on a production copy before release.
- Database access is compile-checked `sqlx::query!`/`query_as!`/`query_scalar!` or `sqlx::QueryBuilder`. **Never put a value into SQL text; always bind it.** `AssertSqlSafe` is allowed only in `infrastructure/postgres/merge.rs`.
- No SQL in `interface/`, `application/`, Matrix, MCP or React code — the architecture test enforces it. Convert rows into domain types at the repository edge.
- Match column type to Rust type (`i64`↔`BIGINT`, `i32`↔`INTEGER`, `f64`↔`DOUBLE PRECISION`, `DateTime<Utc>`↔`TIMESTAMPTZ`, `AccountId`↔`UUID`); change types one at a time against the field that reads them. Build test timestamps with `utils::time::now_at_storage_precision`.
- After changing a query or a migration, regenerate the `.sqlx/` cache against a migrated scratch database and commit it; CI fails on drift.
- **Any new owner-keyed table must be added to `OWNED_TABLES`** (`domain/accounts/mod.rs`), or an account merge silently leaves its rows behind.
- Money and quantities are integer minor units; never floating point.

## 7. Security rules

Ownership and security boundaries are the most expensive thing to get wrong.

- **Authentication identifies; authorization scopes.** Every user-owned resource is scoped to the authenticated account. Never trust a Matrix user id, account id or resource ownership supplied by a client, a request body or a model; resolve ownership from the authenticated context (`AuthUser::owner_id()`). `owner_id` is an `accounts.id`, never derived any other way. No endpoint may reach another user's data without an explicit authorization model. Design for multi-tenant isolation even at small scale.
- **Step-up.** A sensitive route requires `VerifiedUser` **and** an entry in `SENSITIVE` in `interface/http/mod.rs::step_up_tests`; the test checks only routes already listed, so both are required.
- **Sessions and CSRF.** Passkeys are the only way to start a browser session. Only the SHA-256 of a session cookie is stored. CSRF is enforced twice: `SameSite=Strict` plus an `Origin` check on every cookie-authenticated mutation; a missing `Origin` is refused, an explicit `Authorization: Bearer` credential is exempt.
- **Recovery and linking.** `!recover` is time-fenced; cancelling takes `AuthUser`, not `VerifiedUser`. Sessions never follow an account merge.
- **Config.** `Config::validate` refuses to boot production on localhost WebAuthn defaults. Secrets come from the environment, never source code; log none of them.
- **Untrusted input everywhere:** SQL injection, command injection, SSRF (any outbound request built from user input), path traversal, unsafe file handling, HTML injection (`html_escape()`), resource exhaustion (bound sizes, counts, time). Apply least privilege. Use existing security utilities instead of parallel implementations.
- Every security invariant above needs a test proving it stays enforced.

## 8. Verification

Never commit code that fails compilation, formatting or tests. Run from the repository root; setup, test database and troubleshooting are in the README's *Development* section and [`docs/database.md`](docs/database.md).

```bash
cargo fmt --all -- --check                                  # CI blocks; `cargo fmt --all` fixes
cargo check -p matrix-bot --all-targets                     # warning-free
cargo test  -p matrix-bot --all-targets                     # needs TEST_DATABASE_URL (includes --test architecture)
cargo test  -p matrix-bot live_postgres -- --ignored --test-threads=1   # needs PG_TEST_URL
cargo clippy -p matrix-bot --all-targets -- -D warnings     # zero warnings; -p matrix-bot-inference for that crate
cargo build -p matrix-bot --release                         # release-sensitive changes
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # regenerate; commit if changed
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets           # after query/migration changes
cargo test -p matrix-bot-inference --all-targets            # when inference/ or contracts/ change
npm --prefix frontend run build                             # tsc + vite build (CI gate)
npm --prefix frontend run test:e2e                          # Playwright; required before shipping a UI change
```

Single tests: `cargo test -p matrix-bot <name>` or a module path such as `interface::chat::todos::`. After any `#[utoipa::path]`/`ToSchema` change also run `npm --prefix frontend run generate-api`. CI (`.github/workflows/ci.yml`) runs `backend`, `inference` and `frontend` jobs; clippy is a required local gate but not yet a CI job.

**Checklist by change type**

- *Schema:* migration, `OWNED_TABLES`, `.sqlx/`, backend suite; rehearse a rewriting migration on a production copy.
- *API/OpenAPI:* handler and DTOs, `openapi.json`, frontend client, `contracts/` fixtures, auth/step-up re-check.
- *AI/agent:* validate tool arguments, verify authorization independent of the model, test malformed output, tool failure, timeouts, cancellation, retry limits and duplicate calls.
- *Bug fix:* reproduce it, add a regression test, confirm it fails before the fix, fix it, then run the broader suite. Never delete a test because it makes an implementation inconvenient.
- *Before submitting:* `git diff --check`; review `git status` and `git diff` for unrelated changes, generated artifacts, debug code and secrets.

Tests verify behavior, not implementation: Given/When/Then, at the lowest level that proves the point (domain `#[test]`, repository with `test_pool()`, service transaction and event tests, API, Matrix, MCP). Prioritize business invariants, authorization, data isolation, transactions, idempotency, error paths and public contracts over a coverage number.

## 9. Constraints — what must not be done

**Hard safety rules (never override):**

- Never read or modify the main checkout's `.env` (production credentials) or `data/` (the real Matrix crypto store). Work in a separate worktree.
- **Never run the bot binary** (`cargo run`, `matrix-bot`, `docker compose up`). The only exception is `cargo run --bin generate-openapi`. Never contact a homeserver, LLM provider or other external service from tests or scripts; never run `self_verify`.
- `.env` is read from the current directory only (`dotenvy::from_path`); never `dotenvy::dotenv`.
- Point `TEST_DATABASE_URL`, `PG_TEST_URL` and `DATABASE_URL` only at a throwaway local server; never rehearse a migration against production itself.
- Never use bare `git stash` (the stack is shared across worktrees). Never force-push unless the user explicitly asks.
- **Never push, merge, open a PR, tag or deploy** without the user's explicit instruction; each is a separate approval.

**Never:** rewrite large parts of the project without need; mix a feature with an unrelated refactor, formatting sweep or dependency upgrade; remove existing functionality without justification; introduce a framework because it is popular; duplicate business logic across interfaces; bypass authorization or the service layer; modify the schema without a migration; add a compatibility shim for a purely internal refactor; silence a clippy warning with `#[allow]` without documenting why it exists, why it is safe, its scope and when it can be removed; claim tests passed without running them.

## 10. Git, docs and reporting

- **Commits:** Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`; `!` for a breaking change). Keep each commit one logical change.
- **Forge:** `gitea` (`git.mbm.one`) is primary — push and open pull requests there; `origin` (GitHub) is a mirror updated by hand. `gh` and GitHub MCP tools see only the mirror. A PR description covers **all** commits on the branch, a test plan with checkboxes, and the gates the owner still runs. CI runs only for `master` and PRs into it. No token available: push the branch and hand over the `…/pulls/new/<branch>` link; never ask for or embed a token.
- **Documentation** explains architectural decisions, non-obvious constraints, operational procedures and external contracts — not code line by line. When a decision changes, update the owning doc (`docs/architecture.md`, `database.md`, `api.md`, `llm.md`) in the same change. Plans, specs and progress live in `docs/superpowers/`; completed history is deleted rather than accumulated.
- **Final response** summarizes: (1) what changed; (2) important architectural decisions; (3) files/modules changed; (4) tests and checks executed, with results; (5) remaining limitations and follow-up work.

## 11. Gotchas

- Bare `cargo run` resolves to the bot only via `default-members = ["backend"]` + `default-run = "matrix-bot"`; keep both when adding a workspace member or binary. Always scope commands with `-p`.
- `interface/http/mod.rs` probes `web/dist` then `backend/web/dist`; the latter assumes the process starts from the repo root.
- `frontend/vite.config.ts` proxies `/api` and `/health` to `:8080`, matching `API_PORT`'s default; change one, change the others.
- `WEBAUTHN_RP_ID` is a bare host; `WEBAUTHN_ORIGIN` carries the port and must match the browsing origin.
- Windows: a cold `openssl-sys` build needs a full Perl on `PATH` (Strawberry or scoop `perl`; Git for Windows' Perl lacks `Locale::Maketext::Simple`). Loading the real Kokoro model in a test needs sherpa's `onnxruntime.dll` in `target/debug/deps/`.
- Migration and `.sqlx/` files are pinned to LF; a CRLF checkout breaks checksums.
- Cutting production over to this branch follows `docs/superpowers/runbooks/postgres-foundation-cutover.md`.

## 12. Definition of done

A change is not complete merely because it compiles.

- [ ] Requirements implemented; existing architecture and dependency direction preserved.
- [ ] Business logic is in the right layer; no duplicate logic across interfaces.
- [ ] Input validation, authorization and data isolation enforced.
- [ ] Errors handled explicitly; concurrency and cancellation behavior understood.
- [ ] Transactions and invariants correct; database changes have migrations (and `OWNED_TABLES`, `.sqlx/`).
- [ ] Public contracts (REST, OpenAPI, Matrix, MCP, schema, config) remain intentional and are updated where required.
- [ ] Tests added or updated; no obsolete implementation remains.
- [ ] Logs carry diagnostic context without sensitive data.
- [ ] `cargo fmt`, `cargo check`, `cargo test` pass; `cargo clippy` clean on touched code; frontend build/tests when frontend changed.
- [ ] No unrelated changes; `git diff --check` clean.
- [ ] Docs and `docs/superpowers/HANDOFF.md` updated where necessary.
