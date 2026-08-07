# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Design & Implementation Principles

- Do not preserve backward compatibility. Remove obsolete paths instead of adding compatibility layers, fallbacks, or migrations.

- Choose the simplest implementation that fully meets the current requirements. Avoid speculative abstractions, configuration, and indirection.

- Grow the system in layers. Start from the smallest version that works end to end, and add each new capability on top of a product that already works. Never trade a working product for unfinished complexity.

- Keep components modular and concerns clearly separated.

- Prefer established, well-maintained libraries when they reduce overall complexity or improve reliability. Do not reimplement common functionality without a clear reason.

- Lean on the dependencies already in the project before writing your own implementation or adding packages. Do not assume a library lacks a capability without checking its documentation and types.

- Make architectural decisions for the long term. Do not accept a stopgap that only works for now and is meant to be replaced later.

## Repository Layout

Cargo workspace at the root with a single member, plus a separate npm frontend:

- `backend/` — Rust crate `matrix-bot`: Matrix bot + embedded Axum REST API in one binary.
- `frontend/` — React 19 + Vite + Tailwind v4 SPA, built into `backend/web/dist/`.
- `data/` — runtime SQLite DB + Matrix SDK crypto store (gitignored).

Note: `README.md`'s architecture tree shows `src/` at the repo root and `api/auth.rs` as a single file. Both are stale — the crate lives at `backend/src/` and `api/auth/` is a directory (`extractor.rs`, `handlers.rs`, `matrix.rs`, `types.rs`).

`AGENTS.md` holds the authoritative development conventions (module checklist, file-size limits, SQLx and HTML-formatting rules). Read it before adding a feature module.

## Commands

```bash
# Backend (run from repo root — paths below are cwd-relative)
cargo check                      # must be warning-free
cargo test                       # 60 tests, all in-crate #[cfg(test)] modules
cargo test test_parse_message_exclamation_command   # single test by name
cargo test --package matrix-bot modules::todo::     # one module's tests
cargo run                        # starts bot + API server

# Frontend
cd frontend
npm run dev                      # Vite dev server on :5173
npm run build                    # tsc + vite build -> backend/web/dist
npm run generate-api             # regenerate src/types/api.ts from ../backend/openapi.json
```

After changing any `#[utoipa::path]` annotation or `ToSchema` struct, regenerate both artifacts:

```bash
cargo run --bin generate-openapi          # -> backend/openapi.json
npm --prefix frontend run generate-api    # -> frontend/src/types/api.ts
```

The crate is a lib (`matrix_bot`) plus two bins, so `generate-openapi` can reuse the API types. Tests never write `openapi.json`.

### Working-directory gotchas

- `DATABASE_URL` defaults to the relative `sqlite://data/bot_data.db?mode=rwc`, and `api/mod.rs` probes for `web/dist` then `backend/web/dist`. Both assume `cargo run` from the **repo root**.
- Bare `cargo run` from the root resolves to the bot only because of workspace `default-members = ["backend"]` plus `default-run = "matrix-bot"` in `backend/Cargo.toml` (the crate ships two bins). Keep both if you add a workspace member or a third binary.
- `frontend/vite.config.ts` proxies `/api` to `http://localhost:8080`, matching the `API_PORT` default and `.env`. If you change one, change the other.

## Architecture

### Single process, two front doors

`backend/src/main.rs` boots one `Config` + one shared `Database` + one `EventBroadcaster`, then spawns:

1. The Axum API server (`api::start_api_server`), which returns two shared handles — `Arc<RwLock<Option<String>>>` (bot access token) and `Arc<RwLock<Option<Client>>>` (Matrix SDK client).
2. The Matrix sync loop in a `tokio::spawn` retry loop (reconnects every 5s on error), receiving those same handles via `MatrixBot::new_full`.

Those handles are the coupling point: the REST layer borrows the *bot's* Matrix credentials to serve endpoints like `/rooms` and `/auth/media` that require homeserver access. Shutdown is a `broadcast::channel::<()>` fanned out to both halves on Ctrl-C.

`api/rooms.rs::list_rooms` is a *room picker* feed, not a room dump: it drops spaces, DMs (`room.is_direct()`), rooms not in `RoomState::Joined`, rooms lacking a name or canonical alias, and any room where either the bot or the caller is not a joined member. `bot/client.rs` pins `device_id("MATRIX_BOT_MAIN_DEVICE")` on login so a restart reuses the existing crypto store instead of minting a device per boot; the fallback path drops the client, sleeps, wipes the store, and logs in fresh only when that fails.

### Backend module plugin system

