# Adversarial review — 2026-10-11

Ten parallel reviewers, each finding re-checked against the code before listing.
Status column: `?` = awaiting owner verification, `V` = verified, `X` = rejected, `F` = fixed.

## Critical

| # | Issue | Where | Status |
|---|---|---|---|
| C1 | Static-file fallback path traversal: unauthenticated read of any file (`.env`, `/proc/self/environ`, `data/`) | `interface/http/static_files.rs:15,77` | F |
| C2 | Any `AuthUser` (MCP token, bearer, stale or recovery-restricted session) can enroll a passkey and receive a fresh, step-up-verified, unrestricted session; bypasses the 24h `!recover` fence | `interface/http/account/passkeys.rs:24`, `auth/webauthn.rs:222,387`, `repositories/accounts/sessions.rs:48` | F |

## High

| # | Issue | Where | Status |
|---|---|---|---|
| H1 | Matrix access-token exchange opens a session with `last_verified_at = now` → passes `VerifiedUser`/`AdminUser` with no passkey | `interface/http/auth/matrix.rs:249` | F |
| H2 | SSRF: LLM client follows redirects for user-supplied base URL; body snippet returned | `infrastructure/inference/llm/client.rs:41` | F |
| H3 | SSRF: `validate_ai_base_url` accepts `[::ffff:127.0.0.1]`, ULA via DNS, and passes on DNS failure (weaker duplicate of `web/ssrf.rs`) | `domain/llm/mod.rs:88-146` | F |
| H4 | Reminders/todos (HTTP + AI tools) deliver to any raw room id/address (`!victim:hs`, `email:x@y`); fire path never calls `authorize_delivery` | `application/rooms/mod.rs:134`, `domain/rooms/mod.rs:128`, `interface/jobs/reminders.rs:143` | F |
| H5 | Agent scheduled tasks ignore `cron_expression`; every task runs hourly; invalid cron accepted | `application/agent/tasks.rs:105`, `interface/http/agent.rs:408` | F |
| H6 | Life activity `truncate` slices bytes → panics on CJK titles | `read_models/life.rs:29` | F |
| H7 | Price refresh ignores quote currency (GBp ×100, `BTC-EUR` priced in USD); alerts likewise | `application/portfolio/prices.rs:34`, `alerts.rs:150`, `web/portfolio_feed.rs:145` | F |
| H8 | Missing FX rate silently treated as 1.0 EUR in net worth | `domain/finance/fx.rs:70` | F |
| H9 | Recurring bills: f64 amounts, hard-coded exponent 2 (JPY ×100), `inf` → `i64::MAX` breaks SUMs; patch changes currency without exponent; net worth mixes exponents | `chat/recurring_bills.rs:163`, `llm/tools/bills.rs:37`, `repositories/recurring_bills.rs:204`, `read_models/net_worth.rs:95` | F |
| H10 | Inference: Ogg Opus decode unbounded until end → decompression-bomb OOM | `inference/src/stt/audio.rs:84-130` | F |
| H11 | SPA bills: zone-local picker time sent zone-less, stored as UTC; edit shows UTC | `frontend/src/modules/recurring_bills/BillModal.tsx:90,114` | F |
| H12 | SPA "today" = UTC date in mood, time blocks, life, footprints | `modules/mood/index.tsx:17`, `time_blocks/index.tsx:16`, … | F |
| H13 | SPA footprint edit pre-fills `toFixed(2)` → JPY edits rejected, KWD truncated | `modules/footprints/FootprintModal.tsx:68` | F |

## Medium

