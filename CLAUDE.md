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
cargo test                       # 151 tests, all in-crate #[cfg(test)] modules
cargo test --package matrix-bot self_verify -- --ignored   # live check: is the running bot's session verified?
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

`api/rooms.rs::list_rooms` is a *room picker* feed, not a room dump: it drops spaces, DMs (`room.is_direct()`), rooms not in `RoomState::Joined`, rooms lacking a name or canonical alias, and any room where either the bot or the caller is not a joined member. `bot/client.rs` treats `data/matrix_session.json` and `data/bot_store/` as **one unit**: the session is reused only when `restore_session` plus a live `whoami` both succeed, and any other path (no session file, rejected token, device deleted in a client) archives the store to `data/bot_store.orphaned-<UTC timestamp>/` before logging in fresh. Never keep the crypto store across a fresh login — its Olm account belongs to the dead device, the crypto machine believes it already published those keys, and every message the bot then encrypts shows in Element as "Encrypted by an unknown or deleted device". The store is archived rather than deleted because it may hold the only copy of room keys that never reached key backup; `MATRIX_RECOVERY_KEY` is what repopulates a fresh store (`restore_room_keys_from_backup`).

A fresh login does **not** pin a device ID. It used to (`MATRIX_BOT_MAIN_DEVICE`, constant across every restart), which silently rotated that device's keys under the same ID on every restore failure — routine, not rare, since "no session file / rejected token / device deleted in a client" are the normal triggers. The homeserver accepts that re-upload without complaint, but every peer who already talked to the old device keeps encrypting *to* it using its old keys: the peer is the sender in that direction, so it never hits a decrypt error and has no trigger to rebuild its Olm session. The result is exactly what CLAUDE.md already described as the failure mode this section guards against — except it was also happening on ordinary token-rejection restarts, not just on manual device deletion. Now `client.rs::delete_stale_device` captures the outgoing device's ID before a fresh login (session file must be read *before* `restore_previous_session` deletes it on failure), logs in with **no** pinned ID so the homeserver assigns a fresh one, then deletes the old device via password-authenticated UIA once the new session is live. Every fresh login now presents as an unambiguous new device to every peer — the well-tested "new device, verify me" path — instead of an in-place key swap peers have no reliable way to notice.

`bot/self_verify.rs::ensure_self_verified` makes the bot's own session show as *Verified* in Element without a human: it signs the bot's device with the account's own self-signing key (imported from `MATRIX_RECOVERY_KEY` via secret storage) and retries because the SDK's own attempt inside `Recovery::recover()` races the first sync — it silently finds nothing to sign if the device isn't in the crypto store yet, and `recover()` returns `Ok` either way, so treat that `Ok` as "secrets imported," never as "verified." `bot/verification.rs::request_verification_from_allowlist` handles the *other* trust edge — an allowlisted human verifying the bot — by sending the request from the bot's side on every boot (a client that refuses unverified sessions would otherwise block the very `!verify` message that starts the flow) unless `data/verified_users.json` already records a completed SAS with that user; that file is the only signal treated as durable, because `UserIdentity::is_verified()` reports only *our* signature over *them* and was observed true while their client was still refusing to send to the bot.

`bot/self_verify.rs` is what keeps the session from showing as *Unverified* in Element. Element's rule is cross-signing: the account's self-signing key must have signed the device, with the signature on the homeserver. A password login mints unsigned device keys, so signing has to follow it — that second step, not the password, is why an Element login looks verified and a bare bot login does not. `MATRIX_RECOVERY_KEY` unlocks secret storage for two independent things: the cross-signing keys (which make the session verified) and the megolm backup key (which makes history decryptable). An account whose secret storage carries only the latter recovers its room keys and stays unverified forever.

`ensure_self_verified` runs on **every** boot, after `recover()` but regardless of whether a recovery key was configured or worked — a restored crypto store already holds the self-signing key, so a restart re-verifies with no recovery key involved. Do not fold it back into the `recover()` success branch. It retries because a fresh login races itself: the SDK attempts the same signing inside `Recovery::recover`, but only when our device is already in the crypto store, and our device keys are still uploading on the first sync — so that attempt silently finds nothing to sign while `recover()` still returns `Ok`. Never treat `recover()` returning `Ok` as a verified session.

Two measured facts about matrix-sdk 0.7 drive the shape of that code. `Recovery::recover` returns `Ok` even when secret storage held no self-signing key and nothing was signed (`SecretStore::import_secrets` gates signing on `has_self_signing`, and a missing secret reads as `None`, not an error). And `Device::is_cross_signed_by_owner()` on our *own* device keeps reporting false for at least a second after the signature is uploaded and re-queried, across restarts, even though `/keys/query` shows it — so the outcome is taken from `Device::verify()` returning `Ok` (the signature reached the server), never from a local re-read. Consequently the bot re-signs on most boots; that is intended and idempotent.

