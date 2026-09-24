# AGENTS.md

## Overview

`matrix-bot` is a Matrix chat bot with an embedded web UI: a Rust Cargo workspace holds the bot itself (`backend/`) plus a local-inference server for OCR, speech-to-text and text-to-speech (`inference/`), and a React SPA (`frontend/`) builds into the bot's static assets. Data lives in SQLite by default, with PostgreSQL as a supported second target picked at boot from `DATABASE_URL`. This file states every hard rule and convention; `docs/ARCHITECTURE.md` is the long-form companion explaining the *why* behind the parts of the design that aren't obvious from the code alone.

## Commands

```bash
cargo check                              # must be warning-free
cargo fmt --all -- --check               # CI blocks on this; `cargo fmt --all` to fix
cargo test -p matrix-bot --all-targets   # backend suite (CI's own invocation)
cargo test -p matrix-bot-inference --all-targets   # inference crate; not covered by bare `cargo test`
cargo test --package matrix-bot self_verify -- --ignored   # live check: is the running bot verified?
cargo test test_parse_message_exclamation_command   # single test by name
cargo test --package matrix-bot modules::todo::     # one module's tests
cargo run                                # starts bot + API server

# Migrate a COPY of a real database and assert nothing was stranded — run
# before shipping any migration that touches ownership.
cp data/bot_data.db /tmp/copy.db
MIGRATE_TEST_DB=/tmp/copy.db cargo test migrates_a_real_database -- --ignored

# Run the suite against PostgreSQL instead of in-memory SQLite (what
# backend-postgres does in CI)
TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/postgres cargo test -p matrix-bot

# After any #[utoipa::path] or ToSchema change, regenerate both and verify no drift
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
DATABASE_URL=postgres://bot:bot@postgres:5432/matrix_bot ENVIRONMENT=development WEBAUTHN_ORIGIN=http://localhost:8080 docker compose --profile postgres up -d
```

## Repository Layout

- `backend/` — Rust crate `matrix-bot`: Matrix bot + embedded Axum REST API.
- `inference/` — Rust crate `matrix-bot-inference`: Axum server hosting local ONNX models (OCR, STT, Kokoro TTS).
- `contracts/` — JSON fixtures each crate asserts its wire formats against.
- `frontend/` — React 19 + Vite + Tailwind v4 SPA, built into `backend/web/dist/`.
- `data/` — runtime SQLite DB + Matrix crypto store (gitignored).
- `docs/` — `ARCHITECTURE.md` (design rationale), `testing-auth-locally.md` (passkeys against `localhost`), roadmap, reviews.

## Working Principles

- Do not preserve backward compatibility. Remove obsolete paths instead of adding compatibility layers, fallbacks, or migrations.
- Choose the simplest implementation that fully meets the current requirements. Avoid speculative abstractions, configuration, and indirection.
- Grow the system in layers. Start from the smallest version that works end to end, and add each new capability on top of a product that already works. Never trade a working product for unfinished complexity.
- Keep components modular and concerns clearly separated.
- Prefer established, well-maintained libraries when they reduce overall complexity or improve reliability. Do not reimplement common functionality without a clear reason.
- Lean on the dependencies already in the project before writing your own implementation or adding packages. Do not assume a library lacks a capability without checking its documentation and types.
- Make architectural decisions for the long term. Do not accept a stopgap that only works for now and is meant to be replaced later.

## Conventions and Rules

**`BotModule` plugin system** — features are independent modules implementing `BotModule` (`backend/src/modules/mod.rs`), registered in `backend/src/bot/mod.rs`. Never add command handling to `main.rs` or `bot/handler.rs`.

**Embedded Axum REST API** — endpoints live in `backend/src/api/`, aggregated via `api_routes()` in `api/routes.rs`. Use `ApiError` (`backend/src/error.rs`) for every response (`{ "error": "...", "code": 400 }`). Every mutation must `broadcaster.send(SystemEvent::…)`, and **every `SystemEvent` variant must return a meaningful `owner_id()`** — the broadcast channel is process-wide and per-owner SSE delivery depends on it; a variant with no owner leaks across users or is never delivered. Cap API payload strings at 500 chars.