`modules::BotModule` (in `backend/src/modules/mod.rs`) is the extension point. `ModuleRegistry::dispatch` walks modules in registration order and returns the first `Some(html_reply)`. Registration happens in `bot/mod.rs::new_full` — Core, Todo, Reminder. `CoreModule` receives an `Arc<RwLock<Option<Arc<ModuleRegistry>>>>` back-reference so `!help` can enumerate all modules (a deliberate cycle-breaking indirection; keep it if you touch help generation).

Background work goes through `spawn_background_workers(&ModuleContext)` — that's how the reminder scheduler (5s tick loop) gets the Matrix `Client`, `Database`, and broadcaster.

`bot/dispatcher.rs` accepts a message either when it starts with `!` or when the bot is mentioned, strips the HTML mention pill, and yields a `CommandContext`.

### Event flow (SSE)

Every state mutation — from both the Matrix command path and the REST path — must `broadcaster.send(SystemEvent::…)`. `SystemEvent` is serialized `#[serde(tag = "type", content = "payload")]` and sent as an **unnamed** SSE message, so the frontend reads `EventSource.onmessage` and dispatches on the `type` field (the variant name verbatim, e.g. `TodoCreated` — not snake_case).

The broadcast channel is process-wide. `sse::owned_events` narrows it to one owner via `SystemEvent::owner_id()`, so a client sees only its own rows. **Any new variant must return a meaningful `owner_id()`** — the deletion variants carry an explicit `owner_id` field because their row is already gone. Adding a mutation without a matching event silently breaks the live UI; adding a variant without an owner leaks it or drops it.

### Auth: two identity spaces, one extractor

`api/auth/extractor.rs` implements `FromRequestParts for AuthUser`, reading the token from the `Authorization: Bearer` header **only**. It tries, in order: Matrix access token (`syt_` prefix → homeserver `/whoami`), local JWT, then `/whoami` again as a fallback (gated by `looks_like_access_token` so a malformed bearer doesn't cost a network round trip).

Tokens carry a `TokenScope`. The extractor accepts only `Api`-scoped tokens. Because `EventSource` cannot set headers, SSE clients `POST /api/v1/events/ticket` to exchange their API token for a 30-second `SseStream`-scoped ticket, passed as `?ticket=`; `sse::stream_subscriber` accepts only that scope from the query string. Never reintroduce `?token=` — that put a 7-day full-access credential into URLs.

Matrix calls go through `ApiState.matrix_identity` (`MatrixIdentityProvider`). Tests swap in a fake via `ApiState::with_matrix_identity`, which is how the extractor and `login`'s Matrix branch are covered without a homeserver.

`AuthUser` is `{ subject, kind }` where `kind: AuthKind` is `Local` or `Matrix`, carried as a **signed JWT claim** — never inferred from whether `subject` starts with `@`. `AuthKind` defaults to `Local` on deserialization so pre-`kind` tokens fail closed.

`AuthUser::owner_id()` is the single ownership key, matching the `sender_mxid` column and nothing else:
- `Matrix` → the MXID verbatim
- `Local` → `@web-<username>:api`

`sender_name` is display-only and must never appear in an ownership predicate — it is user-controlled and collides across the two identity spaces. `is_valid_local_username` bans `@` and `:` at registration so a local account can never be shaped like an MXID. API-created rows default to the sentinel `room_id = "web"`.

`ApiState.bot_access_token` / `bot_client` hold the **bot's** credentials: written by the bot bootstrap, read-only to request handlers. A handler must never store a caller's token there.

Matrix homeserver URLs are resolved via `.well-known` discovery and memoized in `ApiState.resolved_homeserver` (`api/auth/matrix.rs::resolved_homeserver`), so a configured `MATRIX_HOMESERVER` of `https://mbm.one` can redirect to the real server without re-resolving per request.

### Persistence

`db/schema.rs::run_migrations` is the only migration mechanism: idempotent `CREATE TABLE IF NOT EXISTS`, best-effort `ALTER TABLE` (errors deliberately swallowed with `let _ =`), and data-fixup `UPDATE`s. There is no `sqlx migrate` directory. Use runtime-bound queries (`sqlx::query_as::<_, Model>(...).bind(...)`), never the `query!` macros — the project avoids compile-time DB connections.

### Frontend module mirror

The SPA mirrors the backend plugin shape: `core/registry.ts` holds `FrontendModule`s registered in `App.tsx` (`core`, `todo`, `reminder`). Each module supplies a `MainView`, optional `navTab`/`getStatItems`, and a `handleSseEvent(event, queryClient)` used to invalidate React Query caches from the live stream (`core/useSystemEvents.ts`). Adding a backend module means adding a matching `frontend/src/modules/<name>/index.tsx` and registering it in `App.tsx`.

All API calls go through `core/apiClient.ts` (`fetchApi`), which attaches the token from `localStorage['matrix_bot_jwt']` and unwraps the standardized `{ "error": "...", "code": 400 }` shape produced by `backend/src/error.rs::ApiError`.
