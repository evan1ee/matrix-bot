# API

The embedded Axum server (`backend/src/interface/http/`) exposes the application's capabilities to the React SPA, native clients, MCP clients and external senders. This document is the contract's conventions: errors, events, authentication, webhooks and the OpenAPI workflow. The list of endpoints is **`backend/openapi.json`** (served at `GET /api-docs/openapi.json`); `README.md` carries a human-readable summary. The layer model behind the handlers is in [`architecture.md`](architecture.md).

## Conventions

- **Mounting.** Application endpoints live under `/api/v1`, aggregated by `api_routes()` in `interface/http/routes.rs`. The health probes, the Telegram update webhook receiver (`POST /api/webhooks/telegram`), and the Telegram-compatible webhook receiver are mounted at the router root, outside `/api/v1`.
- **Handler shape.** Parse and validate → call a service on `ApiState.services` → map the result to a response DTO. A handler holds no business rule and runs no SQL.
- **Errors.** Every response uses `ApiError` (`backend/src/error.rs`): `{ "error": "...", "code": 400 }`. `?` maps `AppError` through `interface/http/app_error.rs`: `Validation` → 400, `NotFound` → 404, `Forbidden` → 403, `Conflict` → 409; `Database` and `External` become an opaque 500 with the detail logged, never returned. `.api_context("Failed to …")?` keeps a handler's log context.
- **Input limits.** Cap every API payload string at 500 characters. The router carries a 180 s `TimeoutLayer` so long research requests (AI, web search) complete.
- **DTOs are not domain types.** DTOs derive `ToSchema` and implement `From<&DomainType>`; domain types derive neither `ToSchema` nor `sqlx::FromRow`. Never put a `///` doc comment on a DTO that had none (utoipa copies it into the schema).
- **Ownership.** The owner comes from the authenticated context (`AuthUser::owner_id()`), never from a path, query or body field. Every query enforces the caller's own scope.
- **Rate limits.** `interface/http/rate_limit.rs` holds in-memory sliding-window limiters (webhook tokens, recovery, auth ceremony starts, geocoding). A new unauthenticated or expensive endpoint gets one.

### Changing the contract

REST endpoints, OpenAPI, Matrix commands, MCP interfaces and the database schema are public contracts. Before changing one: identify its consumers, decide whether the change is breaking, update the source of truth, regenerate derived artifacts, update tests and docs. Do not silently break an existing contract; compatibility is a deliberate decision.

```bash
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # regenerate; commit the file if it changed
npm --prefix frontend run generate-api                                          # regenerate the TypeScript client
```

`generate-openapi` is the only binary an agent may run (see `AGENTS.md`). Wire formats shared with the inference crate are JSON fixtures in `contracts/` that both sides assert against. A new sensitive route needs `VerifiedUser` **and** an entry in `SENSITIVE` (see *Step-up*).

## Health probes

`/health/live` (always `200`, touches nothing) and `/health/ready` (`200` when `SELECT 1` succeeds *and* the last recorded sync is within `MATRIX_SYNC_MAX_AGE` — 120 s — of now, else `503` with a JSON reason) are mounted at the router root in `interface/http/mod.rs`, not under `/api/v1` and not behind auth: they are infrastructure surface a load balancer or orchestrator probes, not application API. They answer different questions on purpose. Liveness drives container restarts, which cannot fix a downstream outage (a lost database, a homeserver that stopped answering sync) and would only thrash the process; readiness drives traffic routing. The former `/api/v1/health` is retired; the break was intentional.

## Event flow (SSE)

