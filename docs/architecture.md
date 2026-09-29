# Architecture

Why the system is shaped the way it is. `AGENTS.md` states the rules an agent must follow; this document records the design behind them, one section per area, each covered once. Topic documents:

| Topic | Document |
|---|---|
| Schema, migrations, SQLx conventions, test databases | [`database.md`](database.md) |
| REST/SSE contract, authentication, webhooks, health probes | [`api.md`](api.md) |
| LLM transport, tool loop, agent limits, MCP server | [`llm.md`](llm.md) |
| Plans, designs, progress and hand-off state | [`superpowers/`](superpowers/) |

## One capability, many interfaces

The backend is a modular monolith and the source of truth for application state. Every business capability (a todo, an expense, a portfolio trade, a room lookup) is implemented once, in `application/` over `domain/`, and reached through any number of interfaces:

```text
                     ┌── Matrix chat (BotModule commands)
                     ├── REST API + SSE (Axum, OpenAPI)
 User / LLM ────────►├── MCP (stdio + HTTP/SSE)
                     ├── AI tool calls (llm/tools)
                     └── Web UI (React SPA, via the REST API)
                                  │
                                  ▼
                 application/<f>  (<F>Service: use cases, transactions, events)
                                  │
                                  ▼
                 domain/<f>       (entities, invariants, pure functions)
                                  │
                                  ▼
                 infrastructure/postgres  (repositories, read models)
                                  │
                                  ▼
                             PostgreSQL
```

Interfaces parse, authenticate, resolve the caller's scope, call a service, and format the reply. They never re-derive a rule. If two interfaces disagree about a capability, one of them holds business logic that belongs in the service.

### Layers and dependency direction

| Layer | Path (under `backend/src/`) | Owns | May import |
|---|---|---|---|
| Domain | `domain/<f>/` | entities, value objects, invariants, domain errors; synchronous and I/O-free | `std`, `chrono`, `uuid`, `serde`, `thiserror` only |
| Application | `application/<f>/` | `<F>Service` use cases, transaction boundaries, event publication after commit | `domain`, `infrastructure` |
| Interface | `interface/{http,chat,channels,mcp}/` | protocol translation: Axum handlers and DTOs, `BotModule` handlers, the Matrix channel, MCP | `application`, `domain` |
| LLM | `llm/` | prompts, the tool-calling loop, conversation context ([`llm.md`](llm.md)) | `application`, `domain` |
| Infrastructure | `infrastructure/{postgres,inference,web}/` | repositories, HTTP clients for inference, LLM and web feeds | `domain`; never `application` or `interface` |

`backend/tests/architecture.rs` enforces the rules mechanically: layer imports, pure domain modules, zero SQL in `interface/` and `application/`, `AssertSqlSafe` confined to `infrastructure/postgres/merge.rs`, no `dotenvy::dotenv`.

`AppServices` (`application/mod.rs`) is built once per process by a composition root (`app.rs` for the server, `main.rs` for the stdio MCP server) and reaches every adapter as a parameter: `ApiState.services`, `ModuleContext.services`, the `services` argument of `BotModule::handle_command`, and the AI turn. No adapter constructs a service itself.

The per-feature template and the settled rulings live in [`superpowers/HANDOFF.md`](superpowers/HANDOFF.md) §8–9; the realised layout design is `superpowers/specs/2026-09-24-postgres-foundation-design.md` §3.

## Single process, two front doors

`backend/src/app.rs` is the composition root. `main.rs` only loads `./.env`, sets up tracing, handles the `mcp` stdio mode, installs the signal handler, and calls `app::run(config, token)`. `app::run` builds one shared `Database`, one `EventBroadcaster` and one `BotHandles`, then starts:

