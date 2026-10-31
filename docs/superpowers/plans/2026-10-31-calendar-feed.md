# Calendar Feed (iCalendar) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user mints a private subscription link in Settings and a calendar client (Apple, Google, Outlook, Thunderbird) shows their dated todos, recurring bills and standalone reminders.

**Architecture:** A pure `domain/calendar` module turns plain domain values (`TodoItem`, `RecurringBill`, `Reminder`) into RFC 5545 text. `CalendarService` owns the token lifecycle (only the SHA-256 is stored) and assembles the feed from three owner-scoped repository reads. An unauthenticated `GET /api/v1/calendar/feed.ics` resolves the owner from the token; mint/revoke are ordinary authenticated routes. The Settings modal gets a small panel beside `McpPanel`.

**Tech Stack:** Rust, SQLx, Axum/utoipa, React 19 + React Query. No new crates (`sha2`, `hash_token` and `utils::random::random_hex` exist).

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` (Blueprint 2)

## Global Constraints

- Only `hash_token(token)` (SHA-256 hex) is stored; the plaintext is returned once, on mint.
- Token format `mb_cal_` + `random_hex(32)`.
- Max 500 items per feed. Rate limit 60 requests/minute per client IP.
- Response: `Content-Type: text/calendar; charset=utf-8`, `Cache-Control: private, no-cache`.
- Timestamps `YYYYMMDDTHHMMSSZ`; lines folded at 75 octets with `\r\n ` (never inside a UTF-8 sequence); TEXT escapes `\\`, `\;`, `\,`, `\n`; CRLF line endings.
- UIDs are stable across edits: `todo-{id}@matrix-bot`, `bill-{id}@matrix-bot`, `reminder-{id}@matrix-bot`.
- A reminder with `source_kind` or `source_id` set never produces its own entry.
- Domain code imports only `std`, `chrono`, `uuid`, `serde`, `thiserror` and other `domain` modules; no SQL outside repositories; no file over 500 production lines.

## Rulings made while planning

- Bill UID is `bill-{id}@matrix-bot` (not the spec's `bill-{id}-{due_date}`): the entry carries an `RRULE`, and a UID that changes each time `next_due_at` advances would leave every past cycle behind as a duplicate in the client.
- Table is keyed by `account_id` and listed in `ACCOUNT_ID_DISCARDED` (like `mcp_tokens`), not `OWNED_TABLES` as the spec says: `OWNED_TABLES` entries carry `owner_id`, and a merge should drop the absorbed account's link, not move it.
- Mint/revoke are ordinary `AuthUser` routes, not `VerifiedUser`, and publish no `SystemEvent`: same class as MCP tokens, which do neither.
- `last_accessed_at` is updated in the resolving statement and only when older than one hour, instead of the spec's "asynchronously" (AGENTS §4.3 forbids a detached task).
- Bearer header is accepted as well as `?token=`, per the spec; no new auth extractor, the handler reads both itself.

## Review Focus

- Bad, empty or revoked token on the feed → 404 with an identical body for all three (no oracle); never 200 with an empty calendar.
- A todo or bill name containing `,` `;` `\` newline or a multi-byte character (CJK, emoji) straddling the 75-octet fold → output unfolds back to the exact original text.
- A user with 600 dated items → exactly 500 entries, earliest first, still a valid calendar.
- Completed todos, cancelled/triggered reminders and non-active bills never appear.
- Re-minting a token invalidates the previous URL at once; revoking leaves the feed 404.
- A bill with an unparseable `next_due_at` or unknown frequency is skipped / emitted without `RRULE`, never a 500.

---

### Task 1: Pure iCalendar domain

**Files:**
- Create: `backend/src/domain/calendar/mod.rs`, `backend/src/domain/calendar/ics.rs`, `backend/src/domain/calendar/feed.rs`
- Modify: `backend/src/domain/mod.rs` (add `pub mod calendar;`)

**Interfaces:**
- Consumes: `TodoItem`, `RecurringBill` (`parse_due_at`, `still_recurring`), `Reminder`, `ReminderSource`.
- Produces:
  - `ics::escape_text(&str) -> String`, `ics::fold_line(&str) -> String`, `ics::format_utc(DateTime<Utc>) -> String`
  - `feed::CalendarInput { todos: Vec<TodoItem>, bills: Vec<RecurringBill>, reminders: Vec<Reminder> }`
  - `feed::should_emit_reminder(&Reminder) -> bool`
  - `feed::build_feed(input: &CalendarInput, now: DateTime<Utc>) -> String`
  - `pub const MAX_FEED_ITEMS: usize = 500;`

Mapping `build_feed` must implement: todo → `VTODO` (`DUE`, `SUMMARY`, `PRIORITY` 1/5/9 for high/medium/low, `CATEGORIES` from tags, `VALARM` `TRIGGER:-PT{n}M` when `remind_before_minutes` is set) for incomplete todos with `due_at`; bill → all-day-less `VEVENT` at `parse_due_at(next_due_at)` for `still_recurring` bills, `SUMMARY` "{name}" , `DESCRIPTION` with amount formatted from minor units and `currency_exponent`, `RRULE` from frequency (`daily`→`FREQ=DAILY`, `weekly`→`FREQ=WEEKLY`, `biweekly`→`FREQ=WEEKLY;INTERVAL=2`, `monthly`→`FREQ=MONTHLY`, `quarterly`→`FREQ=MONTHLY;INTERVAL=3`, `yearly`→`FREQ=YEARLY`, else none) plus `;COUNT={total_installments - paid_installments}` when finite and positive; reminder → `VEVENT` (zero length, `VALARM` `TRIGGER:PT0S`) for `Pending` reminders passing `should_emit_reminder`. Every component carries `UID`, `DTSTAMP` (= `now`), `DTSTART`/`DUE`. Entries sorted by their start, truncated to `MAX_FEED_ITEMS`. Calendar header: `VERSION:2.0`, `PRODID:-//matrix-bot//calendar//EN`, `CALSCALE:GREGORIAN`, `X-WR-CALNAME:matrix-bot`.

