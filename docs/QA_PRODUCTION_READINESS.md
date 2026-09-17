# QA Review — Production Readiness

**Repo:** `matrix-hello-bot` · **Branch:** `visualizing-life` · **HEAD:** `80953c2`
**Date:** 2026-09-05 · **Scope:** full codebase (Rust backend + React SPA + Docker/CI)

---

## Verdict

**Not ready to ship to a public deployment.** Three blockers, all of which are
reachable by someone who is not a user of the system or who is a user of a
*different* account. The engineering underneath is genuinely good — the auth
design, the ownership model, and the test discipline are better than most
projects of this size — but three doors were left open and one advertised
safety net (account recovery) is not connected to anything.

For a **single-user deployment on a private homeserver**, items P0-2 and P0-3
drop to low risk and P0-1 becomes the only thing that must be fixed before you
rely on it.

### Verification run

| Check | Result |
|---|---|
| `cargo check --all-targets` | ✅ clean, zero warnings |
| `cargo test` | ✅ **441 passed**, 0 failed, 4 ignored |
| `npx tsc --noEmit` (frontend) | ✅ clean |
| `cargo run --bin generate-openapi` + diff | ✅ `openapi.json` in sync with the code |
| `npx playwright test` (150 specs) | ⚠️ run 1: 149/150 · run 2: **150/150** — one flaky spec (see L-1) |
| Production file-size rule (<500 lines above `#[cfg(test)]`) | ✅ largest is 493 (`modules/portfolio/handler.rs`) |
| `SystemEvent::owner_id()` coverage | ✅ every variant returns a real owner |
| Dynamic SQL (`format!` into a query) | ✅ interpolates constants only — no injection path |
| Secrets in log output | ✅ none found |
| `dangerouslySetInnerHTML` / `console.log` in SPA | ✅ zero of each |

---

## P0 — Blockers

### P0-1 · Account recovery cannot be completed. A lost passkey is a permanent lockout.

**Files:** `backend/src/accounts/recovery.rs:120`, `backend/src/modules/account/mod.rs:181`, `backend/src/api/mod.rs:70-72`

Passkeys are the only way in — no password, no bearer token. Recovery is
therefore the whole safety net, and **neither half of it is reachable**:

- `RecoveryRepository::redeem` (redeem a recovery code and sign in) — called
  from **tests only**. No route, no handler, no UI.
- `modules::account::redeem_matured_recovery` (redeem the matured code from the
  24-hour `!recover` flow) — called from **tests only**. Same.

The router exposes exactly three recovery endpoints: `POST /account/recovery-codes`
(issue), `GET /account/recovery-status`, `POST /account/recovery/cancel`. There
is no redeem endpoint, and `grep -ri recovery frontend/src` finds no redemption
UI.

Everything up to the redemption works and looks convincing: codes are generated
and shown once, `!recover` DMs a code, the 24h clock runs, security events are
recorded, the SPA shows the pending-recovery alarm. A user will reasonably
believe they are covered.

**Failure scenario:** user registers, saves their ten recovery codes, later loses
their laptop and their phone. They type a recovery code into… nothing. They send
`!recover` from Matrix, wait 24 hours, receive a code, and have nowhere to enter
it. The account and all its data are unreachable, permanently, with no operator
workaround short of hand-editing SQLite.

**Fix:** add `POST /api/v1/auth/recovery/redeem` (unauthenticated, body carries
the code) that calls `RecoveryRepository::redeem`, and a second route for the
matured `!recover` code calling `redeem_matured_recovery`; both then open a
session via the same path `login_finish` uses, with `restricted_until` set per
`RECOVERY_RESTRICTION_HOURS`. Add the code-entry affordance to `AuthModal`. This
endpoint **must** be rate-limited (see P1-4) — it is the one place in the system
where a guessable secret is accepted.

---

### P0-2 · Server-side request forgery in the bookmark fetcher, reachable without an account

**Files:** `backend/src/modules/bookmarks/metadata.rs:126-146`, `backend/src/api/bookmarks.rs:75-100`, `backend/src/modules/bookmarks/handler.rs:37`