| # | Issue | Where | Status |
|---|---|---|---|
| M1 | Recurrence drifts (Jan 31 → Feb 28 → Mar 28 …), steps in UTC (DST shift, wrong local day); bills ignore `due_day` | `domain/reminders/mod.rs:140`, `domain/recurring_bills/mod.rs:141` | F |
| M2 | Reminder delivery race: `reschedule`/`mark_triggered` unguarded → a cancel/snooze during send is undone | `repositories/reminders.rs:202,287`, `application/reminders/delivery.rs:72,121` | F |
| M3 | Completed recovery never announced to linked identities (`outbound = None`) | `interface/http/auth/recovery.rs:111` | F |
| M4 | `mark_paid`: no status check (pays completed bills), no lock, expense created after commit | `application/recurring_bills/mod.rs:117` | F |
| M5 | LLM numeric amounts forced through `{:.2}` (JPY rejected, KWD rounded, 0.004 → 0) | `llm/tools/finance.rs:162`, `portfolio/*` | F |
| M6 | Portfolio cash ledger mirror outside the tx; failure swallowed → double count | `application/portfolio/cash.rs:64` | F |
| M7 | Merge drops colliding holdings/cash/snapshots instead of summing; moved transactions keep deleted category ids | `infrastructure/postgres/merge.rs:83` | F |
| M8 | Portfolio audit hash chain forks under concurrent writes (no per-owner lock) | `repositories/portfolio/audit.rs:27` | F |
| M9 | Habits: web/chat "today" is UTC, AI uses user tz; `date` unvalidated | `interface/http/habits.rs:147`, `chat/habits.rs:43` | F |
| M10 | Day/month buckets use DB session tz; life-activity sections swallow errors | `read_models/life.rs`, `repositories/finance/analytics.rs:64` | F |
| M11 | Streaming LLM decodes each chunk with `from_utf8_lossy` → CJK split across chunks corrupted | `infrastructure/inference/llm/client.rs:196` | F |
| M12 | Reminders created via MCP/agent stored with room `mcp`/`agent:internal` → silently "delivered" | `interface/mcp/adapter.rs:40`, `llm/tools/reminder.rs:111` | F |
| M13 | MCP token passes every `AuthUser` route, incl. minting non-expiring MCP tokens and AI config | `interface/http/mcp.rs:137` | F |
| M14 | Bookmark metadata fetch: DNS rebinding between check and connect | `infrastructure/web/metadata.rs:99` | F |
| M15 | Matrix edits (`m.replace`) re-executed as new commands / AI turns | `interface/channels/matrix/handler.rs:63` | F |
| M16 | Email DMARC accepts any matching `Authentication-Results` header (forgeable lower header) | `interface/channels/email/inbound.rs:86` | F |
| M17 | `!webhook create` posts the secret token into group rooms | `interface/chat/webhooks.rs:44` | F |
| M18 | Inbound webhook title/message uncapped (`MAX_TITLE_LEN` unused) | `interface/http/webhooks/inbound.rs:98` | F |
| M19 | Footprint update with only a new currency reinterprets stored minor units | `application/footprints/mod.rs:199` | F |
| M20 | Inference STT/OCR: decode on Tokio worker, no concurrency cap, model reloaded per request | `inference/src/api/stt.rs:166`, `stt/sherpa.rs:179`, `ocr/paddle.rs:216` | F |
| M21 | FLAC / 24-bit WAV decode to empty transcript (S32/S24 buffers dropped) | `inference/src/stt/audio.rs:180` | F |
| M22 | STT/OCR/TTS models downloaded without checksum from mutable URLs | `stt/sherpa.rs:87`, `ocr/paddle.rs:59`, `tts/kokoro.rs:22` | F |
| M23 | SPA money: activity log shows `1,000.0` for exponent 0; retrospective divides by 100 | `modules/portfolio/ActivitySection.tsx:34`, `ai/RetrospectiveModal.tsx:58` | F |
| M24 | SPA timestamps formatted without the saved zone | webhooks, bookmarks reader, portfolio, notes | F |
| M25 | SPA habits/bookmarks use raw `fetch` (no 401 → sign-in, no timeout) | `modules/habits/index.tsx`, `modules/bookmarks/index.tsx` | F |
| M26 | Briefing trigger joins/posts to unvalidated `dev_room_id` | `interface/http/briefing.rs:102` | F |

## Low

| # | Issue | Where | Status |
|---|---|---|---|
| L1 | Concurrent passkey deletes can remove the last passkey (count without lock) | `repositories/accounts/credentials.rs:136` | F |
| L2 | Trade cost truncates toward zero (0.5 @ 0.01 costs 0) | `domain/portfolio/models.rs:78` | F |
| L3 | `!mcp revoke .` → `LIKE '%'` revokes every token | `repositories/mcp.rs:155` | F |
| L4 | Webhook `sanitize_html`: `>` inside quoted attr and entity-encoded `javascript:` bypass | `domain/webhooks/renderer.rs:103` | F |
| L5 | Reminder `seconds` huge → `Duration::seconds` panic; unbounded `page` overflow | `interface/http/reminders.rs:172,245` | F |
| L6 | Matrix parser: `@bot botanical` → command `anical`; byte-offset slice can panic | `interface/channels/matrix/dispatcher.rs:61` | F |
| L7 | Footprint arrival/departure dates in HTML unescaped | `interface/chat/footprints.rs:134,217` | F |
| L8 | 100 permanently-deferred due reminders starve later ones | `application/reminders/delivery.rs:24` | F |
| L9 | Long low-rate audio skips resampling (played at 2× speed) | `inference/src/stt/audio.rs:244` | F |
| L10 | HTML mention detection includes `<mx-reply>` fallback | `interface/channels/matrix/dispatcher.rs:37` | F |
| L11 | Legacy-dir models trigger per-request concurrent downloads into one tmp path | `stt/sherpa.rs:225`, `ocr/paddle.rs:236` | F |
| L12 | Default storage signing secret not refused in production | `config/storage.rs:43` | F |

## Plausible, not fully verified
- Fresh Matrix login (no sync token) may replay recent timeline messages as commands (`matrix/sync_loop.rs:78`).
- Daily briefing has a 5-minute window; downtime or a DST-skipped time loses that day's briefing (`domain/settings/mod.rs:252`).