- [ ] **Step 1: Write failing tests** in the three files' `#[cfg(test)]` modules:
  - `format_utc_is_basic_iso` → `2026-03-04T05:06:07Z` becomes `20260304T050607Z`.
  - `escape_text_escapes_specials` → `a,b;c\d\ne` becomes `a\,b\;c\\d\ne` (newline → literal `\n`).
  - `fold_line_never_exceeds_75_octets_or_splits_utf8` → a 300-char CJK line folds to lines ≤ 75 bytes, each continuation starts with one space, and unfolding (remove `\r\n `) returns the input.
  - `reminder_with_source_is_suppressed` → `should_emit_reminder` false for `source_kind: Some(Todo)`, false for `source_id: Some(1)` alone, true for neither.
  - `todo_becomes_vtodo_with_stable_uid_and_alarm`, `completed_todo_and_undated_todo_omitted`, `monthly_bill_has_rrule_and_stable_uid`, `finite_bill_gets_count`, `unknown_frequency_has_no_rrule`, `bad_due_date_bill_skipped`, `non_pending_reminder_omitted`, `feed_caps_at_500_earliest_first` (600 todos in, 500 `BEGIN:VTODO` out, first is the earliest), `feed_uses_crlf_and_balanced_begin_end`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot --lib domain::calendar` — expect FAIL (module missing).
- [ ] **Step 3: Implement** the three files per the Interfaces and mapping above. `fold_line` counts octets and cuts on char boundaries. Keep `ics.rs` (escaping/folding/format) separate from `feed.rs` (mapping).
- [ ] **Step 4: Run** the same command — expect PASS; then `cargo clippy -p matrix-bot --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat: pure iCalendar feed builder`.

---

### Task 2: Token table and repository

**Files:**
- Create: `backend/migrations/0016_calendar_feed_tokens.sql`, `backend/src/infrastructure/postgres/repositories/calendar.rs`
- Modify: `repositories/mod.rs` (register `calendar`), `backend/src/domain/accounts/mod.rs` (add `"account_calendar_tokens"` to `ACCOUNT_ID_DISCARDED`), `backend/src/db/migrate.rs` (add `("account_calendar_tokens", "account_id")` to the catalog list near line 220), `.sqlx/` cache

**Interfaces:**
- Produces (all `pub async fn` over `impl sqlx::PgExecutor<'_>`):
  - `replace_token(conn, &AccountId, token_hash: &str) -> Result<(), sqlx::Error>` (upsert; resets `created_at`, clears `last_accessed_at`)
  - `delete_token(conn, &AccountId) -> Result<bool, sqlx::Error>`
  - `has_token(conn, &AccountId) -> Result<Option<DateTime<Utc>>, sqlx::Error>` (the `created_at`)
  - `resolve_and_touch(conn, token_hash: &str) -> Result<Option<AccountId>, sqlx::Error>` (one `UPDATE … SET last_accessed_at = now WHERE token_hash = $1 AND (last_accessed_at IS NULL OR last_accessed_at < now - interval '1 hour')` is not enough to return the owner for fresh rows — use a `SELECT` followed by the conditional touch, or a CTE; one round trip preferred)
  - `list_open_dated_todos(conn, &AccountId, limit: i64) -> Vec<TodoItem>` (`completed = false AND due_at IS NOT NULL`, ordered by `due_at`)
  - `list_pending_standalone_reminders(conn, &AccountId, limit) -> Vec<Reminder>` (`status = 'pending' AND source_kind IS NULL AND source_id IS NULL`, ordered by `trigger_at`)
  - `list_active_bills(conn, &AccountId, limit) -> Vec<RecurringBill>` (`lower(status) = 'active'`)
  - Reuse the private row structs/conversions of `todos.rs`, `reminders.rs`, `recurring_bills.rs` by making the needed `…Row` types `pub(super)` rather than copying them.