`fetch_and_extract_metadata` performs a server-side `GET` on a caller-supplied
URL. The only validation is `starts_with("http://") || starts_with("https://")`
and a 2048-char length cap. There is no host allowlist, no private-range denial,
no redirect policy, and no response-size cap.

What comes back is not discarded — `og:title`, `og:description`, `author` and up
to 25 paragraphs of `reader_content` are stored on the bookmark and rendered
back to the caller. This is a **readable** SSRF, not a blind one.

It is reachable two ways, and the second is the serious one:

1. `POST /api/v1/bookmarks` — any authenticated account.
2. `!save http://…` over Matrix — **any Matrix user anywhere**, because the bot
   auto-joins every invite (P1-1) and `resolve_or_provision` mints an unclaimed
   account for any unknown MXID. No signup, no link, no approval.

**Failure scenario:** a stranger on any federated homeserver invites the bot to a
room and sends `!save http://169.254.169.254/latest/meta-data/iam/security-credentials/`
— or `http://127.0.0.1:8080/api/v1/health`, or an internal admin panel on the
same Docker network. The bot fetches it and replies with the extracted title and
description; the body text is saved to `reader_content` and readable through the
bookmarks list.

**Fix:** resolve the host before connecting and refuse loopback, link-local,
private (RFC1918), CGNAT, and unique-local ranges plus `.internal`/`.local`;
re-check after every redirect (or set `redirect::Policy::none()`); cap the
response body (`Content-Length` check plus a streaming byte limit — 2 MB is
generous for `<head>` metadata) instead of `resp.text()`. The same guard covers
P1-3.

---

### P0-3 · Webhook payloads can target any room the bot has joined

**Files:** `backend/src/api/webhooks.rs:63-70,145-160,247-252`, `backend/src/modules/webhook/scheduler.rs:35-50`

The inbound webhook body's `room_id` (or Telegram-shim `chat_id`) overrides the
webhook's configured room:

```rust
let target_room = room_override.unwrap_or_else(|| webhook.default_room_id.clone());
```

Nothing checks that room against the webhook's owner. The outbox worker looks up
the room on the bot's client and sends if the bot is joined — no ownership test
anywhere in the path. `default_room_id` is equally unchecked at
`create_webhook`/`update_webhook`: any non-empty string is accepted.

This is a straight contradiction of the care taken in `api/rooms.rs::list_rooms`,
which goes to real trouble to show a caller only rooms where one of *their*
linked MXIDs is joined. The picker is scoped; the thing the picker feeds is not.

**Failure scenario:** two tenants share an instance. Tenant A creates a webhook,
then POSTs `{"message":"...","room_id":"!tenantB-private-room:hs"}` — or simply
sets `default_room_id` to tenant B's room in the UI. The bot, which is joined to
both, delivers A's content into B's private room. With P1-1 in play, an attacker
who can get the bot into a room can also aim their own webhook at any other room
the bot occupies.

**Fix:** validate the target room on write and on send — it must be one where a
linked MXID of `webhook.owner_id` is joined, reusing the membership predicate
`list_rooms` already implements. Reject an unknown room at create/update time
with a 400, and drop (log, mark failed) an outbox item whose room fails the check
at delivery.

---

## P1 — High

### P1-1 · The bot joins any room it is invited to

**File:** `backend/src/bot/handler.rs:24-32`

```rust
client.add_event_handler(|ev: StrippedRoomMemberEvent, room: Room| async move {
    …
    if let Err(err) = room.join().await { … }
```