**File organization** — no file in `backend/src/` or `inference/src/` exceeds 500 lines of production code (target < 300; `#[cfg(test)]` modules don't count). Split along a real seam, never by line count alone. Feature module shape: `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`, optional `scheduler.rs`.

**`services/` boundary** — `services/` never imports from `modules/`. `modules/` is what the app does; `services/` is how it reaches external capabilities. A wire type and a domain type that share fields stay separate, with a `From` impl in the module.

**`contracts/`** — every HTTP wire contract between the two crates lives here as a JSON fixture both sides assert against (or the server side alone, for a server-only format) — a renamed field fails a test, not production.

**Database & SQLx**:
- **Never call `sqlx::query*` directly.** Use the dual-dialect macros in `backend/src/db/executor.rs`, which holds `DbPool` (an enum over both drivers) — `db_fetch_all!`, `db_fetch_one!`, `db_fetch_optional!`, `db_scalar_*!`, `db_execute!`, `db_fetch_all_dyn!` for dynamic filters, and the `tx_*` family inside a transaction. Write SQL once in SQLite's `?` style; `dialect::render_placeholders` rewrites it to `$1..$n` for Postgres. Compile-time `query!` macros stay banned — they need a DB URL at build time.
- **Never put a value into SQL text; always bind it**, even when assembling SQL dynamically — `db_fetch_all_dyn!` (e.g. `modules/bookmarks/repository.rs`) builds the clause with `push_str` but still binds every value as a parameter.
- Keep SQL portable — every row below was a real defect SQLite accepted and PostgreSQL rejected; `backend-postgres` in CI is blocking.

  | Do not write | Write instead | Why |
  |---|---|---|
  | `last_insert_rowid()` | `RETURNING id` with `db_scalar_one!` | SQLite-only |
  | `randomblob(n)` | `utils::random` | SQLite-only, and a token's entropy should not depend on the database |
  | `datetime('now')`, `strftime(...)` | `CAST(CURRENT_TIMESTAMP AS TEXT)`, or compute it in Rust | SQLite-only |
  | `INSERT OR IGNORE` | `INSERT ... ON CONFLICT DO NOTHING` | SQLite-only |
  | `UPDATE OR IGNORE` | `accounts::merge::move_rows` | PostgreSQL has no equivalent |
  | `x IS NOT ?` | `x IS DISTINCT FROM ?` | `IS NOT` expects `NULL`/`TRUE` on PostgreSQL |
  | `COLLATE NOCASE` | `LOWER(...)` | SQLite-only collation |
  | `PRAGMA`, `sqlite_master` | `information_schema`, or gate with `pool::skip_on_postgres` | SQLite-only |
  | `SUM(bigint_col)` | `CAST(SUM(...) AS BIGINT)` | PostgreSQL returns `NUMERIC` |
  | `SELECT 1` read as `i64` | `SELECT CAST(1 AS BIGINT)` | an integer literal is `INT4` |
  | `date(x)` compared or unioned with text | `CAST(date(x) AS TEXT)` | PostgreSQL returns a `date` |
  | `substr(timestamp_col, ...)` | `substr(CAST(col AS TEXT), ...)` | `substr` needs text |
  | `ON CONFLICT DO UPDATE SET c = c + excluded.c` | qualify: `tbl.c + excluded.c` | ambiguous on PostgreSQL |
  | `0`/`1` for a boolean | `TRUE`/`FALSE` | PostgreSQL has a real boolean |

- Match the column type to the Rust type it is read as (`i64`→`BIGINT`, `i32`→`INTEGER`, `f64`→`DOUBLE PRECISION`, `bool`→`BOOLEAN`, `DateTime<Utc>`→`TIMESTAMPTZ`). **Change column types one at a time, against the field that reads them** — three rounds of defects came from blanket `INTEGER`→`BIGINT` / `TIMESTAMP`→`TIMESTAMPTZ` sweeps.
- `TIMESTAMPTZ` holds microseconds, `DateTime<Utc>` holds nanoseconds — build test values with `utils::time::now_at_storage_precision`, not `Utc::now()`.
- Migrate via `CREATE TABLE IF NOT EXISTS` + best-effort `ALTER TABLE` in `db/schema_*.rs`, orchestrated by `db/schema.rs`. No `sqlx migrate` directory.
- Room caching/resolution lives only in `db/rooms.rs` — never couple domain repositories or bot modules to API modules.
- SQLite pools: `WAL`, `busy_timeout=5000`, `DATABASE_MAX_CONNECTIONS`. Postgres pools take only the connection limit.

**Matrix response & formatting** — always reply with `text_html(&plain, &html)`. Escape user input with `html_escape()`; strip HTML for the plain-text fallback with `strip_html()` (`backend/src/utils/html.rs`). Quoted-message context (`CommandContext.quoted`, parsed in `bot/quote.rs`) — AI injects it into prompts; Todo/Reminder use it as the body when none is supplied.

**AI tools & web search** — define tools with the standard OpenAI `type: "function"` schema (`modules/ai/search.rs`); never a provider-specific tool variant. Each domain tool file exports its own `tool_definitions()`; `definitions.rs` just aggregates them. Gate web search by intent (`should_enable_web_search()`) — skip it for routine productivity commands. Search backend: Tavily (`TAVILY_API_KEY`) first, falling back to keyless DuckDuckGo scraping. Execute concurrent tool calls with `futures_util::future::join_all`, not a sequential loop. Long research requests need a generous router timeout (`TimeoutLayer` >= 180s); dedupe immediate retries by checking the conversation's tail message before inserting.

**AI transport configuration** — settings read exactly one env name each: `AI_ENDPOINT` (base URL incl. `/v1`), `AI_API_KEY`, `AI_MODEL`. Per-user base URLs are an SSRF surface: `AI_ALLOW_PRIVATE_BASE_URL` (off by default) is the only way to allow one that resolves to a private host. The AI transport's own proxy chain (`config.rs`) is `AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy` → `HTTP_PROXY`/`http_proxy`. The web-search HTTP client has a **separate, shorter** chain (`modules/ai/search.rs`) — `AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy`, with no `HTTP_PROXY` fallback — don't assume the two share one list. Web-search key: `TAVILY_API_KEY`.

**Frontend design system** (`Design.md`) — monochrome-first, Geist Sans for prose/controls, Geist Mono for identifiers (code, paths, timestamps, currency codes). Sentence-case headings, no all-caps overlines. No redundant outer padding or centering containers on module root views — `AppShell` already provides the gutters. No decorative gradients, glow, glassmorphism, pulsing badges, or simulated animation; color only for meaningful state, always paired with a non-color cue. Stat Strip pattern for summary metrics; tables right-align numbers, left-align text. Every control keyboard-accessible (`Escape` closes modals, `Enter` submits single-line forms).

## Module Implementation Checklist

When creating or enhancing a feature module:

1. **Backend directory**: `backend/src/modules/<feature_name>/` with `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`.
2. **Database migration**: table schema in `backend/src/db/schema.rs` (or a modular `schema_<feature>.rs`).
3. **Repository**: SQLx query binding CRUD methods in `repository.rs`.
4. **Command handler**: parse Matrix commands, enforce length limits (max 500 chars), format HTML output.
5. **REST API**: handlers in `backend/src/api/<feature_name>.rs` returning `Result<_, ApiError>`, registered in `backend/src/api/mod.rs`.
6. **SystemEvents**: variants in `backend/src/events.rs` with `owner_id()`, broadcast on every state change.
7. **BotModule registration**: implement `BotModule` in `mod.rs`, register in `backend/src/bot/mod.rs`.
8. **OpenAPI & types**: update `ApiDoc` in `backend/src/api/openapi.rs`, then run the OpenAPI regeneration commands above.
9. **Frontend**: `frontend/src/modules/<feature_name>/` exporting `<feature>Module: FrontendModule`; register in `frontend/src/App.tsx`; nav item in `Sidebar.tsx`; label in `AppShell.tsx`.
10. **Tests**: backend unit tests in `repository.rs`/`handler.rs`/`api/<feature_name>.rs` using `sqlite::memory:`; mock routes in `frontend/e2e/fake-api.ts`; Playwright spec in `frontend/e2e/<feature_name>.spec.ts`.

## Architecture

One- to three-sentence summaries; full rationale is in `docs/ARCHITECTURE.md`.

- **Single process, two front doors** — `main.rs` boots one `Config`/`Database`/`EventBroadcaster` and spawns the Axum API server and the Matrix sync loop, coupled through shared handles (bot token, Matrix client, pending merges). [Details](docs/ARCHITECTURE.md#single-process-two-front-doors).
- **Backend module plugin system** — features are `BotModule`s dispatched in registration order by `ModuleRegistry`; background work runs through `spawn_background_workers`. [Details](docs/ARCHITECTURE.md#backend-module-plugin-system).
- **Event flow (SSE)** — every mutation broadcasts a `SystemEvent`, delivered per-owner via `owner_id()`. [Details](docs/ARCHITECTURE.md#event-flow-sse).
- **Matrix session and crypto store** — never keep the crypto store across a fresh login, and never pin a device ID on one, or peers see broken encryption or a silent key rotation. [Details](docs/ARCHITECTURE.md#matrix-session-and-crypto-store).
- **Verification** — `ensure_self_verified` must run on every boot; never treat `Recovery::recover()`'s `Ok` as "verified". [Details](docs/ARCHITECTURE.md#verification-self-and-peer).
- **Accounts and ownership** — `owner_id` on every table is an `accounts.id`, resolved from a Matrix link or a web session, never derived any other way. [Details](docs/ARCHITECTURE.md#accounts-one-owner-several-ways-in).
- **Auth: passkeys and sessions** — passkeys are the only way to start a browser session; sensitive routes must require `VerifiedUser`, not just `AuthUser`, or `step_up_tests` fails CI. [Details](docs/ARCHITECTURE.md#auth-passkeys-and-cookie-sessions).
- **Linking and recovery** — `!link` must be handled before the sender is resolved, or the claim provisions an empty account first; `!recover` is time-fenced to avoid a passkey/Matrix trust circle. [Details](docs/ARCHITECTURE.md#linking-recovery-and-the-matrix-ceiling).
- **Per-user settings** — timezone/currency, read by every other module; a missing row means defaults, never an error. [Details](docs/ARCHITECTURE.md#per-user-settings-timezone-default-currency).
- **Voice replies** — `!say` synthesises through `services/tts`; the reply is Ogg Opus (not WAV) so clients draw a voice bubble. [Details](docs/ARCHITECTURE.md#voice-replies-say).
- **Two crates, services and inference** — `backend/` and `inference/` are separate crates joined by `services/` and `contracts/`; `default-members = ["backend"]` keeps `cargo run`/bare `cargo test` scoped to the bot. [Details](docs/ARCHITECTURE.md#two-crates-services-and-inference).
- **Portfolio** — cash is a hard precondition for buying, never for selling; the holding write and the cash write happen in one transaction. [Details](docs/ARCHITECTURE.md#portfolio-investment-holdings).
- **Persistence and migrations** — `db/schema.rs::run_migrations` is the only mechanism: idempotent `CREATE TABLE IF NOT EXISTS` plus best-effort `ALTER TABLE`, no `sqlx migrate` directory. [Details](docs/ARCHITECTURE.md#persistence-and-migrations).
- **Two databases** — `DATABASE_URL` picks SQLite (default) or PostgreSQL at boot; `sqlx::Any` can't do this job (no chrono in 0.7), which is why the macro layer exists. [Details](docs/ARCHITECTURE.md#two-databases-one-set-of-queries).
- **Frontend module mirror** — the SPA mirrors the backend's module shape; `core/session.ts` is the single source of "who's signed in" since the cookie is unreadable by the page. [Details](docs/ARCHITECTURE.md#frontend-module-mirror).

## Testing and Verification

```bash
cargo fmt --all -- --check                        # CI: backend + inference jobs both block on this
cargo test -p matrix-bot --all-targets             # CI: backend job
cargo test -p matrix-bot-inference --all-targets   # CI: inference job
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # CI: OpenAPI drift check
npm --prefix frontend run build                    # CI: frontend job
npm --prefix frontend run test:e2e                 # not CI-gated yet; still required before shipping a UI change
```

CI (`.github/workflows/ci.yml`) runs four jobs: `backend` (format, tests, OpenAPI drift), `backend-postgres` (the same suite plus `live_postgres --ignored` tests against a real PostgreSQL — blocking), `inference` (format + tests), `frontend` (build only). See "Commands" above to run the Postgres suite locally.

Never commit code that fails compilation, formatting, or tests.

## Security Rules

- Sensitive API routes require `VerifiedUser`, not just `AuthUser` (see Architecture: Auth).
- Never put a value into SQL text — always bind it, even in dynamically assembled queries (see Conventions: Database & SQLx).
- Escape all user input embedded in HTML with `html_escape()`.
- Every `SystemEvent` variant must return a meaningful `owner_id()`, or it leaks across users or is never delivered.
- Matrix has no route to a privileged action (`WEB_ONLY` in `modules/account/mod.rs`): `!export`, `!unlink`, `!passkey`, `!sessions`, `!recoverycodes` all point to the web app.
- `!link` is refused outside a DM, and **the code is burned either way** — a public-room code is readable by anyone.
- Only the SHA-256 of a session cookie is stored server-side.
- CSRF is enforced twice: `SameSite=Strict` plus an `Origin` check on every cookie-authenticated mutation.
- `Config::validate` refuses to boot production on localhost WebAuthn defaults.
- API payload strings are capped at 500 chars.

## Gotchas

- `DATABASE_URL` defaults to `sqlite://data/bot_data.db?mode=rwc`; `api/mod.rs` probes `web/dist` then `backend/web/dist`. Both assume `cargo run` from the repo root.
- Bare `cargo run` resolves to the bot only via `default-members = ["backend"]` + `default-run = "matrix-bot"` in `backend/Cargo.toml` — keep both if you add a workspace member or a third binary.
- `frontend/vite.config.ts` proxies `/api` to `:8080`, matching `API_PORT`'s default. Change one, change the other.
- Windows: a cold `openssl-sys` build needs a full Perl on `PATH` first (e.g. Strawberry or scoop `perl`) — Git for Windows' bundled Perl lacks `Locale::Maketext::Simple`.
- Windows: loading the real Kokoro TTS model in a test needs sherpa's own `onnxruntime.dll` copied into `target/debug/deps/` — see that test's own comment in `inference/src/kokoro.rs` for why a version mismatch there dies with an access violation rather than a clean error.

## Commits and PRs

Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`; `!` for a breaking change, e.g. `refactor!:`).

Two remotes: `gitea` (`git.mbm.one`) is the primary forge — push and open pull requests there; `origin` (GitHub) is a mirror.