Schema: `account_calendar_tokens(account_id UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE, token_hash TEXT NOT NULL UNIQUE, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), last_accessed_at TIMESTAMPTZ)` — `UNIQUE` supplies the lookup index, so no separate index.

- [ ] **Step 1: Write failing repository tests** with `test_pool()` + `seed_accounts` (see `repositories/mcp.rs` tests): `replace_token_invalidates_previous_hash`, `resolve_returns_owner_and_none_for_unknown_hash`, `delete_token_returns_false_when_absent`, `touch_is_throttled_to_once_an_hour`, `feed_queries_are_owner_scoped` (alice's rows never returned for bob), `todo_query_excludes_completed_and_undated`, `reminder_query_excludes_sourced_and_non_pending`, `bill_query_excludes_non_active`, `cascade_delete_removes_token_with_account`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot --lib repositories::calendar` — expect FAIL.
- [ ] **Step 3: Implement** migration, repository and the two registry edits. Compile-checked macros only.
- [ ] **Step 4: Regenerate `.sqlx/`** against a migrated scratch DB (`cargo sqlx prepare --workspace -- -p matrix-bot --all-targets`), then run the repository tests plus `cargo test -p matrix-bot --test architecture` and the schema catalog / merge tests (`cargo test -p matrix-bot accounts::merge`) — expect PASS.
- [ ] **Step 5: Commit** `feat: calendar feed token storage and agenda queries`.

---

### Task 3: CalendarService

**Files:**
- Create: `backend/src/application/calendar/mod.rs`
- Modify: `backend/src/application/mod.rs` (declare module, add `pub calendar: CalendarService` to `AppServices`, build it in `AppServices::new`)

**Interfaces:**
- Consumes: Task 1 `build_feed`, `CalendarInput`, `MAX_FEED_ITEMS`; Task 2 repository.
- Produces:
  - `CalendarService::new(pool: PgPool) -> Self` (`Clone`)
  - `mint_token(&self, &AccountId) -> Result<String, AppError>` — plaintext `mb_cal_…`, stored hashed, replaces any existing one
  - `revoke_token(&self, &AccountId) -> Result<bool, AppError>`
  - `token_status(&self, &AccountId) -> Result<Option<DateTime<Utc>>, AppError>`
  - `ics_feed(&self, raw_token: &str) -> Result<Option<String>, AppError>` — `None` for an unknown/revoked token; fetches each list with limit `MAX_FEED_ITEMS`, calls `build_feed(…, Utc::now())`

- [ ] **Step 1: Write failing service tests** (`test_pool`): `mint_then_feed_contains_owner_items_only`, `second_mint_kills_first_token`, `revoked_token_yields_none`, `unknown_token_yields_none`, `sourced_reminder_not_in_feed_but_its_todo_is`, `token_is_stored_hashed` (raw token not present in the table).
- [ ] **Step 2: Run** `cargo test -p matrix-bot --lib application::calendar` — expect FAIL.
- [ ] **Step 3: Implement** the service; no transaction needed (reads and single-row writes), no events.
- [ ] **Step 4: Run** the same tests — expect PASS.
- [ ] **Step 5: Commit** `feat: calendar service`.

---

### Task 4: HTTP endpoints, rate limit, OpenAPI

**Files:**
- Create: `backend/src/interface/http/calendar.rs`
- Modify: `interface/http/mod.rs` (`pub mod calendar;`), `routes.rs` (register), `rate_limit.rs` (add `check_calendar_limit(ip)`: 60 per 60 s, same `OnceLock` pattern), `openapi.rs` (paths and schemas), `backend/openapi.json`, `frontend` generated types

**Interfaces:**
- Consumes: Task 3 service; `extract_client_ip` (as `link.rs` does).
- Produces:
  - `GET /api/v1/calendar/feed.ics?token=` (public; also accepts `Authorization: Bearer`) → 200 `text/calendar; charset=utf-8` + `Cache-Control: private, no-cache`; 404 `ApiError` for a missing/invalid token; 429 over the limit (limit checked before the DB lookup)
  - `POST /api/v1/account/calendar-token` (`AuthUser`) → `CalendarTokenResponse { token: String, feed_path: String }` (`feed_path` = `/api/v1/calendar/feed.ics?token=…`; the SPA builds the absolute and `webcal://` URLs from `window.location`)
  - `GET /api/v1/account/calendar-token` (`AuthUser`) → `CalendarTokenStatus { active: bool, created_at: Option<DateTime<Utc>> }`
  - `DELETE /api/v1/account/calendar-token` (`AuthUser`) → 204

- [ ] **Step 1: Write failing handler tests** (pattern: `finance_analytics.rs` tests): `feed_without_token_is_404`, `feed_with_bad_token_is_404_same_body_as_revoked`, `feed_returns_calendar_headers_and_vcalendar`, `bearer_token_works`, `mint_returns_token_once_and_status_reports_active`, `delete_revokes`, `rate_limit_returns_429_after_60` (use the limiter directly in `rate_limit.rs` tests), plus `POST/GET/DELETE account/calendar-token` appear in the `ORDINARY` list of `step_up_tests` (so they stay ordinary on purpose).
- [ ] **Step 2: Run** `cargo test -p matrix-bot --lib interface::http::calendar` — expect FAIL.
- [ ] **Step 3: Implement** handlers returning `Result<_, ApiError>`; do not log the token; add `#[utoipa::path]` annotations.
- [ ] **Step 4: Regenerate and run** `cargo run --bin generate-openapi`, `npm --prefix frontend run generate-api`, then `cargo test -p matrix-bot --lib interface::http` — expect PASS and a diff limited to the new paths.
- [ ] **Step 5: Commit** `feat: calendar feed endpoints`.

---

### Task 5: Settings panel and e2e

**Files:**
- Create: `frontend/src/components/CalendarFeedPanel.tsx`, `frontend/e2e/calendar-feed.spec.ts`
- Modify: `frontend/src/components/SettingsModal.tsx` (render `<CalendarFeedPanel />` next to `<McpPanel />`), `frontend/e2e/fake-api.ts` (mock the three account routes and `feed.ics`)

**Interfaces:**
- Consumes: `fetchApi` from `core/apiClient`, Task 4 response shapes.
- Produces: `CalendarFeedPanel: React.FC` — status via `useQuery(['calendar-token'])`; "Create link" / "Regenerate" (confirm via `ConfirmModal`, warning that existing subscriptions stop working) / "Revoke"; after minting shows the full `https://…` URL in a read-only field with Copy and a `webcal://` "Subscribe" anchor; the URL is held in component state only and is not shown again after reload (the stored status shows "Link active since {date}" via `formatDateTimeInZone`).

Follow `Design.md` (monochrome, sentence-case, visible labels, focus outlines, `Escape` closes the confirm).

- [ ] **Step 1: Write the failing Playwright spec**: creates a link and sees a `webcal://` anchor and URL containing `mb_cal_`; reload shows "Link active" without the URL; regenerate asks for confirmation and changes the URL; revoke returns to the empty state.
- [ ] **Step 2: Run** `npm --prefix frontend run test:e2e -- calendar-feed` — expect FAIL.
- [ ] **Step 3: Implement** panel, modal wiring and fake-api routes.
- [ ] **Step 4: Run** the spec, then `npm --prefix frontend run build` and the full `test:e2e` — expect PASS.
- [ ] **Step 5: Commit** `feat: calendar feed settings panel`.

---

### Task 6: Docs, handoff, full gates

**Files:**
- Modify: `docs/api.md` (feed + token routes, auth model), `docs/superpowers/HANDOFF.md` (Blueprint 2 done, rulings above, next actions), this plan (tick boxes)

- [ ] **Step 1:** Document the endpoints and the rulings in the owning docs and HANDOFF in the same commit as the last code.
- [ ] **Step 2: Run the §8 gates:** `cargo fmt --all -- --check`; `cargo check -p matrix-bot --all-targets`; `cargo test -p matrix-bot --all-targets` (needs `TEST_DATABASE_URL`); `cargo clippy -p matrix-bot --all-targets -- -D warnings`; `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`; `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets`; `npm --prefix frontend run build`; `git diff --check`. Expected: all green; report actual results.
- [ ] **Step 3: Commit** `docs: record calendar feed`.