No allowlist, no owner check, no invite policy. On a federated homeserver, any
stranger can pull the bot into a room and command it. Every `!` command then
runs against a freshly provisioned unclaimed account: `!save` (P0-2), `!ai` (paid
tokens against the operator's key), `!expense`, `!portfolio`, unbounded row
creation.

This is the multiplier that turns P0-2 from "authenticated SSRF" into
"unauthenticated SSRF", so it is worth fixing even after P0-2 is patched.

**Fix:** join only invites from `MATRIX_VERIFICATION_ALLOWLIST` (or a new
`MATRIX_INVITE_ALLOWLIST`), or from an MXID already present in `matrix_links`.
Reject and leave otherwise.

### P1-2 · Nothing runs the tests in CI

**File:** `.github/workflows/docker-image.yml`

The only workflow is a **manually triggered** Docker build (`on: workflow_dispatch`).
Nothing runs `cargo check`, `cargo test`, `tsc`, or Playwright on push or PR —
and nothing verifies `openapi.json` is still in sync. AGENTS.md lays out a
four-step verification gate that is entirely dependent on a human remembering it.

The step-up test (`api/mod.rs::step_up_tests`) is explicitly designed as the
thing that catches a sensitive endpoint shipped without `VerifiedUser` — "CI
catches it," says CLAUDE.md. CI does not currently run it.

**Fix:** a `on: [push, pull_request]` workflow running the AGENTS.md four steps
plus `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`.

### P1-3 · Per-user AI base URL is a second SSRF vector; the API key is stored in plaintext

**Files:** `backend/src/modules/ai/models.rs:38-56`, `backend/src/db/schema_ai.rs:6-15`

`api_base_url` is user-supplied and the server POSTs to `{base}/chat/completions`.
Validation blocks only `169.254.*`. `127.0.0.1`, `localhost`, `10.*`,
`192.168.*`, `::1`, `*.internal` and any DNS name that resolves into those are
all accepted. Blocking loopback outright isn't viable (the default is
`http://localhost:11434/v1` for Ollama), so this needs an explicit opt-in rather
than a blanket deny.

Separately, `ai_user_configs.api_key` is stored **in cleartext**. Every other
secret in this codebase is hashed or salted deliberately — session cookies
(SHA-256), recovery codes (bcrypt), webhook tokens (hashed) — with `session.rs`
stating outright that "a leaked backup cannot be replayed." A database backup
leaks every user's provider API key.

**Fix:** reuse the P0-2 host guard, with an `AI_ALLOW_PRIVATE_BASE_URL` escape
hatch for self-hosted Ollama. Encrypt `api_key` at rest with a key from the
environment, or document plainly that the DB file is secret-bearing.

### P1-4 · No rate limiting outside the webhook path

**Searched:** rate limiting exists in `api/webhooks.rs` only.

Unlimited: passkey `register/start` + `login/start` (challenge minting and
in-memory ceremony storage), the recovery-redeem endpoint once P0-1 lands,
`POST /ai/chat` (each call is up to 3 tool-loop round-trips to a paid provider),
`GET /footprints/geocode` (an open relay in front of Nominatim, whose usage
policy caps you at 1 req/s and will block the IP), and
`POST /portfolio/refresh-prices` (one outbound Yahoo request per holding, per
click).

The webhook limiter itself is in-process (`OnceLock<Mutex<HashMap>>`), so it
resets on restart and would not hold across replicas.

**Fix:** a shared IP/account limiter layer (`tower_governor`) on the auth,
recovery, AI and outbound-proxy routes.

### P1-5 · `docker stop` is not a graceful shutdown

**File:** `backend/src/main.rs:120`

Shutdown is wired to `tokio::signal::ctrl_c()` — SIGINT only. Docker, Kubernetes
and systemd all send **SIGTERM**, which is unhandled, so the process is killed
outright after the grace period: in-flight HTTP requests are cut, the Matrix sync
loop does not stop cleanly, and the SQLite WAL is left to recover on next boot.

The careful `select!`-based shutdown plumbing in `run_bot_loop` — including the
comment explaining why an earlier version couldn't be interrupted — never fires
in a container.

**Fix:**

```rust
let mut term = tokio::signal::unix::signal(SignalKind::terminate())?;
tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} };
```

---

## P2 — Medium

**P2-1 · Recovery redemption is O(every unused code in the table) bcrypt.**
`accounts/recovery.rs:126-137` selects *all* unused hashes across all accounts and
bcrypt-verifies each. The design rationale (no index that could leak which codes
exist) is sound, but at cost 8 × 10 codes × N accounts, a single redemption
attempt is measurable CPU, and once P0-1 exposes it over HTTP it becomes a cheap
amplification DoS. Pair the endpoint with a strict rate limit, and consider a
short unencrypted prefix column to narrow the candidate set.

