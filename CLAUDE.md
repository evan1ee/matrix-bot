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

Cargo workspace at the root with two members, plus a separate npm frontend:

- `backend/` — Rust crate `matrix-bot`: Matrix bot + embedded Axum REST API in one binary.
- `inference/` — Rust crate `matrix-bot-inference`: Axum server hosting the local ONNX models (OCR, speech-to-text and Kokoro text-to-speech).
- `frontend/` — React 19 + Vite + Tailwind v4 SPA, built into `backend/web/dist/`.
- `data/` — runtime SQLite DB + Matrix SDK crypto store (gitignored).

- `docs/` — `testing-auth-locally.md` covers running passkeys against `localhost` (virtual authenticator, the `!link` DM flow, SQL to skip the step-up and recovery delays).

`AGENTS.md` holds the authoritative development conventions (module checklist, file-size limits, SQLx and HTML-formatting rules). Read it before adding a feature module.

## Commands

```bash
# Backend (run from repo root — paths below are cwd-relative)
cargo check                      # must be warning-free
cargo test                       # backend suite only, all in-crate #[cfg(test)] modules —
                                  # default-members = ["backend"] means bare `cargo test`
                                  # never touches the inference crate
cargo test -p matrix-bot-inference  # the inference crate's suite; not covered by bare `cargo test`
cargo test --package matrix-bot self_verify -- --ignored   # live check: is the running bot's session verified?
cargo test test_parse_message_exclamation_command   # single test by name
cargo test --package matrix-bot modules::todo::     # one module's tests
cargo run                        # starts bot + API server

# Migrate a COPY of a real database and assert nothing was stranded. Run this
# before shipping any migration that touches ownership.
cp data/bot_data.db /tmp/copy.db
MIGRATE_TEST_DB=/tmp/copy.db cargo test migrates_a_real_database -- --ignored

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

1. The Axum API server (`api::start_api_server`), which returns three shared handles — `Arc<RwLock<Option<String>>>` (bot access token), `Arc<RwLock<Option<Client>>>` (Matrix SDK client), and `Arc<PendingMerges>` (merges a `!link` recorded, waiting on web confirmation).
2. The Matrix sync loop in a `tokio::spawn` retry loop (reconnects every 5s on error), receiving those same handles via `MatrixBot::new_full`.

Those handles are the coupling point, in both directions: the REST layer borrows the *bot's* Matrix credentials to serve endpoints like `/rooms` and `/auth/media` that need homeserver access, and the bot writes into `pending_merges` so a link begun in a Matrix DM can be finished in the web UI. Shutdown is a `broadcast::channel::<()>` fanned out to both halves on Ctrl-C.

`api/rooms.rs::list_rooms` is a *room picker* feed, not a room dump: it drops spaces, DMs (`room.is_direct()`), rooms not in `RoomState::Joined`, rooms lacking a name or canonical alias, and any room where the bot is not a joined member or *none* of the caller's linked MXIDs is. An account can have several Matrix identities, so "the caller" is a set — an account with no link gets an empty list, which is the honest answer rather than an error. `bot/client.rs` treats `data/matrix_session.json` and `data/bot_store/` as **one unit**: the session is reused only when `restore_session` plus a live `whoami` both succeed, and any other path (no session file, rejected token, device deleted in a client) archives the store to `data/bot_store.orphaned-<UTC timestamp>/` before logging in fresh. Never keep the crypto store across a fresh login — its Olm account belongs to the dead device, the crypto machine believes it already published those keys, and every message the bot then encrypts shows in Element as "Encrypted by an unknown or deleted device". The store is archived rather than deleted because it may hold the only copy of room keys that never reached key backup; `MATRIX_RECOVERY_KEY` is what repopulates a fresh store (`restore_room_keys_from_backup`).

A fresh login does **not** pin a device ID. It used to (`MATRIX_BOT_MAIN_DEVICE`, constant across every restart), which silently rotated that device's keys under the same ID on every restore failure — routine, not rare, since "no session file / rejected token / device deleted in a client" are the normal triggers. The homeserver accepts that re-upload without complaint, but every peer who already talked to the old device keeps encrypting *to* it using its old keys: the peer is the sender in that direction, so it never hits a decrypt error and has no trigger to rebuild its Olm session. The result is exactly what CLAUDE.md already described as the failure mode this section guards against — except it was also happening on ordinary token-rejection restarts, not just on manual device deletion. Now `client.rs::delete_stale_device` captures the outgoing device's ID before a fresh login (session file must be read *before* `restore_previous_session` deletes it on failure), logs in with **no** pinned ID so the homeserver assigns a fresh one, then deletes the old device via password-authenticated UIA once the new session is live. Every fresh login now presents as an unambiguous new device to every peer — the well-tested "new device, verify me" path — instead of an in-place key swap peers have no reliable way to notice.

`bot/self_verify.rs::ensure_self_verified` makes the bot's own session show as *Verified* in Element without a human: it signs the bot's device with the account's own self-signing key (imported from `MATRIX_RECOVERY_KEY` via secret storage) and retries because the SDK's own attempt inside `Recovery::recover()` races the first sync — it silently finds nothing to sign if the device isn't in the crypto store yet, and `recover()` returns `Ok` either way, so treat that `Ok` as "secrets imported," never as "verified." `bot/verification.rs::request_verification_from_allowlist` handles the *other* trust edge — an allowlisted human verifying the bot — by sending the request from the bot's side on every boot (a client that refuses unverified sessions would otherwise block the very `!verify` message that starts the flow) unless `data/verified_users.json` already records a completed SAS with that user; that file is the only signal treated as durable, because `UserIdentity::is_verified()` reports only *our* signature over *them* and was observed true while their client was still refusing to send to the bot.

`bot/self_verify.rs` is what keeps the session from showing as *Unverified* in Element. Element's rule is cross-signing: the account's self-signing key must have signed the device, with the signature on the homeserver. A password login mints unsigned device keys, so signing has to follow it — that second step, not the password, is why an Element login looks verified and a bare bot login does not. `MATRIX_RECOVERY_KEY` unlocks secret storage for two independent things: the cross-signing keys (which make the session verified) and the megolm backup key (which makes history decryptable). An account whose secret storage carries only the latter recovers its room keys and stays unverified forever.

`ensure_self_verified` runs on **every** boot, after `recover()` but regardless of whether a recovery key was configured or worked — a restored crypto store already holds the self-signing key, so a restart re-verifies with no recovery key involved. Do not fold it back into the `recover()` success branch. It retries because a fresh login races itself: the SDK attempts the same signing inside `Recovery::recover`, but only when our device is already in the crypto store, and our device keys are still uploading on the first sync — so that attempt silently finds nothing to sign while `recover()` still returns `Ok`. Never treat `recover()` returning `Ok` as a verified session.

Two measured facts about matrix-sdk 0.7 drive the shape of that code. `Recovery::recover` returns `Ok` even when secret storage held no self-signing key and nothing was signed (`SecretStore::import_secrets` gates signing on `has_self_signing`, and a missing secret reads as `None`, not an error). And `Device::is_cross_signed_by_owner()` on our *own* device keeps reporting false for at least a second after the signature is uploaded and re-queried, across restarts, even though `/keys/query` shows it — so the outcome is taken from `Device::verify()` returning `Ok` (the signature reached the server), never from a local re-read. Consequently the bot re-signs on most boots; that is intended and idempotent.

The acceptance test is `self_verify.rs::test_the_running_bot_session_is_verified_on_the_homeserver` — `#[ignore]`d because it needs credentials, a network, and a bot that has run at least once. It reads the token the running bot saved and asks `/keys/query`, applying Element's own rule via `device_is_cross_signed`; it never logs in, because a second login would mint another device and leave the running session's token to be reasoned about separately.