`GET /api/v1/events` streams `SystemEvent`s. Every state mutation, from every interface (Matrix, REST, MCP, AI tools), must publish one **inside the transaction of its state change** (`repositories::outbox::publish`, see [`architecture.md`](architecture.md#events-and-the-outbox)). `SystemEvent` is serialised `#[serde(tag = "type", content = "payload")]` and sent as an **unnamed** SSE message, so the frontend reads `EventSource.onmessage` and dispatches on `type`, the variant name verbatim (`TodoCreated`, not snake_case).

The stream is fed by the outbox: one `LISTEN outbox` per process pushes each committed event to that instance's SSE hub, which sends it only to subscriptions of the owning account (`SystemEvent::owner_id()`, the `outbox_events.account_id` column). Hubs are per instance; there is no cross-instance fan-out beyond every instance listening. **Every variant must return a meaningful `owner_id()`**; deletion variants carry an explicit `owner_id` field because their row is already gone. A mutation with no event silently breaks the live UI; a variant with no owner cannot be stored. The stream ends on the process shutdown token so open tabs never hold the drain open.

Reconnects. If the server's listener connection drops, the hub replays recent events of connected accounts when it returns (a duplicate only re-invalidates a client cache). A subscriber that cannot keep up is dropped and the browser's own `EventSource` reconnect applies, unchanged: the client refetches its state and the stream carries no history of its own.

## Authentication

### Passkeys and sessions

Passkeys are the only way to establish a session in the browser. There is still no password and no JWT; `bcrypt` survives only to hash recovery codes. A native client (Element X iOS) exchanges a Matrix session for a backend session token and carries it as `Authorization: Bearer` instead of a cookie, and an MCP client authenticates the same way with an `mb_mcp_`-prefixed personal access token — see `README.md` for both. An MCP token never satisfies the step-up gate below: it is a machine credential, so its creation time is never read as a passkey assertion.

`interface/http/auth/webauthn.rs` runs two ceremonies. Sign-in is **discoverable**: the credential carries the account handle, so there is no username field anywhere and no user enumeration to defend against. `AccountId::to_handle` and `AccountId::from_handle` map an account id to the WebAuthn user handle and back: the account id *is* a UUID (rendered `acct_<32 hex>`), so the handle needs no column of its own.

`register/start` mints the account id but writes **no row**; the id has to travel inside the challenge, and an abandoned ceremony must leave nothing behind. Hence `create_with_id` alongside `create`.

Sessions are held server-side (`infrastructure/postgres/repositories/accounts/sessions.rs`). Only the SHA-256 of the cookie is stored, so a leaked backup cannot be replayed. Both timeouts are checked on read rather than swept, so expiry never waits on a background job.

`interface/http/auth/extractor.rs` reads whichever credential the request carries and enforces CSRF twice: `SameSite=Strict`, plus an `Origin` check on every state-changing request that arrived on the ambient session cookie. A missing `Origin` on such a mutating request is **refused** — browsers always send one on `fetch`. A request carrying an explicit `Authorization: Bearer` credential is exempt from the Origin requirement, because a hostile site's ambient authority can never attach a header it does not know the value of.

`VerifiedUser` is the step-up boundary: it wraps `AuthUser` and rejects a session whose last passkey assertion is older than `STEP_UP_WINDOW_MINUTES` (5), or that came from account recovery and is still restricted. Putting it in a handler signature is the whole opt-in. **`interface/http/mod.rs::step_up_tests` drives the real router** (via the extracted `api_routes()`) and fails if a route in its own hand-written `SENSITIVE` list accepts a stale session — but `SENSITIVE` is a literal `const` of `(method, path)` pairs, not something derived from the router, so it does **not** discover a new route on its own. Adding `VerifiedUser` to a handler is necessary but not sufficient: the route must also be appended to `SENSITIVE`, or the test simply never exercises it and a missing `VerifiedUser` ships silently.

`WEBAUTHN_RP_ID` is a bare hostname and ignores ports; `WEBAUTHN_ORIGIN` carries the port and is what the `Origin` header is compared against. Browsing the Vite dev server on `:5173` while `WEBAUTHN_ORIGIN` says `:8080` makes every write fail with "This request did not come from this site". `Config::validate` refuses to boot production on the localhost defaults, because passkeys minted against `localhost` are ones no real browser would return.

### Linking, recovery, and the Matrix ceiling

Linking is chat-initiated for every channel (`!link` on Matrix, `/link` on Discord and Telegram, in a direct conversation, mints a ten-minute single-use token; see `application/identity/link.rs` and the flow in [`architecture.md`](architecture.md#identity-and-linking)). Proof of identity ownership is the direct conversation that minted the token, proof of account ownership is the passkey session that claims it. The web half:

| Route | Extractor | Notes |
|---|---|---|
| `GET /account/identities` | `AuthUser` | every channel's identities of the caller |
| `POST /account/identities/unlink` | `VerifiedUser`, in `SENSITIVE` | `{channel, user_id}`; `409` when it is the last way in |
| `POST /link/inspect` | optional session | read-only preview: `bind`, `already_yours`, `needs_merge`, `owned_elsewhere`, `signup_allowed`, `sign_in_required`, `expired`; an unknown or dead token is `expired` |
| `POST /link/claim` | `VerifiedUser`, in `SENSITIVE` | binds (or merges, then binds), burns the token, publishes `IdentityLinked` |
| `POST /link/signup/start` / `finish` | none, rate-limited per IP (20/min, `429`) | only for an identity on the deployment's signup allowlist (`DISCORD_ALLOWED_USERS`, `TELEGRAM_ALLOWED_USERS`, `EMAIL_ALLOWED_SENDERS`; `403` otherwise, re-checked at finish). `start` returns a `ceremony_id`; `finish` takes `{ceremony_id, token, credential, label?}`, and the ceremony carries the token's SHA-256 so a challenge started for one token cannot finish another. Finish creates the account, stores the passkey and binds the identity in **one** transaction (anything but a completed bind rolls the account back: `400` expired or already yours, `409` owned elsewhere), then issues the session cookie |

The token travels in the request body, is capped at 500 characters, and appears in no log line or error; the landing page strips it from the address bar after reading it. `link` is handled by the inbound pipeline before the sender is resolved: resolving first would provision an empty account for the very identity being claimed. A claim requires `VerifiedUser` (so the web's passkey step-up and the `Origin` check apply); `/link/inspect` takes an optional session and reads only. The old `/account/matrix/*` endpoints (web-minted link code, pending merge confirmation, Matrix-only unlink) are gone, together with `!link <code>`: the replacement is the table above, and an identity is removed with `POST /account/identities/unlink`.

### Telegram update webhook (`POST /api/webhooks/telegram`)

Mounted outside `/api/v1` at `POST /api/webhooks/telegram`. Receives raw JSON `Update` payloads from Telegram Bot API servers.

- **Authentication & Rate Limit:** Checked before reading the request body. Rate-limited by IP sliding window (120 req / 60 s, `429`). The `X-Telegram-Bot-Api-Secret-Token` header is verified against `TELEGRAM_WEBHOOK_SECRET` using constant-time comparison (`utils::secure::constant_time_eq`); missing or incorrect secret returns `401 Unauthorized` without reading the body (invariant 7).
- **Queueing:** Returns `200 OK` immediately upon enqueuing to the adapter's bounded worker channel (capacity 32). Returns `503 Service Unavailable` if the queue is full.
- **Availability:** Returns `404 Not Found` when Telegram is disabled or configured in polling mode.
- **Security:** Neither the secret header nor the payload body is logged at any level. Not included in OpenAPI (`backend/openapi.json` is byte-identical, T12).

The drop-in Bot-API emulation shim (`/bot<token>/sendMessage`) lives in `interface/http/webhooks/telegram_compat.rs` and remains at the router root for external monitoring services.

Unlinking keeps every row. Data belongs to the account; unlinking revokes an access path, and the MXID comes back as a stranger.

Recovery has two independent routes. Codes sign you straight in. `!recover` is fenced because it would otherwise close a circle — linking needs a recent passkey, and passkey recovery would need only the Matrix account: the request is announced immediately, does nothing for 24h, any live session can cancel it, and it then grants a session restricted for 72h. Cancelling deliberately takes `AuthUser`, not `VerifiedUser` — demanding a passkey to stop a recovery you did not ask for is backwards.

`interface/chat/account.rs::WEB_ONLY` is the capability ceiling. Matrix cannot prompt for a passkey, so it gets **no** route to a privileged action. `!export`, `!sessions`, `!recoverycodes` and `!unlink` are `WEB_ONLY` entries and answer with a pointer to the web app; `!passkey rm` gives the same pointer. That refusal is what makes the step-up rule a guarantee rather than a decoration.

Matrix keeps exactly three account capabilities: `!link` (mints a link token; the claim itself is web-only), `!recover`, and `!passkey add` (alias `!web`), plus the read-only `!passkeys` list. `!passkey add` mints a single-use enrollment link that `/auth/passkey/enroll/start` redeems with no session, and finishing that ceremony opens a full, freshly verified session. So it enrolls only an account's **first** passkey. For an account with none (unclaimed, or one whose owner deleted the last passkey while Matrix was linked), the Matrix identity is already the only way in. Once a passkey exists, a link would let the Matrix account alone add its own passkey and pass `VerifiedUser` at once, which is exactly the circle the `!recover` delay closes. The rule is enforced twice: the bot refuses to mint the link, and `EnrollmentRepository::validate_and_consume` refuses a token for an account that has since gained a passkey, because redemption is where the capability is actually granted. `!unlink` sits behind the wall for the same reason: revoking every session, or cutting the other linked identities a `!recover` request is announced to, would let the Matrix account alone disarm the fence's warning channels.

Two of those are forward-looking rather than descriptive: **there is no financial export feature yet**, and `SecurityEventKind::FinancialDataExported` is constructed nowhere. The command name and the event variant are reserved so that whoever builds export puts it behind `VerifiedUser` on the web side rather than discovering the requirement afterwards. Do not read them as evidence the feature exists.

`interface/channels/matrix/notify.rs` records a `security_events` row and DMs every linked Matrix identity through the registry, opening a DM room if none exists. Neither half can fail the action it describes — by the time it runs, the thing has happened.

## Native clients (Matrix session exchange)

A native client that is already signed in to Matrix (for example a SwiftUI tab inside Element X iOS) never shows a login form and never needs a passkey. It exchanges its homeserver session for a backend session:

1. The client posts its existing credentials to `POST /api/v1/auth/matrix/login`: `{ "access_token": "syt_…", "homeserver": "https://…" }`.
2. The backend calls the homeserver's `/_matrix/client/v3/account/whoami` to prove the token belongs to that MXID, then resolves or provisions the account (`IdentityService::resolve` with the Matrix identity; Matrix auto-provisions).
3. The response carries an opaque backend token, `{ "session_token": "<opaque token>", "account_id": "acct_…", "display_name": "…", "mxid": "@…" }`. Every later request sends `Authorization: Bearer <session_token>`; the SSE stream authenticates the same way.

Bearer requests are exempt from the browser `Origin` check (see *CSRF*). Push notifications need no custom APNs: the bot's reminders, bill alerts and price alerts are ordinary Matrix messages to the user's bot DM, which the Matrix client already delivers. The Swift client and view code that used to live in `docs/` is in git history (`docs/ELEMENT_X_IOS_INTEGRATION.md` at `c728bd5`).

## Inbound webhook gateway

External senders (Alertmanager, Grafana, Uptime Kuma, Sentry, cron, CI) post into a Matrix room through a per-webhook secret token.

| Endpoint | Auth | Purpose |
|---|---|---|
| `POST /api/v1/webhooks/:token` | the token in the path | JSON `{message\|text, title?, level\|severity?, room_id?, format?}` or a raw plaintext body; `level` is `critical`, `warning`, `info` (default) or `success` |
| `GET`/`POST /bot<token>/sendMessage` | the token replaces the Telegram bot token | drop-in Telegram Bot API emulation (`text`, `parse_mode`), mounted at the router root (implemented in `webhooks/telegram_compat.rs`) |
| `GET/POST/PATCH/DELETE /api/v1/account/webhooks[/:id]`, `…/:id/{rotate,test,logs}` | session | management; rotation and deletion require step-up |

Management is also available from chat: `!webhook list | create <name> [room] | test <id> | revoke <id>`. Design rules:

- The raw token (`whk_<hex>`, CSPRNG) is shown once; only its SHA-256 is stored.
- A request returns `202 Accepted` once its alert is written to the outbox (`WebhookTriggered` plus one `matrix:<room>` delivery, committed together with the webhook's `last_triggered_at`). The events runtime delivers it: up to 10 attempts, waiting `retry_delay` after each failure (1 s, doubling, capped at 512 s), each send bounded to 30 s. `failure_count` rises on every failed attempt and `total_delivered_count` on delivery, each only when the delivery row actually changed state. `GET /account/webhooks/:id/logs` reads the outbox deliveries, so history is kept 30 days; the `outbox_id` in the accepted response and the `id` of each log entry are delivery ids. The `WebhookTriggered` SSE payload carries `id`, `owner_id`, `room_id`, `severity`, `title`, `html_body` and `plain_body`, and no outbox id. See [`architecture.md`](architecture.md#events-and-the-outbox).
- A sliding-window limiter allows 60 requests per minute per token.

## Knowledge Base & Notes API

Personal notes and semantic search endpoints are mounted under `/api/v1`:

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/v1/notes` | Session / Bearer | List `NoteSummary` items (metadata and an `excerpt`; never `content`) with optional `?archived=bool`, `?tag=string`, `?search=text` (case-insensitive substring of title or body, literal, at most 200 characters, 400 beyond), `?limit=n` |
| `POST` | `/api/v1/notes` | Session / Bearer | Create note (`CreateNoteInput: { title, content, note_type?, tags?, format? }`); `format` is accepted and ignored, an unknown `note_type` is a 400. Returns `Note` |
| `GET` | `/api/v1/notes/tags` | Session / Bearer | List all unique tags used across notes |
| `GET` | `/api/v1/notes/:id` | Session / Bearer | Fetch one `Note`: the summary fields plus `content` (Markdown) and `format` (always `"markdown"`) |
| `PUT` | `/api/v1/notes/:id` | Session / Bearer | Update note (`UpdateNoteInput: { title?, content?, note_type?, tags?, is_archived?, expected_content_hash? }`). When `expected_content_hash` (the `content_hash` the editor loaded) no longer matches the stored body the update is refused with 409 `{"error":"note changed since it was loaded","code":409}`; omit it for last-writer-wins. A malformed hash is a 400 |
| `GET` | `/api/v1/notes/export` | Verified session (step-up) | Download every note, archived included, as `notes-<date>.zip` holding `notes/<slug>-<id>.md` files: YAML frontmatter (`id`, `type`, `title`, `tags`, `source`, an optional `source_ref`, `archived`, `created_at`, `updated_at`), a blank line, then the Markdown byte for byte. At most 10,000 notes (400 beyond) |
| `POST` | `/api/v1/notes/import` | Verified session (step-up) | Multipart upload (field `file`) of a zip of `*.md` files, returning `{ created, updated, skipped: [{file, reason}] }`. Frontmatter is optional; a title falls back to the first `# ` heading, then the file name. A file's `archived` and `source_ref` are restored when present (an import is always captured on the `web` channel). A file whose `id` is one of the caller's notes updates it (reported as skipped `unchanged` when nothing differs); any other id creates a new note, so an id is never trusted across accounts. Each note is written in its own transaction through the normal write path (event and embedding job included), and a file that fails validation is listed in `skipped` without stopping the rest. Caps, each a 400 naming the limit: zip 20 MB (the route lifts the default body limit), 5,000 entries, 1 MB per entry and 100 MB unpacked, both measured from the bytes actually decompressed. Entry names are labels only and never reach the file system |
| `DELETE` | `/api/v1/notes/:id` | Session / Bearer | Permanently delete note (204 No Content) |
| `POST` | `/api/v1/knowledge/search` | Session / Bearer | Hybrid search (`KnowledgeSearchRequest: { query, filter_types?, limit?, include_archived? }`; `query` at most 500 characters, 400 beyond) returning `Vec<SearchResultItem>`. The `X-Search-Mode` response header is `hybrid`, or `lexical` when the embedder was unavailable and only keyword results are returned; the body stays a bare array |

Request bodies are limited to 1 MB for extractors (`DefaultBodyLimit`); `POST /notes/import` raises that to 20 MB on its own route, and a 20 MB `RequestBodyLimitLayer` is the hard backstop for handlers that read the body themselves.

Mutations publish `NoteCreated`, `NoteUpdated`, or `NoteDeleted` outbox events with valid `owner_id()`. Background durable jobs (`knowledge.embed`) chunk and embed note content asynchronously for vector search.

## Admin Configuration & Secrets Vault API

Administrative configuration and encrypted secret management endpoints are mounted under `/api/v1/admin`. Ordinary read endpoints use `AdminSession` (requiring an authenticated cookie session, `role == 'admin'`, at least one enrolled passkey, non-MCP token, and non-recovery-restricted session). Write and secret-testing endpoints use `AdminUser` (`AdminSession` plus fresh WebAuthn passkey step-up within 5 minutes, `STEP_UP_WINDOW_MINUTES = 5`, enforced in `SENSITIVE`):

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/v1/admin/config-status` | `AdminSession` | System configuration overview (`settings_count`, `custom_settings_count`, `encryption_configured`, `secrets_count`, `active_secrets_count`, `last_reload_at`) |
| `GET` | `/api/v1/admin/settings` | `AdminSession` | List all 20 hot-reloadable `SettingItem`s with current value, default, type, category, `is_custom`, and `hot_reloadable` flag |
| `GET` | `/api/v1/admin/settings/:key` | `AdminSession` | Fetch a single `SettingItem` by key |
| `PUT` | `/api/v1/admin/settings/:key` | `AdminUser` (5m step-up) | Update a setting (`{ value }`), validate schema/bounds, persist to `app_settings`, record `config_audit_log`, publish `ConfigUpdated`, and refresh `RuntimeConfigSnapshot` |
| `DELETE` | `/api/v1/admin/settings/:key` | `AdminUser` (5m step-up) | Reset a setting to its built-in default (`ResetSettingResponse: { status: "reset", key }`) |
| `GET` | `/api/v1/admin/secrets` | `AdminSession` | List `SecretMetadataDto` items (`id`, `provider`, `name`, `status`, `key_version`, `masked_preview`, `last_tested_at`, `last_test_status`, `last_test_error`, `created_at`, `updated_at`). Plaintext and ciphertext are never returned |
| `POST` | `/api/v1/admin/secrets` | `AdminUser` (5m step-up) | Encrypt and store a new credential (`CreateSecretRequest: { provider, name, secret, metadata? }`) using AES-256-GCM with AAD `{provider}:{name}:{key_version}` |
| `PUT` | `/api/v1/admin/secrets/:id` | `AdminUser` (5m step-up) | Replace an existing credential's secret payload (`ReplaceSecretRequest: { secret }`) |
| `DELETE` | `/api/v1/admin/secrets/:id` | `AdminUser` (5m step-up) | Delete an encrypted credential from the vault (`DeleteSecretResponse: { status: "deleted", id }`) |
| `POST` | `/api/v1/admin/secrets/:id/test` | `AdminUser` (5m step-up) | Decrypt credential in memory and run live provider connectivity check (`SecretTestResultDto: { id, status, error }`) |
| `GET` | `/api/v1/admin/audit-logs` | `AdminSession` | Paginated `AuditLogPageDto` (`{ entries: Vec<ConfigAuditRowDto>, total, limit, offset }`, `?limit=50&offset=0`) |

`GET /api/v1/auth/me` includes `role: "user" | "admin"` so the SPA can conditionally display the Admin Console navigation item. Administrator role bootstrap and recovery are CLI/SQL-only (`matrix-bot admin grant <account_id>` / `matrix-bot admin revoke <account_id>`).

## Testing passkey auth locally

Everything runs on `localhost`, without HTTPS or a hardware key.

```bash
# .env
WEBAUTHN_RP_ID=localhost
WEBAUTHN_ORIGIN=http://localhost:5173     # the port you actually browse on

cargo run                                  # bot + API on :8080 (a human runs this, never an agent)
npm --prefix frontend run dev              # SPA on :5173, proxies /api and /health to :8080
```

`WEBAUTHN_ORIGIN` must be the browsing origin: a mismatch fails every write with `This request did not come from this site`. `WEBAUTHN_RP_ID` ignores ports and cookies ignore ports, so `localhost` covers both servers.

**Virtual authenticator.** In Chrome/Edge: DevTools → More tools → WebAuthn → *Enable virtual authenticator environment* → add an authenticator with protocol **ctap2**, transport **internal**, and both **resident keys** and **user verification** ticked (sign-in is usernameless and registration asks for verification). Deleting a credential in that panel is how you test "I lost my passkey".

| Flow | How to exercise it |
|---|---|
| Register / sign in | Sign-in dialog → *Create an account instead* → save the recovery codes (shown once) → sign out → *Sign in* (one button, no username). Invite-gated registration uses `API_REGISTRATION_SECRET`. |
| Link a chat account | Send `!link` (Matrix) or `/link` (Discord, Telegram) **in a DM** to the bot and open the reply's link while signed in. In a shared room it must refuse and mint nothing. An identity with existing data shows a merge preview before the passkey confirmation; the bot confirms in the chat afterwards. |
| Step-up | Expire it with `psql "$DATABASE_URL" -c "UPDATE sessions SET last_verified_at = NOW() - INTERVAL '10 minutes';"`, then click *Sign out other devices*: the server answers 403, the UI prompts for the passkey, the action retries. |
| Recovery | `!recover` in a DM starts a 24 h delay; skip it with `UPDATE recovery_requests SET valid_after = NOW() - INTERVAL '1 minute';`. Cancelling must work without a passkey. |
| Passkey from chat | `!passkey add` (alias `!web`) in a DM mints a single-use 10-minute link, valid only while the account has no passkey. `!passkeys` lists them read-only. |
| Matrix ceiling | `!export`, `!sessions`, `!recoverycodes`, `!unlink` and `!passkey rm` must refuse and point at the web app. |

Reset a dev database with `DROP SCHEMA public CASCADE; CREATE SCHEMA public;` (migrations re-apply on boot) and delete the virtual authenticator too; credentials pointing at a vanished account fail sign-in confusingly.

**Production.** Set `WEBAUTHN_RP_ID=bot.example.com`, `WEBAUTHN_ORIGIN=https://bot.example.com` and `ENVIRONMENT=production`. `Config::validate` refuses to boot production on the localhost defaults. Passkeys registered locally never work in production: the RP ID is part of the credential.