**P2-2 · Unbounded response body in the metadata fetcher.**
`metadata.rs:143` does `resp.text().await` with no size cap, then
`parse_html_metadata` calls `html.to_lowercase()` — a second full copy. A
multi-hundred-MB response (or an endless stream inside the 5s window) is an OOM
on a small container. Covered by the P0-2 fix.

**P2-3 · Mutex poisoning takes the webhook limiter down permanently.**
`api/webhooks.rs:28` — `limiter.lock().unwrap()`. Any panic while the lock is held
poisons it and every subsequent inbound webhook 500s until restart. Use
`lock().unwrap_or_else(|e| e.into_inner())`.

**P2-4 · Geocode proxy builds a fresh `reqwest::Client` per request.**
`api/footprints.rs:227-231` — no connection pooling, a new TLS handshake per
lookup, on a typeahead-shaped endpoint. `state.http_client` already exists and is
passed everywhere else. Also see P1-4 on Nominatim's rate policy.

**P2-5 · Inconsistent input-length validation.**
AGENTS.md mandates a 500-char cap on API payload strings. `update_bookmark`
enforces it (`api/bookmarks.rs:163,168`); `create_bookmark` does not check
`title` or the joined `tags` at all. `api/life.rs::upsert_day` caps `note` but
serialises `activities` to JSON with no bound. `portfolio_alerts.rs`,
`portfolio_prices.rs` and `settings.rs` rely on their normalisers, which is fine
— but the gaps above are real DB-bloat vectors.

**P2-6 · SSE drops events silently when a client falls behind.**
The broadcast channel is capacity 100 (`events.rs:156`) and `sse.rs:23` turns a
`Lagged` error into `None` via `item.ok()?`. A burst larger than the buffer
leaves the SPA's React Query caches stale with no resync signal and no error. At
minimum, emit a `Resync` event on lag so the client can invalidate everything.

**P2-7 · Portfolio buy/sell writes two tables without a transaction.**
`PortfolioRepository::buy` debits cash and writes the holding as separate
statements. The reasoning in CLAUDE.md (no repository uses `pool.begin()`, so
this follows precedent) is honest, but this is the one place in the codebase
where the two writes are a *financial* pair: a crash between them loses the cash
or the shares. Precedent is a weaker argument than money. Wrap these two in a
transaction even if nothing else in the codebase uses one.

**P2-8 · Price feed scrapes Yahoo Finance behind a spoofed browser User-Agent.**
`modules/portfolio/price_feed.rs:13-14` sends a full Chrome UA string to an
undocumented endpoint. This is a ToS and reliability liability, not a bug — the
feed will break without warning. Worth naming as a known operational risk, with
a licensed provider as the long-term answer.

---

## L — Low / hygiene