### Two crates, one deployment

The workspace has two members. `backend/` builds `matrix-bot`; `inference/`
builds `matrix-bot-inference`, an Axum server that hosts the local ONNX
models. `default-members = ["backend"]` keeps bare `cargo run` meaning the
bot, and `cargo test -p matrix-bot` is the everyday suite —
`cargo test --workspace` additionally builds the crate carrying the native ML
dependencies, which is why CI scopes the backend jobs explicitly.

OCR reaches the inference server over HTTP via `services/ocr`, speech to
text via `services/stt`, and text to speech via `services/tts`. STT speaks
OpenAI's `POST /v1/audio/transcriptions` — so `STT_BASE_URL` can point at the
bundled server, OpenAI, Groq or anything else with that shape, and
`STT_API_KEY` is sent as a bearer token. The client asks for
`response_format=json`, the one format every compatible transcription model
accepts (OpenAI's `gpt-4o-*-transcribe` models reject `verbose_json`); a voice
note's duration prefers the provider's own figure when one comes back, falling
back to the Matrix event's own `AudioInfo.duration` otherwise — a `json`
response carries none, so that fallback is the usual case. Text to speech
reaches `POST /v1/audio/speech` via `services/tts`, which speaks the OpenAI
shape and, under `TTS_DIALECT=elevenlabs`, the ElevenLabs one. An unset base
URL, or an unreachable server, degrades that one feature: the bot still boots,
and `docker run` with a volume still yields a working bot without it.

The inference server provisions models in the background (`provision.rs`),
spawned beside the listener rather than awaited in front of it, so `/health`
answers at once and each route reports 503 until its own files land. Every
wire contract between the two crates that a client actually consumes is a
JSON fixture in `contracts/` asserted by both sides — a format only one side
speaks (the inference server's `verbose_json`, which the bot never requests)
is asserted by that side alone — because the two crates' types are separate
by design and nothing else would notice a renamed field.

`backend/src/services/` is the boundary this split created. `modules/` is what
the application does; `services/` is how external capabilities are reached.
Nothing in `services/` may import from `modules/` — `services::ocr::OcrScan`
is a separate type from `modules::ocr::OcrResult` for exactly that reason,
with a `From` impl in the module doing the mapping.

### Backend module plugin system

`modules::BotModule` (in `backend/src/modules/mod.rs`) is the extension point. `ModuleRegistry::dispatch` walks modules in registration order and returns the first `Some(html_reply)`. Registration happens in `bot/mod.rs::new_full` — Core, Todo, Reminder, Finance, Portfolio, Settings. `CoreModule` receives an `Arc<RwLock<Option<Arc<ModuleRegistry>>>>` back-reference so `!help` can enumerate all modules (a deliberate cycle-breaking indirection; keep it if you touch help generation).

Background work goes through `spawn_background_workers(&ModuleContext)` — that's how the reminder scheduler (5s tick loop) gets the Matrix `Client`, `Database`, and broadcaster.

`bot/dispatcher.rs` accepts a message either when it starts with `!` or when the bot is mentioned, strips the HTML mention pill, and yields a `CommandContext`.

### Event flow (SSE)

Every state mutation — from both the Matrix command path and the REST path — must `broadcaster.send(SystemEvent::…)`. `SystemEvent` is serialized `#[serde(tag = "type", content = "payload")]` and sent as an **unnamed** SSE message, so the frontend reads `EventSource.onmessage` and dispatches on the `type` field (the variant name verbatim, e.g. `TodoCreated` — not snake_case).

The broadcast channel is process-wide. `sse::owned_events` narrows it to one owner via `SystemEvent::owner_id()`, so a client sees only its own rows. **Any new variant must return a meaningful `owner_id()`** — the deletion variants carry an explicit `owner_id` field because their row is already gone. Adding a mutation without a matching event silently breaks the live UI; adding a variant without an owner leaks it or drops it.

### Accounts: one owner, several ways in

`accounts` is the only thing that owns a row. `owner_id` — the column on every
data table — holds an `accounts.id` (`acct_` + 32 hex chars) and nothing else.

Two access paths resolve onto it and are then indistinguishable:

- A Matrix message resolves through `matrix_links` (`bot/handler.rs` →
  `AccountRepository::resolve_or_provision`).
- A web session resolves directly from the session cookie.

This replaced two disjoint identity spaces: `owner_id` used to hold a raw MXID
for anything created over Matrix and `@web-<username>:api` for anything created
through the API, so **one human using both saw two separate datasets**. Ownership
is now carried, never derived. `AuthUser::owner_id()` returns `account_id`
verbatim; if you ever find yourself computing an owner from a subject, that is
the bug this design exists to remove.

An unknown MXID gets an **unclaimed** account: real data, no credential,
reachable only through that MXID. That is what keeps the bot usable with no web
signup. `matrix_links.mxid` is the primary key, and that alone enforces "one
Matrix account, one local account" — schema-level, so a concurrent link cannot
race it. `resolve_or_provision` handles the other race (the sync loop does not
serialise event handlers) by having the loser delete the account it minted.

`accounts/merge.rs` absorbs one account into another, rewriting `OWNED_TABLES`.
That constant is shared with the Phase 1 backfill deliberately: a table missing
from it is a silent data leak, not a compile error. Sessions do **not** follow a
merge — a session belongs to the credential that opened it.

### Auth: passkeys and cookie sessions

Passkeys are the only way to establish a session in the browser. There is still
no password and no JWT; `bcrypt` survives only to hash recovery codes. A native
client (Element X iOS) exchanges a Matrix session for a backend session token
and carries it as `Authorization: Bearer` instead of a cookie, and an MCP
client authenticates the same way with an `mb_mcp_`-prefixed personal access
token — see `README.md` for both. An MCP token never satisfies the step-up
gate below: it is a machine credential, so its creation time is never read as
a passkey assertion.

`api/auth/webauthn.rs` runs two ceremonies. Sign-in is **discoverable**: the
credential carries the account handle, so there is no username field anywhere and
no user enumeration to defend against. `accounts::account_id_to_handle` is a
total bijection between an account id and the WebAuthn user handle — an account
id is `acct_` plus the 32 hex chars of 16 random bytes, which is exactly a UUID's
simple form, so no column and no UUID generation are needed.

`register/start` mints the account id but writes **no row**; the id has to travel
inside the challenge, and an abandoned ceremony must leave nothing behind. Hence
`create_with_id` alongside `create`.

`accounts/session.rs` holds sessions server-side. Only the SHA-256 of the cookie
is stored, so a leaked backup cannot be replayed. Both timeouts are checked on
read rather than swept, so expiry never waits on a background job.

`api/auth/extractor.rs` reads whichever credential the request carries and
enforces CSRF twice: `SameSite=Strict`, plus an `Origin` check on every
state-changing request that arrived on the ambient session cookie. A missing
`Origin` on such a mutating request is **refused** — browsers always send one on
`fetch`. A request carrying an explicit `Authorization: Bearer` credential is
exempt from the Origin requirement, because a hostile site's ambient authority
can never attach a header it does not know the value of.

`VerifiedUser` is the step-up boundary: it wraps `AuthUser` and rejects a session
whose last passkey assertion is older than `STEP_UP_WINDOW_MINUTES` (5), or that
came from account recovery and is still restricted. Putting it in a handler
signature is the whole opt-in. **`api/mod.rs::step_up_tests` drives the real
router** (via the extracted `api_routes()`) and fails if a sensitive route
accepts a stale session — add a sensitive endpoint without `VerifiedUser` and CI
catches it.

`WEBAUTHN_RP_ID` is a bare hostname and ignores ports; `WEBAUTHN_ORIGIN` carries
the port and is what the `Origin` header is compared against. Browsing the Vite
dev server on `:5173` while `WEBAUTHN_ORIGIN` says `:8080` makes every write fail
with "This request did not come from this site". `Config::validate` refuses to
boot production on the localhost defaults, because passkeys minted against
`localhost` are ones no real browser would return.

### Linking, recovery, and the Matrix ceiling

`!link` is handled in `bot/linking.rs` **before** the sender is resolved, and
that ordering is load-bearing: resolving first would provision an empty account
for the very MXID being claimed, turning every first-time link into a merge of
nothing. Do not move it into a `BotModule`.

Proof of ownership is the message sender, so no password is involved. The bot
refuses `!link` outside a DM and burns the code — in a public room everyone can
read it, and a bystander could send it from their own MXID. A link that finds
existing data does not merge; it reports the count and records a pending merge in
`ApiState.pending_merges`, which is also the *authorisation* for
`confirm-merge` (without it, naming any account id would absorb it).

Unlinking keeps every row. Data belongs to the account; unlinking revokes an
access path, and the MXID comes back as a stranger.

Recovery has two independent routes. Codes sign you straight in. `!recover` is
fenced because it would otherwise close a circle — linking needs a recent
passkey, and passkey recovery would need only the Matrix account: the request is
announced immediately, does nothing for 24h, any live session can cancel it, and
it then grants a session restricted for 72h. Cancelling deliberately takes
`AuthUser`, not `VerifiedUser` — demanding a passkey to stop a recovery you did
not ask for is backwards.

`modules/account/mod.rs::WEB_ONLY` is the capability ceiling. Matrix cannot
prompt for a passkey, so it gets **no** route to a privileged action; `!export`,
`!unlink`, `!passkey`, `!sessions` and `!recoverycodes` answer with a pointer to
the web app. That refusal is what makes the step-up rule a guarantee rather than
a decoration.

Two of those are forward-looking rather than descriptive: **there is no financial
export feature yet**, and `SecurityEventKind::FinancialDataExported` is
constructed nowhere. The command name and the event variant are reserved so that
whoever builds export puts it behind `VerifiedUser` on the web side rather than
discovering the requirement afterwards. Do not read them as evidence the feature
exists.

`bot/notify.rs` records a `security_events` row and DMs every linked identity,
opening a DM room if none exists. Neither half can fail the action it describes —
by the time it runs, the thing has happened.

### Per-user settings (timezone, default currency)

`modules/settings/` is an ordinary `BotModule` (`!settings`, `!settings timezone <IANA zone>`, `!settings currency <CODE>`; `tz`/`cur` are accepted aliases) whose data every *other* module reads — todo and reminder format timestamps against the sender's zone, finance defaults an amount's currency to it — and which `api/settings.rs` exposes to the web UI. Rows in `user_settings` are keyed by `owner_id`, the same account key every other table uses, and a user has no row until they change something: `SettingsRepository::get` returns `UserSettings::default()` (UTC/USD) for a missing row, which is the normal case, not an error. The command path uses `get_or_default`, which additionally swallows a database error into the defaults — a settings lookup must never be the reason a `!todo list` reply is lost.

Because it mutates state from the Matrix side, `BotModule::handle_command` takes an `Option<&EventBroadcaster>` (threaded from `bot/mod.rs` → `register_handlers` → `ModuleRegistry::dispatch`). It is `None` only in tests. The settings handler broadcasts `SettingsUpdated` on a successful save and on nothing else — a read or a rejected value announces no change. Portfolio follows the same discipline (below). **Todo, Reminder, and Finance do not yet broadcast from the Matrix path** (`due_reminder.rs` documents that gap); the parameter is there so they can, and any new mutating command should use it.

`utils::format_datetime` takes a `chrono_tz::Tz` rather than assuming UTC, and `todo`/`reminder` handlers resolve the sender's zone once, *after* the command is claimed, so an unrelated room message costs no query. The finance handler's `DEFAULT_CURRENCY` constant is gone: `parse_entry` takes the sender's saved currency, so `!expense add 9.99 coffee` books in what they actually spend. An explicit code in the command still wins.

Validation lives at the API boundary (`api/settings.rs`), not at render time: an unknown zone stored now would render as UTC on every later read with nothing to point at. `parse_timezone` accepts exactly the IANA names `Intl.supportedValuesOf('timeZone')` offers the picker; currency goes through the finance module's `normalise_currency`.

On the frontend, `core/settings.ts` is the single source — `useSettings()` reads one shared React Query entry, so a change reformats the whole app from one cache write. Anything rendering an instant must pass the zone explicitly (`formatDateTimeInZone` and friends); a bare `toLocaleString` silently uses the browser's zone, which is exactly what the setting exists to override. The same applies in reverse for *input*: `zonedInputToUtcIso`/`utcIsoToZonedInput` convert the `DateTimePicker`'s naive `YYYY-MM-DDTHH:mm` against the chosen zone, applying the offset twice so a daylight-saving boundary cannot land the result an hour out. `DateTimePicker` itself stays zone-agnostic and takes a `now` prop purely so "today" and the "Now" button mean the right day.

### Voice replies (`!say`)

`modules/tts` speaks a reply into the room as a native Matrix voice message.
It prepares the text (`text.rs` strips HTML and emoji — the phonemiser has no
reading for `🔊` or `✅`, and leaving them in produces unpredictable silent
gaps mid-sentence), resolves which voice speaks, and uploads the result.
Synthesis itself happens elsewhere: `services/tts` reaches
`POST /v1/audio/speech`, served by the bundled inference server's Kokoro
(`inference/src/kokoro.rs`) or by any OpenAI-compatible or ElevenLabs
provider, chosen by `TTS_BASE_URL` / `TTS_DIALECT`. An unset base URL makes
`!say` answer that speech is unavailable, and nothing else changes — the same
degrade-one-feature shape `services/ocr` and `services/stt` use.

The reply is **Ogg Opus, not WAV**, and that is what makes a client draw a
voice bubble instead of a file card. The bundled server adds
`X-Duration-Ms` and `X-Waveform` response headers, which a Matrix voice
message needs and raw audio does not carry. A provider that omits them —
every third-party one — still produces a bubble:
`services::tts::ogg_opus_duration` reads the duration straight from the Ogg
container, the last page's granule position minus the `OpusHead` pre-skip,
and the bubble simply has no amplitude bars. The granule arithmetic of the
encoder itself (`inference/src/opus.rs`) has no equivalent local check; it is
verified externally with `ffprobe` over the route's own output.

The per-user voice is `user_settings.tts_voice: Option<String>`, and
`TTS_VOICE` is the house default for someone who has never chosen. It is an
`Option` rather than a plain string for the same reason it was an
`Option<i64>` before this crate split: `"0"` is a real Kokoro voice, so a bare
default would make "never customised" and "chose voice 0" indistinguishable.
`!settings voice <voice|none>` validates only the *shape* of a saved value
(`parse_tts_voice`) — a Kokoro id, an OpenAI name and an ElevenLabs id all fit
the same shape, and the bot has no way to know which voices a configured
provider actually has. The bundled server is the one place that does know: it
clamps an out-of-range Kokoro id to the nearest valid voice
(`kokoro::clamp_speaker_id`) rather than failing the request, and answers 400
to a voice that is not an integer at all.

The Kokoro engine loads once into a `static OnceLock`, deliberately unlike
`inference/src/sherpa.rs`, which rebuilds its SenseVoice recogniser on every
call — that is fine for a small model loading in milliseconds and wrong for
Kokoro, which is 311 MB and takes seconds. It is provisioned at inference
server startup the same way the other models are: downloaded from the
sherpa-onnx `tts-models` release archive, unpacked into a staging directory,
and renamed into place so a route never sees a partial model. Its
text-normalisation FSTs ship inside that same archive.

Two build facts follow from the encoder and the runtime now living in a
separate crate. `matrix-bot-inference` needs `cmake`, because `opusic-sys`
compiles a vendored libopus from source for `inference/src/opus.rs`; the bot
needs neither `cmake` nor any sherpa shared library at all. On a Windows
development machine, a test that loads the real Kokoro model needs sherpa's
own `onnxruntime.dll` copied into `target/debug/deps/` — see that test's own
comment in `inference/src/kokoro.rs` for why a version mismatch there dies
with an access violation rather than a clean error.

### Portfolio (investment holdings)

`modules/portfolio/` tracks investment positions — symbol, quantity, weighted-average cost basis — manually. There is no live price feed anywhere in this codebase; `!portfolio price` and the web UI's price field are the only way a holding's current value updates, and market value / unrealized P&L are `Option`, not zero, until a price has been set.

Quantity goes through the same integer-minor-units technique `finance::money` uses for currency, at a fixed `QUANTITY_EXPONENT = 8` (enough for a fractional share or satoshi-level crypto) — `parse_amount`/`format_amount` are currency-agnostic, so `portfolio` calls them directly rather than reimplementing decimal parsing. Weighted-average cost on a buy, and realized P&L on a sell, both rely on quantity's `10^8` scale cancelling against the divisor in `models.rs::weighted_avg_cost_minor`/`realized_pl_minor` — the arithmetic is `i128` throughout because a large crypto quantity times a large price can approach `i64`'s ceiling. A second currency for a symbol already held is rejected (`BuyOutcome::CurrencyMismatch`), the same principle as `finance` never summing two currencies: there is no rate source to reconcile them.

Selling the full quantity held deletes the row — average cost has no meaning for a position of zero. `portfolio_holdings` has no `room_id`/`sender_name`, unlike `transactions`: a holding is a per-owner aggregate like `finance_categories`, not a room-broadcast event log, so there is nothing room-scoped to record.

**Cash is a hard precondition for buying, never for selling.** `portfolio_cash_balances` holds one row per `(owner_id, currency)`, the same "no row until you touch it" convention as `user_settings` — a missing row reads as a zero balance, enforced in `portfolio::cash_repo`, not a schema `CHECK` (nothing else in `schema.rs` uses one). `trade::buy` calls `cash_repo::withdraw_cash` as its funding gate: the "is there enough?" check and the debit happen inside that one function, so there is no separate check-then-act window for a concurrent buy to race — if funds are short, `BuyOutcome::InsufficientCash` comes back and *nothing* is written, on either table. `sell` always succeeds in crediting cash with the **gross proceeds** (`cost_of(qty_sold, sell_price)`), not the realized P&L — a losing sale still returns the principal that came back out. `set_price`/`set_note`/`delete` never touch cash: only `buy` and `sell`, the two commands that represent an actual money-for-shares exchange, do. The holding write and the cash write happen inside one transaction in `modules/portfolio/trade.rs`, so a buy cannot debit cash without recording the position.

### Persistence

`db/schema.rs::run_migrations` is the only migration mechanism: idempotent `CREATE TABLE IF NOT EXISTS`, best-effort `ALTER TABLE` (errors deliberately swallowed with `let _ =`), and data-fixup `UPDATE`s. There is no `sqlx migrate` directory.

Two one-shot conversions live there and are guarded rather than versioned, since there is no migration table. `backfill_accounts` runs only when `accounts` is empty and converts legacy `owner_id` values onto real accounts. After it, any `active` account holding no passkey is demoted to `unclaimed` — passwords are gone, so calling it active would assert something the schema can no longer honour. Ordering matters: the demotion must run *after* the backfill that creates those rows.

The ownership column was renamed `sender_mxid` → `owner_id` on `todos`, `reminders`, `transactions` and `recurring_bills`. The renames run *first*, before any index or read touches the column, and are best-effort so a fresh database (no table) and an already-migrated one (already renamed) both no-op.

### Two databases, one set of queries

`DATABASE_URL` picks SQLite or PostgreSQL at boot and everything else follows.
SQLite stays the default: the standalone binary and `docker run` with a volume
need no server. Postgres is a supported target, not a compatibility shim.

**Never call `sqlx::query*` directly.** `db/executor.rs` holds `DbPool` (an enum
over both drivers) and the macros every call site uses — `db_fetch_all!`,
`db_fetch_one!`, `db_fetch_optional!`, `db_scalar_all!`, `db_scalar_one!`,
`db_scalar_optional!`, `db_execute!`, plus `db_fetch_all_dyn!` for the few list
endpoints whose bind count depends on which filters a request carries, and a
`tx_*` family taking `&mut DbTx`. SQL is written once in SQLite's `?` style and
`dialect::render_placeholders` rewrites it to `$1..$n` for Postgres. Pass `_` as
the row type where context infers it.

`sqlx::Any` cannot do this job, which is measured rather than assumed: its type
set in 0.7 is `Null, Bool, SmallInt, Integer, BigInt, Real, Double, Text, Blob`
— **no chrono** — so every `DateTime<Utc>` column would regress to `String`,
which `schema_todos.rs` records as a bug already fixed once. What makes the
macro approach work instead is that one `#[derive(FromRow)]` model satisfies
both `SqliteRow` and `PgRow`; `db/dialect_probe.rs` asserts that at compile
time, and if it ever stops compiling this design is unsound.

`db/ddl.rs` carries the DDL that has no shared spelling, through **named
tokens** (`{{PK}}`, `{{NOW_TEXT}}`) rather than pattern-matching SQL: an
unhandled construct then leaves an unsubstituted token and fails loudly instead
of being mistranslated. The audit-log triggers are written out per dialect,
because PostgreSQL has no `CREATE TRIGGER IF NOT EXISTS` and needs a `plpgsql`
function behind each guard.

Portability is enforced by the `backend-postgres` CI job and essentially
nothing else, because SQLite accepts every violation silently. `AGENTS.md` has
the full table of what not to write; the shape of the problem is that SQLite is
dynamically typed and PostgreSQL is not, so a column whose declared type
disagrees with the Rust type it is read as works locally and fails there.

The costly version of that lesson: three separate rounds of defects came from
blanket type sweeps over the schema — widening every `INTEGER` to `BIGINT` and
every `TIMESTAMP` to `TIMESTAMPTZ` without checking what each column is read as.
`briefing_enabled` and `is_active` became integers that models read as `bool`;
`currency_exponent` is read as `i32` in two modules and `i64` in thirty-eight;
`latitude`, `longitude` and `temperature` were `REAL`, which is `float4`, while
the models read `f64`; and `next_due_at` plus six tables' `created_at`/
`updated_at` became timestamps that models read as `String`. **Change column
types one at a time, against the field that reads them.**

A related trap with no defect in it: `TIMESTAMPTZ` holds microseconds and
`DateTime<Utc>` holds nanoseconds, so a round-tripped instant differs in the
last three digits on PostgreSQL only. `utils::time::now_at_storage_precision`
exists for tests that assert on one.

`accounts::merge::move_rows` replaces `UPDATE OR IGNORE`, which Postgres has no
equivalent of. It tries the bulk update in a savepoint and falls back to moving
rows one at a time only on a unique violation. All-or-nothing would lose data:
if `from` has "food" and "travel" while `into` already has "food", **"travel"
must still transfer**. It deliberately enumerates no constraints — the database
reports the collision — because the comment it replaced named four
unique-constrained tables when the schema has seven.

Identifiers and secrets come from `utils::random`, not the database. That began
as a portability fix for `randomblob` but is the right shape anyway: a session
token's unguessability should not depend on which database was deployed.

Tests run on either. `db::pool::test_pool()` returns in-memory SQLite by
default and, with `TEST_DATABASE_URL` set, a PostgreSQL pool in a schema of its
own — the suite runs in parallel and `search_path` belongs to a connection, not
a statement. CI runs both on every pull request, and the PostgreSQL job is
blocking. Seven tests skip there and print why: they are about SQLite itself —
`db::backfill`'s fixtures build a legacy SQLite database by hand, and the
`accounts`/`db::schema` guards read `sqlite_master` to derive the table list.
See `docs/POSTGRES_MIGRATION.md` for the full history.

### Frontend module mirror

The SPA mirrors the backend plugin shape: `core/registry.ts` holds `FrontendModule`s registered in `App.tsx` (`core`, `todo`, `reminder`). Each module supplies a `MainView`, optional `navTab`/`getStatItems`, and a `handleSseEvent(event, queryClient)` used to invalidate React Query caches from the live stream (`core/useSystemEvents.ts`). Adding a backend module means adding a matching `frontend/src/modules/<name>/index.tsx` and registering it in `App.tsx`.

All API calls go through `core/apiClient.ts` (`fetchApi`), which sends `credentials: 'include'` — the session is an HttpOnly cookie, so there is no token to read or store — and unwraps the standardized `{ "error": "...", "code": 400 }` shape produced by `backend/src/error.rs::ApiError`.

`core/session.ts` is the single source of "who is signed in": the cookie is unreadable by the page, so only the server can answer, and `useSession()` asking `/auth/me` is that answer. There is deliberately no client-side token to inspect and nothing that can drift from the server's view.

`core/security.ts::useSensitiveAction` wraps any step-up-gated call. A 403 is not an error to show someone — it is the server asking for a passkey — so it prompts, retries, and only reports a second failure.