The acceptance test is `self_verify.rs::test_the_running_bot_session_is_verified_on_the_homeserver` — `#[ignore]`d because it needs credentials, a network, and a bot that has run at least once. It reads the token the running bot saved and asks `/keys/query`, applying Element's own rule via `device_is_cross_signed`; it never logs in, because a second login on the pinned device ID would revoke the running session's token.

### Backend module plugin system

`modules::BotModule` (in `backend/src/modules/mod.rs`) is the extension point. `ModuleRegistry::dispatch` walks modules in registration order and returns the first `Some(html_reply)`. Registration happens in `bot/mod.rs::new_full` — Core, Todo, Reminder, Finance, Settings. `CoreModule` receives an `Arc<RwLock<Option<Arc<ModuleRegistry>>>>` back-reference so `!help` can enumerate all modules (a deliberate cycle-breaking indirection; keep it if you touch help generation).

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

### Per-user settings (timezone, default currency)

`modules/settings/` is an ordinary `BotModule` (`!settings`, `!settings timezone <IANA zone>`, `!settings currency <CODE>`; `tz`/`cur` are accepted aliases) whose data every *other* module reads — todo and reminder format timestamps against the sender's zone, finance defaults an amount's currency to it — and which `api/settings.rs` exposes to the web UI. Rows in `user_settings` are keyed by `owner_id`, the same key as `sender_mxid`, and a user has no row until they change something: `SettingsRepository::get` returns `UserSettings::default()` (UTC/USD) for a missing row, which is the normal case, not an error. The command path uses `get_or_default`, which additionally swallows a database error into the defaults — a settings lookup must never be the reason a `!todo list` reply is lost.

Because it mutates state from the Matrix side, `BotModule::handle_command` takes an `Option<&EventBroadcaster>` (threaded from `bot/mod.rs` → `register_handlers` → `ModuleRegistry::dispatch`). It is `None` only in tests. The settings handler broadcasts `SettingsUpdated` on a successful save and on nothing else — a read or a rejected value announces no change. **The other modules do not yet broadcast from the Matrix path** (`due_reminder.rs` documents that gap); the parameter is there so they can, and any new mutating command should use it.

`utils::format_datetime` takes a `chrono_tz::Tz` rather than assuming UTC, and `todo`/`reminder` handlers resolve the sender's zone once, *after* the command is claimed, so an unrelated room message costs no query. The finance handler's `DEFAULT_CURRENCY` constant is gone: `parse_entry` takes the sender's saved currency, so `!expense add 9.99 coffee` books in what they actually spend. An explicit code in the command still wins.

Validation lives at the API boundary (`api/settings.rs`), not at render time: an unknown zone stored now would render as UTC on every later read with nothing to point at. `parse_timezone` accepts exactly the IANA names `Intl.supportedValuesOf('timeZone')` offers the picker; currency goes through the finance module's `normalise_currency`.

On the frontend, `core/settings.ts` is the single source — `useSettings()` reads one shared React Query entry, so a change reformats the whole app from one cache write. Anything rendering an instant must pass the zone explicitly (`formatDateTimeInZone` and friends); a bare `toLocaleString` silently uses the browser's zone, which is exactly what the setting exists to override. The same applies in reverse for *input*: `zonedInputToUtcIso`/`utcIsoToZonedInput` convert the `DateTimePicker`'s naive `YYYY-MM-DDTHH:mm` against the chosen zone, applying the offset twice so a daylight-saving boundary cannot land the result an hour out. `DateTimePicker` itself stays zone-agnostic and takes a `now` prop purely so "today" and the "Now" button mean the right day.

### Persistence

`db/schema.rs::run_migrations` is the only migration mechanism: idempotent `CREATE TABLE IF NOT EXISTS`, best-effort `ALTER TABLE` (errors deliberately swallowed with `let _ =`), and data-fixup `UPDATE`s. There is no `sqlx migrate` directory. Use runtime-bound queries (`sqlx::query_as::<_, Model>(...).bind(...)`), never the `query!` macros — the project avoids compile-time DB connections.

### Frontend module mirror

The SPA mirrors the backend plugin shape: `core/registry.ts` holds `FrontendModule`s registered in `App.tsx` (`core`, `todo`, `reminder`). Each module supplies a `MainView`, optional `navTab`/`getStatItems`, and a `handleSseEvent(event, queryClient)` used to invalidate React Query caches from the live stream (`core/useSystemEvents.ts`). Adding a backend module means adding a matching `frontend/src/modules/<name>/index.tsx` and registering it in `App.tsx`.

All API calls go through `core/apiClient.ts` (`fetchApi`), which attaches the token from `localStorage['matrix_bot_jwt']` and unwraps the standardized `{ "error": "...", "code": 400 }` shape produced by `backend/src/error.rs::ApiError`.