- **L-1 · One flaky e2e spec.** `e2e/finance.spec.ts:275` ("shows a server
  rejection inside the row") failed on the first full 150-spec parallel run,
  then passed both in isolation and on a clean second full run — a timing flake
  under parallel load, not a defect. Local config has `retries: 0`; CI has
  `retries: 2`, so it would surface as flaky rather than red. Worth an explicit
  wait on the mutation settling rather than leaning on retries.
- **L-2 · Windows-path junk directories in the repo.** `frontend/C:/` and
  ``frontend/C:\Users\mbmon\.gemini\antigravity\brain\…/`` (228 KB of PNGs)
  are untracked, unignored, and will be picked up by the next `git add -A`.
  Delete them.
- **L-3 · `CLAUDE.md` says 377 tests; there are 441.** Small, but it is the
  number a reviewer checks against.
- **L-4 · Duplicate route registrations.** `/webhooks/:token`,
  `/bot:token/sendMessage` and `/api-docs/openapi.json` are registered both
  inside `api_routes()` (so, under `/api/v1`) and again at the router root
  (`api/mod.rs:344-350`). Harmless today, but it means two live paths per
  endpoint and only one of them documented.
- **L-5 · Two `Cargo.lock` files.** Root and `backend/`. The workspace uses the
  root one; `backend/Cargo.lock` is dead weight that will drift.
- **L-6 · `/health` always returns 200.** It reports `db_status: "disconnected"`
  in the body when SQLite is unreachable but still answers 200, so an orchestrator
  probe treats a broken instance as healthy. Return 503 when the DB check fails.
  The Dockerfile also has no `HEALTHCHECK`.
- **L-7 · Registration is open by default.** `check_invite` returns `Ok(())` when
  `API_REGISTRATION_SECRET` is unset. Reasonable for self-hosting, but a public
  deployment that forgets the variable gets open signup silently. Consider
  refusing to boot in production without either the secret or an explicit
  `ALLOW_OPEN_REGISTRATION=true`.
- **L-8 · Non-constant-time secret comparison.** `webauthn.rs:79` compares the
  registration secret with `==`. Low risk (it is not per-user and the endpoint is
  noisy), but `subtle::ConstantTimeEq` is a one-line change.
- **L-9 · Legacy `web_users` table still created on every migration.**
  Passwords are gone; the table survives only for the Phase 1 backfill. Once no
  live database needs backfilling it should go, per the project's own
  "no backward compatibility baggage" rule.
- **L-10 · Modulo bias in the link-code alphabet.** `linking.rs:65` does
  `ALPHABET[b % 31]` over random bytes; 256 is not a multiple of 31, so the first
  eight symbols are marginally more likely. At ~39 bits with a TTL and
  single-use semantics this is not exploitable — noted for completeness.

---

## What is solid

Worth stating plainly, because it is most of the codebase:

- **The auth design.** Server-side sessions with only the SHA-256 stored,
  HttpOnly + `SameSite=Strict` + a mandatory `Origin` check on every mutation,
  idle and absolute timeouts checked on read rather than swept, discoverable
  passkeys so there is no username and no enumeration surface, and a `VerifiedUser`
  step-up boundary that is **enforced by a test driving the real router**. The
  `register/start`-mints-no-row detail is the kind of thing that is usually got
  wrong.
- **The ownership model.** One `accounts.id` on every table, carried rather than
  derived, with `matrix_links.mxid` as a primary key doing the "one Matrix
  account, one local account" enforcement at the schema level and
  `resolve_or_provision` handling the race by having the loser delete its own
  row. `OWNED_TABLES` shared between merge and backfill so an omission is a data
  bug in one place rather than two.
- **Test discipline.** 441 in-crate tests, 150 e2e specs, and — unusually —
  tests that encode *design invariants* rather than just behaviour: the step-up
  route table, the `migrates_a_real_database` guard, the "internal errors do not
  leak detail" assertion, the ignored live self-verification check.
- **The Matrix device-identity work.** The `delete_stale_device` reasoning in
  `bot/client.rs` and the `ensure_self_verified` retry loop are documented at a
  level that shows they were derived from measured behaviour, not guessed.
- **Error hygiene.** `ApiError::internal` logs the detail and returns a fixed
  string, with a test proving it.
- **No injection surface found.** Every `format!`-built query interpolates
  compile-time constants; all user values are bound.

---

## Recommended gate before deploying

1. Wire up both recovery redemption paths + UI, rate-limited. **(P0-1)**
2. Add the SSRF host guard and response-size cap; apply to bookmarks and the AI
   base URL. **(P0-2, P1-3)**
3. Scope webhook target rooms to the owner, at write and at send. **(P0-3)**
4. Restrict bot invite auto-join to known MXIDs. **(P1-1)**
5. Handle SIGTERM. **(P1-5)**
6. Add a push/PR CI workflow running the AGENTS.md four steps + OpenAPI drift
   check. **(P1-2)**
7. Add rate limiting to auth, recovery, AI chat, geocode and price-refresh. **(P1-4)**

Items 1–4 are the ones that change the security posture. 5–7 are what keep it
from regressing.