1. The Axum API server (`interface::http::start_api_server`), on a listener `app::run` binds first (`interface::http::bind`) so a bind failure stops boot rather than leaving a bot with no API and no health probes. It returns the server's `JoinHandle`.
2. The Matrix sync loop (`interface/channels/matrix/sync_loop.rs::run_bot_loop`), a reconnect loop with 5 s → 300 s backoff, receiving the same `BotHandles` via `MatrixBot::new_full`. `BotHandles` is built once and cloned into every reconnect generation, so `SyncHealth` and pending merges survive a reconnect.

`BotHandles` (bot access token, Matrix SDK client, pending merges, sync health) is the coupling point, in both directions: the REST layer borrows the *bot's* Matrix credentials to serve endpoints like `/rooms` and `/auth/media` that need homeserver access, and the bot writes into `pending_merges` so a link begun in a Matrix DM can be finished in the web UI.

**Shutdown is one `CancellationToken` tree, bounded at 15 s** (`app::SHUTDOWN_GRACE`, spec §5.3). `main.rs` owns the root and cancels it on SIGINT/SIGTERM (the handler is installed before boot, and boot's database connect is itself cancellable). The API server and the Matrix loop each get a `child_token()`:

- The API server stops through axum's graceful shutdown. SSE responses never end on their own, so `sse_handler` ends its stream on the same token (`ApiState::shutdown`) — otherwise every open browser tab would hold the drain open until the deadline.
- The Matrix loop has every long await (client build, sync, backoff) inside a `select!` against `shutdown.cancelled()`. That structure is a deliberate fix: an earlier version awaited `bot.run()` inside a select *branch body*, where Ctrl-C could not reach it. Returning drops the running generation, and with it its `BackgroundWorkers` guard, which aborts that generation's scheduler tasks (the same guard that prevents duplicate deliveries across reconnects).

After cancellation `app::stop_by` waits for both tasks against one deadline (signal + 15 s); any task still running then is named in a `warn!` and aborted — not detached, so its drop guards run, and not awaited again, since a task stuck outside an await point would hold the process past the bound. `db.pool.close()` gets whatever remains of the same deadline. Finally `main.rs` drops the runtime with `shutdown_background()`, because an ordinary runtime drop waits without limit for in-flight blocking-pool work.

The `mcp` stdio mode is not part of the tree: it is a separate process invocation that ends at stdin EOF and starts neither front door.

`BotHandles` (`interface/channels/matrix/handles.rs`) also carries `sync_health: SyncHealth` — a lock-free `Arc<AtomicI64>` of the last completed sync's unix millis, written by the sync loop's `sync_with_callback` after every successful response and read by `/health/ready`. `sync_with_callback`, not plain `sync`, is the point of the change: it is the only hook that fires once per completed response rather than once for the whole (otherwise-infinite) sync loop, so it is the one place "a sync completed" can be observed and recorded.

## Chat command plugin system

`BotModule` (`interface/chat/mod.rs`) is the extension point for Matrix commands. `ModuleRegistry::dispatch` walks modules in registration order and returns the first `Some(html_reply)`; registration happens in `interface/channels/matrix/mod.rs::new_full` and **order matters** (the TTS module must be registered before the AI module's direct-message catch-all, and a test pins that). `CoreModule` receives an `Arc<RwLock<Option<Arc<ModuleRegistry>>>>` back-reference so `!help` can enumerate all modules — a deliberate cycle-breaking indirection; keep it if you touch help generation.

Background work goes through `spawn_background_workers(&ModuleContext)`, which is how the reminder scheduler (5 s tick) gets the Matrix `Client`, the services and the event broadcaster. `interface/channels/matrix/dispatcher.rs` accepts a message either when it starts with `!` or when the bot is mentioned, strips the HTML mention pill, and yields a `CommandContext`.

Room caching and target-room resolution belong to `application/rooms` (rules in `domain/rooms`, Matrix-side discovery in `interface/chat/rooms.rs`). `interface/http/rooms.rs::list_rooms` is a *room picker* feed, not a room dump: it drops spaces, DMs, rooms not in `RoomState::Joined`, rooms lacking a name or canonical alias, and any room where the bot is not joined or *none* of the caller's linked MXIDs is. An account can have several Matrix identities, so "the caller" is a set; an account with no link gets an empty list, which is the honest answer rather than an error.

## Matrix session and crypto store

`interface/channels/matrix/client.rs` treats `data/matrix_session.json` and `data/bot_store/` as **one unit**: the session is reused only when `restore_session` plus a live `whoami` both succeed, and any other path (no session file, rejected token, device deleted in a client) archives the store to `data/bot_store.orphaned-<UTC timestamp>/` before logging in fresh. Never keep the crypto store across a fresh login — its Olm account belongs to the dead device, the crypto machine believes it already published those keys, and every message the bot then encrypts shows in Element as "Encrypted by an unknown or deleted device". The store is archived rather than deleted because it may hold the only copy of room keys that never reached key backup; `MATRIX_RECOVERY_KEY` is what repopulates a fresh store (`restore_room_keys_from_backup`).

A fresh login does **not** pin a device ID. It used to (`MATRIX_BOT_MAIN_DEVICE`, constant across every restart), which silently rotated that device's keys under the same ID on every restore failure — routine, not rare, since "no session file / rejected token / device deleted in a client" are the normal triggers. The homeserver accepts that re-upload without complaint, but every peer who already talked to the old device keeps encrypting *to* it using its old keys: the peer is the sender in that direction, so it never hits a decrypt error and has no trigger to rebuild its Olm session. The result is exactly the failure mode the paragraph above guards against — except it was also happening on ordinary token-rejection restarts, not just on manual device deletion. Now `client.rs::delete_stale_device` captures the outgoing device's ID before a fresh login (session file must be read *before* `restore_previous_session` deletes it on failure), logs in with **no** pinned ID so the homeserver assigns a fresh one, then deletes the old device via password-authenticated UIA once the new session is live. Every fresh login now presents as an unambiguous new device to every peer — the well-tested "new device, verify me" path — instead of an in-place key swap peers have no reliable way to notice.

## Verification (self and peer)

`interface/channels/matrix/self_verify.rs::ensure_self_verified` makes the bot's own session show as *Verified* in Element without a human: it signs the bot's device with the account's own self-signing key (imported from `MATRIX_RECOVERY_KEY` via secret storage) and retries because the SDK's own attempt inside `Recovery::recover()` races the first sync — it silently finds nothing to sign if the device isn't in the crypto store yet, and `recover()` returns `Ok` either way, so treat that `Ok` as "secrets imported," never as "verified." `interface/channels/matrix/verification.rs::request_verification_from_allowlist` handles the *other* trust edge — an allowlisted human verifying the bot — by sending the request from the bot's side on every boot (a client that refuses unverified sessions would otherwise block the very `!verify` message that starts the flow) unless `data/verified_users.json` already records a completed SAS with that user; that file is the only signal treated as durable, because `UserIdentity::is_verified()` reports only *our* signature over *them* and was observed true while their client was still refusing to send to the bot.

`interface/channels/matrix/self_verify.rs` is what keeps the session from showing as *Unverified* in Element. Element's rule is cross-signing: the account's self-signing key must have signed the device, with the signature on the homeserver. A password login mints unsigned device keys, so signing has to follow it — that second step, not the password, is why an Element login looks verified and a bare bot login does not. `MATRIX_RECOVERY_KEY` unlocks secret storage for two independent things: the cross-signing keys (which make the session verified) and the megolm backup key (which makes history decryptable). An account whose secret storage carries only the latter recovers its room keys and stays unverified forever.

`ensure_self_verified` runs on **every** boot, after `recover()` but regardless of whether a recovery key was configured or worked — a restored crypto store already holds the self-signing key, so a restart re-verifies with no recovery key involved. Do not fold it back into the `recover()` success branch. It retries because a fresh login races itself: the SDK attempts the same signing inside `Recovery::recover`, but only when our device is already in the crypto store, and our device keys are still uploading on the first sync — so that attempt silently finds nothing to sign while `recover()` still returns `Ok`. **Never treat `recover()` returning `Ok` as a verified session.**

Two measured facts about matrix-sdk 0.7 drive the shape of that code. `Recovery::recover` returns `Ok` even when secret storage held no self-signing key and nothing was signed (`SecretStore::import_secrets` gates signing on `has_self_signing`, and a missing secret reads as `None`, not an error). And `Device::is_cross_signed_by_owner()` on our *own* device keeps reporting false for at least a second after the signature is uploaded and re-queried, across restarts, even though `/keys/query` shows it — so the outcome is taken from `Device::verify()` returning `Ok` (the signature reached the server), never from a local re-read. Consequently the bot re-signs on most boots; that is intended and idempotent.

*(Re-measure on 0.18 at first boot — see `superpowers/runbooks/postgres-foundation-cutover.md` step 4. These facts predate the matrix-sdk 0.18 upgrade and have not yet been reconfirmed against it.)*

The acceptance test is `self_verify.rs::test_the_running_bot_session_is_verified_on_the_homeserver` — `#[ignore]`d because it needs credentials, a network, and a bot that has run at least once. It reads the token the running bot saved and asks `/keys/query`, applying Element's own rule via `device_is_cross_signed`; it never logs in, because a second login would mint another device and leave the running session's token to be reasoned about separately.

## Accounts: one owner, several ways in

`accounts` is the only thing that owns a row. `owner_id` — the column on every data table — holds an `accounts.id` and nothing else. The id is a native PostgreSQL `UUID`, typed `AccountId` in Rust and rendered `acct_<32 hex>` on the wire and in the portfolio audit hash (see [`database.md`](database.md#account-ids)).

Two access paths resolve onto it and are then indistinguishable:

- A Matrix message resolves through `matrix_links` (`interface/channels/matrix/handler.rs` → `AccountService::resolve_or_provision`).
- A web session resolves directly from the session cookie.

This replaced two disjoint identity spaces: `owner_id` used to hold a raw MXID for anything created over Matrix and `@web-<username>:api` for anything created through the API, so **one human using both saw two separate datasets**. Ownership is now carried, never derived. `AuthUser::owner_id()` returns `account_id` verbatim; if you ever find yourself computing an owner from a subject, that is the bug this design exists to remove.

An unknown MXID gets an **unclaimed** account: real data, no credential, reachable only through that MXID. That is what keeps the bot usable with no web signup. `matrix_links.mxid` is the primary key, and that alone enforces "one Matrix account, one local account" — schema-level, so a concurrent link cannot race it. `resolve_or_provision` handles the other race (the sync loop does not serialise event handlers) by having the loser delete the account it minted.

The merge (`application/accounts/merge.rs`, with its allow-listed dynamic SQL in `infrastructure/postgres/merge.rs`) absorbs one account into another, rewriting every table `OWNED_TABLES` (`domain/accounts`) lists: a table missing from it is a silent data leak, not a compile error. Sessions do **not** follow a merge — a session belongs to the credential that opened it.

## Per-user settings (timezone, default currency)

Settings is a layered feature (`domain::settings`, `application::settings::SettingsService`, `infrastructure/postgres/repositories/settings.rs`, `interface/chat/settings.rs`'s `SettingsModule`) whose data every *other* feature reads — todo and reminder format timestamps against the sender's zone, finance and portfolio default an amount's currency to it — and which `interface/http/settings.rs` exposes to the web UI. Rows in `user_settings` are keyed by `owner_id`, the same account key every other table uses, and a user has no row until they change something: this is a domain rule (`UserSettings::default()`, UTC/USD), not something baked into SQL — the repository returns `None` for a missing row, and `SettingsService::get`/`get_or_default` apply the default. `get_or_default` additionally swallows a database error into the defaults for a read that must not fail the caller's own work (a command reply, another feature's read path) — a settings lookup must never be the reason a `!todo list` reply is lost.


Every write goes through `SettingsService`, which validates `timezone`/`currency` before persisting regardless of what built the row (`save`), or reads-modifies-writes one field at a time in a single transaction with the row locked (`modify`, `SELECT ... FOR UPDATE`) so two concurrent changes for the same owner cannot interleave and lose one side's update. `SettingsModule::handle_command` broadcasts `SettingsUpdated` (via the service, after commit) on a successful save and on nothing else — a read or a rejected value announces no change. Portfolio follows the same discipline (below).


`utils::format_datetime` takes a `chrono_tz::Tz` rather than assuming UTC, and `todo`/`reminder` handlers resolve the sender's zone once, *after* the command is claimed, so an unrelated room message costs no query. The finance handler's `DEFAULT_CURRENCY` constant is gone: `parse_entry` takes the sender's saved currency, so `!expense add 9.99 coffee` books in what they actually spend. An explicit code in the command still wins.

Validation is a domain rule (`domain::settings::parse_timezone`/`parse_currency`/…), applied at every boundary that writes — the HTTP `PUT` handler, each Matrix `!settings` subcommand, and the AI tool alike — not at render time: an unknown zone stored now would render as UTC on every later read with nothing to point at. `parse_timezone` accepts exactly the IANA names `Intl.supportedValuesOf('timeZone')` offers the picker; `parse_currency` requires a 3-letter code, and `domain::finance::money::normalise_currency` delegates to it rather than re-deriving the same rule.

On the frontend, `core/settings.ts` is the single source — `useSettings()` reads one shared React Query entry, so a change reformats the whole app from one cache write. Anything rendering an instant must pass the zone explicitly (`formatDateTimeInZone` and friends); a bare `toLocaleString` silently uses the browser's zone, which is exactly what the setting exists to override. The same applies in reverse for *input*: `zonedInputToUtcIso`/`utcIsoToZonedInput` convert the `DateTimePicker`'s naive `YYYY-MM-DDTHH:mm` against the chosen zone, applying the offset twice so a daylight-saving boundary cannot land the result an hour out. `DateTimePicker` itself stays zone-agnostic and takes a `now` prop purely so "today" and the "Now" button mean the right day.

## Voice replies (`!say`)

`interface/chat/tts` speaks a reply into the room as a native Matrix voice message. It prepares the text (`text.rs` strips HTML and emoji — the phonemiser has no reading for `🔊` or `✅`, and leaving them in produces unpredictable silent gaps mid-sentence), resolves which voice speaks, and uploads the result. Synthesis itself happens elsewhere: `infrastructure/inference/tts` reaches `POST /v1/audio/speech`, served by the bundled inference server's Kokoro (`inference/src/tts/kokoro.rs`) or by any OpenAI-compatible or ElevenLabs provider, chosen by `TTS_BASE_URL` / `TTS_DIALECT`. An unset base URL makes `!say` answer that speech is unavailable, and nothing else changes — the same degrade-one-feature shape `infrastructure/inference/ocr` and `infrastructure/inference/stt` use.

The reply is **Ogg Opus, not WAV**, and that is what makes a client draw a voice bubble instead of a file card. The bundled server adds `X-Duration-Ms` and `X-Waveform` response headers, which a Matrix voice message needs and raw audio does not carry. A provider that omits them — every third-party one — still produces a bubble: `infrastructure::inference::tts::ogg_opus_duration` reads the duration straight from the Ogg container, the last page's granule position minus the `OpusHead` pre-skip, and the bubble simply has no amplitude bars. The granule arithmetic of the encoder itself (`inference/src/tts/opus.rs`) has no equivalent local check; it is verified externally with `ffprobe` over the route's own output.

The per-user voice is `user_settings.tts_voice: Option<String>`, and `TTS_VOICE` is the house default for someone who has never chosen. It is an `Option` rather than a plain string for the same reason it was an `Option<i64>` before this crate split: `"0"` is a real Kokoro voice, so a bare default would make "never customised" and "chose voice 0" indistinguishable. `!settings voice <voice|none>` validates only the *shape* of a saved value (`parse_tts_voice`) — a Kokoro id, an OpenAI name and an ElevenLabs id all fit the same shape, and the bot has no way to know which voices a configured provider actually has. The bundled server is the one place that does know: it clamps an out-of-range Kokoro id to the nearest valid voice (`kokoro::clamp_speaker_id`) rather than failing the request, and answers 400 to a voice that is not an integer at all.

The Kokoro engine loads once into a `static OnceLock`, deliberately unlike `inference/src/stt/sherpa.rs`, which rebuilds its SenseVoice recogniser on every call — that is fine for a small model loading in milliseconds and wrong for Kokoro, which is 311 MB and takes seconds. It is provisioned at inference server startup the same way the other models are: downloaded from the sherpa-onnx `tts-models` release archive, unpacked into a staging directory, and renamed into place so a route never sees a partial model. Its text-normalisation FSTs ship inside that same archive.

Two build facts follow from the encoder and the runtime living in a separate crate. `matrix-bot-inference` needs `cmake`, because `opusic-sys` compiles a vendored libopus from source for `inference/src/tts/opus.rs`; the bot needs neither `cmake` nor any sherpa shared library at all. On a Windows development machine, a test that loads the real Kokoro model needs sherpa's own `onnxruntime.dll` copied into `target/debug/deps/` — see that test's own comment in `inference/src/tts/kokoro.rs` for why a version mismatch there dies with an access violation rather than a clean error.

## External services and the inference crate

The workspace has two members. `backend/` builds `matrix-bot`; `inference/` builds `matrix-bot-inference`, an Axum server that hosts the local ONNX models. `default-members = ["backend"]` keeps bare `cargo run` meaning the bot, and `cargo test -p matrix-bot` is the everyday suite — `cargo test --workspace` additionally builds the crate carrying the native ML dependencies, which is why CI scopes the backend jobs explicitly.

OCR reaches the inference server over HTTP via `infrastructure/inference/ocr`, speech to text via `infrastructure/inference/stt`, and text to speech via `infrastructure/inference/tts`. STT speaks OpenAI's `POST /v1/audio/transcriptions` — so `STT_BASE_URL` can point at the bundled server, OpenAI, Groq or anything else with that shape, and `STT_API_KEY` is sent as a bearer token. The client asks for `response_format=json`, the one format every compatible transcription model accepts (OpenAI's `gpt-4o-*-transcribe` models reject `verbose_json`); a voice note's duration prefers the provider's own figure when one comes back, falling back to the Matrix event's own `AudioInfo.duration` otherwise — a `json` response carries none, so that fallback is the usual case. Text to speech reaches `POST /v1/audio/speech` via `infrastructure/inference/tts`, which speaks the OpenAI shape and, under `TTS_DIALECT=elevenlabs`, the ElevenLabs one. An unset base URL, or an unreachable server, degrades that one feature: the bot still boots and stays a working bot without it, given a reachable PostgreSQL `DATABASE_URL` — there is no local-volume fallback for the application database.

The inference server provisions models in the background (`provision.rs`), spawned beside the listener rather than awaited in front of it, so `/health` answers at once and each route reports 503 until its own files land. Every wire contract between the two crates that a client actually consumes is a JSON fixture in `contracts/` asserted by both sides — a format only one side speaks (the inference server's `verbose_json`, which the bot never requests) is asserted by that side alone — because the two crates' types are separate by design and nothing else would notice a renamed field.

`backend/src/infrastructure/inference/` is the boundary this split created. `interface/` and `application/` define what the application does; `infrastructure/` is how external capabilities are reached. Nothing in `infrastructure/` may import from `interface/` or `application/` — `infrastructure::inference::ocr::OcrScan` is a separate type from `interface::chat::ocr::OcrResult` for exactly that reason, with a `From` impl doing the mapping.

## Portfolio (investment holdings)

Portfolio tracks investment positions — symbol, quantity, weighted-average cost basis — manually. There is no live price feed anywhere in this codebase; `!portfolio price` and the web UI's price field are the only way a holding's current value updates, and market value / unrealized P&L are `Option`, not zero, until a price has been set.

Quantity goes through the same integer-minor-units technique `finance::money` uses for currency, at a fixed `QUANTITY_EXPONENT = 8` (enough for a fractional share or satoshi-level crypto) — `parse_amount`/`format_amount` are currency-agnostic, so `portfolio` calls them directly rather than reimplementing decimal parsing. Weighted-average cost on a buy, and realized P&L on a sell, both rely on quantity's `10^8` scale cancelling against the divisor in `models.rs::weighted_avg_cost_minor`/`realized_pl_minor` — the arithmetic is `i128` throughout because a large crypto quantity times a large price can approach `i64`'s ceiling. A second currency for a symbol already held is rejected (`BuyOutcome::CurrencyMismatch`), the same principle as `finance` never summing two currencies: there is no rate source to reconcile them.

Selling the full quantity held deletes the row — average cost has no meaning for a position of zero. `portfolio_holdings` has no `room_id`/`sender_name`, unlike `transactions`: a holding is a per-owner aggregate like `finance_categories`, not a room-broadcast event log, so there is nothing room-scoped to record.

**Cash is a hard precondition for buying, never for selling.** `portfolio_cash_balances` holds one row per `(owner_id, currency)`, the same "no row until you touch it" convention as `user_settings` — a missing row reads as a zero balance, enforced in `portfolio::cash_repo`, not a schema `CHECK` (nothing in the migrations uses one). `trade::buy` calls `cash_repo::withdraw_cash` as its funding gate: the "is there enough?" check and the debit happen inside that one function, so there is no separate check-then-act window for a concurrent buy to race — if funds are short, `BuyOutcome::InsufficientCash` comes back and *nothing* is written, on either table. `sell` always succeeds in crediting cash with the **gross proceeds** (`cost_of(qty_sold, sell_price)`), not the realized P&L — a losing sale still returns the principal that came back out. `set_price`/`set_note`/`delete` never touch cash: only `buy` and `sell`, the two commands that represent an actual money-for-shares exchange, do. The holding write and the cash write happen inside one transaction in `application::portfolio::trade`, so a buy cannot debit cash without recording the position.

## Frontend module mirror

The SPA mirrors the backend plugin shape: `core/registry.ts` holds `FrontendModule`s registered in `App.tsx` (`core`, `todo`, `reminder`). Each module supplies a `MainView`, optional `navTab`/`getStatItems`, and a `handleSseEvent(event, queryClient)` used to invalidate React Query caches from the live stream (`core/useSystemEvents.ts`). Adding a backend module means adding a matching `frontend/src/modules/<name>/index.tsx` and registering it in `App.tsx`.

All API calls go through `core/apiClient.ts` (`fetchApi`), which sends `credentials: 'include'` — the session is an HttpOnly cookie, so there is no token to read or store — and unwraps the standardized `{ "error": "...", "code": 400 }` shape produced by `backend/src/error.rs::ApiError`.

`core/session.ts` is the single source of "who is signed in": the cookie is unreadable by the page, so only the server can answer, and `useSession()` asking `/auth/me` is that answer. There is deliberately no client-side token to inspect and nothing that can drift from the server's view.

`core/security.ts::useSensitiveAction` wraps any step-up-gated call. A 403 is not an error to show someone — it is the server asking for a passkey — so it prompts, retries, and only reports a second failure.
