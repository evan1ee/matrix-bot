# Agent handoff

**Read this before touching code.** [`AGENTS.md`](../../AGENTS.md) holds the rules; this file holds the state: what is done, what an operator must still do, what is open, and the rulings that settled each disagreement. Design rationale lives in [`docs/architecture.md`](../architecture.md) and its sibling documents, not here.

> **Status (2026-10):** every architectural phase and all Track 2 post-cutover enhancements are implemented: the PostgreSQL foundation, omnichannel adapters, object storage, the autonomous agent runtime with deep procedural pipelines and Kokoro TTS audio compilation, the web console with WebAuthn conditional UI autofill, the notes and knowledge base (pgvector hybrid search), and dynamic configuration with the encrypted credential vault. The web AI assistant streams replies, tool steps and Stop (phase 1, awaiting merge). What remains is operator cutover (Track 1), immediate technical debt remediation, and the brainstormed feature roadmap (beginning with Web AI Assistant Phase 2).

---

## 1. Orientation

`matrix-bot` is a personal AI/life workspace reached through Matrix, Discord, Telegram, Email, a REST/SSE API, MCP and a React SPA. Crates and layers are described in [`AGENTS.md`](../../AGENTS.md) §1 and §3; the designs behind them in [`docs/architecture.md`](../architecture.md), [`database.md`](../database.md), [`api.md`](../api.md) and [`llm.md`](../llm.md).

## 2. Safety rules

The hard safety rules are [`AGENTS.md`](../../AGENTS.md) §9 and the security rules §7; they are not repeated here. In short: never touch the production `.env` or `data/`, never run the bot binary (only `cargo run --bin generate-openapi`), never push, merge, open a PR or deploy without an explicit instruction. Gitea (`git@git.mbm.one:evan/matrix-bot.git`) is primary and GitHub (`origin`) is a hand-updated mirror.

The step-up routes are whatever `SENSITIVE` lists in `interface/http/mod.rs::step_up_tests`; read the list, not a summary of it.

## 3. Verified baseline

[`reviews/2026-10-11-adversarial-review.md`](reviews/2026-10-11-adversarial-review.md): all 53 verified findings (`C1`–`C2`, `H1`–`H13`, `M1`–`M26`, `L1`–`L12`) across `matrix-bot`, `matrix-bot-inference` and the frontend are fixed (`Status: F`) with regression tests.

The gates in [`AGENTS.md`](../../AGENTS.md) §8 were green when those fixes landed (`6859962`, 2026-10-12). Counts go stale, so none are recorded here: rerun the gates before relying on this line.

---

## 4. Remaining work

### Track 1: Production cutover (operator, user-gated)

Follow `docs/superpowers/runbooks/postgres-foundation-cutover.md` in sequence:

1. **Pre-Cutover Channel Verification (Staging / Non-Production):**
   - **Discord:** Confirm the bot needs no privileged gateway intent (it uses `GUILDS` and `DIRECT_MESSAGES`); verify `/todo`, `/expense`, `/portfolio`, `/ask`, and `/link` in DM and guild.
   - **Telegram:** Verify `/start`, `/todo` inline keyboard buttons ("Done #id"), `/link`, and secret header verification (`TELEGRAM_WEBHOOK_SECRET`) in webhook mode.
   - **Email:** Verify IMAP mailbox polling over TLS, DMARC authentication check (`EMAIL_AUTHSERV_ID`), and multipart plain/HTML reply formatting via SMTP.

2. **Production Database Pre-Flight & Migration Rehearsal:**
   - **Drain Legacy Webhooks:** Check `SELECT COUNT(*) FROM webhook_outbox WHERE status = 'pending'`. Allow the legacy binary to drain pending alerts, or accept dropping them (migration `0004` drops `webhook_outbox`).
   - **Check Orphan Matrix Links:** Verify `SELECT l.mxid FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id WHERE a.id IS NULL;` returns 0 rows. Remove any orphaned links before migration `0006` backfills to `platform_identities`.
   - **Rehearse Migrations on Production Dump:**
     ```bash
     pg_dump -Fc "$PROD_URL" > prod.dump
     createdb rehearsal
     pg_restore -d rehearsal --no-owner prod.dump
     MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
     PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
       cargo test -p matrix-bot migrates_a_real_database -- --ignored
     ```

3. **Inference & Secrets Environment Setup:**
   - Deploy `matrix-bot-inference` and wait until `/health` is ready (first start downloads the ~135 MB `multilingual-e5-small` model into `EMBEDDING_MODEL_DIR` and verifies its SHA-256).
   - Set `EMBEDDING_MODEL=multilingual-e5-small` and configure a 32-byte `SECRETS_ENCRYPTION_KEY` (`openssl rand -base64 32`) in production before booting the backend.

4. **Supervised Production First Boot:**
   - Take a complete backup of PostgreSQL and the `data/` directory (`bot_store/`, `matrix_session.json`, `matrix_token.json`, `verified_users.json`).
   - Ensure writable storage directory (`STORAGE_LOCAL_DIR`, default `data/storage`).
   - Boot binary and verify log output:
     - Log confirms restored session: `Reusing stored Matrix session for ... on device <id>`.
     - Log confirms cross-signing self-verification: `Device::verify()` succeeded (`Ok`).
     - Check bot account for stray device IDs from prior testing.
     - Send `!say Hello` in a Matrix room to verify MSC3245 audio voice bubble with waveform rendering.
     - Verify `SELECT platform, COUNT(*) FROM platform_identities GROUP BY platform;` reflects all migrated user links.
     - Grant initial administrator role to your passkey-enrolled account via `matrix-bot admin grant <account_id>`.

---

### Track 2: Post-cutover enhancements and technical debt

- Tech-debt register: [reviews/2026-10-10-tech-debt-audit.md](reviews/2026-10-10-tech-debt-audit.md). Phases 0 and 1 are done (except TD-05 beyond the admin module). Phase 2 next: TD-10 (reqwest 0.12, own branch), TD-13.

- **Todo tags, recurrence and checklist steps:** implemented on branch `feat/todo-tags-recurrence-steps` (migration `0013`), merged into local `master` and not yet pushed. Spec and plan: `specs/` and `plans/` `2026-10-18-todo-tags-recurrence-steps.md`. Follow-ups from the spec's non-goals: projects/lists; tag rename, colours or metadata; custom intervals ("every 2 weeks", "weekdays"); recurrence measured from completion; steps with their own due date, priority, reminder or nesting; drag-and-drop step reordering.

1. **Quote-Based AI Confirmation Without Taint (Fixed):**
   - Implemented authentic AI reply tracking via in-memory bounded cache (`AiReplyTracker`) in `LlmService`. Sent AI response message IDs are recorded by channel adapters (Matrix `show_reply` capturing both placeholder and replacement event IDs, and Telegram worker). Inbound quotes are verified against `(room_key, event_id, owner_id)`. Quoting genuine AI confirmation prompts avoids turn taint and allows held action execution without infinite confirmation loops, while webhook alerts, foreign messages, and cross-room/cross-owner quotes strictly remain tainted.

2. **Deep Procedural Pipelines for Built-in Agent Tasks (Fixed):**
   - Implemented 4 deterministic domain calculation engines with integer minor-unit math and strict invariant validation:
     - `portfolio::rebalance::evaluate_portfolio_drift`: Computes asset allocation drift percentages against target weights and derives rebalancing actions (`Buy`, `Sell`, `InBand`) without direct trade execution.
     - `finance::audit::evaluate_monthly_audit`: Aggregates transactions, splits recurring bills vs discretionary spending, computes percentage distributions, and flags category spending spikes exceeding a configurable threshold.
     - `recurring_bills::due::evaluate_bill_cash_adequacy`: Groups recurring bills by temporal urgency buckets (`DueToday`, `DueWithin3Days`, `DueWithin14Days`, `Later`) and evaluates cash reserve adequacy against liquid cash account balances.
     - `habits::consistency::evaluate_habit_consistency`: Evaluates 7-day completion rates, tracks active streaks, and categorizes trends (`Strong`, `Stable`, `AtRisk`, `Slipping`).
   - Exposed 4 analytical tools to the autonomous agent and chat LLM via `llm/tools/` conforming to the OpenAI schema standard (`evaluate_portfolio_rebalance`, `audit_monthly_finances`, `evaluate_bill_due_adequacy`, `evaluate_habit_consistency`).
   - Enhanced all 5 built-in task plugins (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`) with specialized tool whitelists, deep procedural prompts, and configurable thresholds.
   - Built multi-modal Kokoro TTS audio compilation pipeline in `backend/src/interface/jobs/agent/audio.rs` and `mod.rs`: extracts `<audio_script>...</audio_script>`, synthesizes Ogg Opus audio via `TtsClient`, attaches it to delivered messages (`AttachmentKind::Audio`), cleanly strips tags from plain/HTML text, and degrades fail-open if TTS is unconfigured or unavailable.

3. **Discord Channel Capabilities Expansion (Fixed):**
   - Implemented interactive ActionRow buttons in `render.rs` and `outbound.rs` chunked up to 5x5 per message (`CreateActionRow::Buttons`), with component interaction parsing in `inbound.rs` and dispatching in `gateway.rs` and `worker.rs` (`Work::Component`).
   - Implemented typed slash command options and subcommands in `commands.rs` and `inbound.rs` (`/todo add task:`, `/todo done id:`, `/todo list page:`, `/expense add amount: note:`, `/expense list`) with backward-compatible single `args` string parsing.
   - Implemented inbound receipt/image attachment ingestion over Discord DMs in `gateway.rs` and `worker.rs` (`Work::DirectAttachment`), resolving linked account owners, enforcing 10 MiB limit and download timeout, persisting into object storage (`StorageCategory::Receipts`), enqueuing durable `media.process_receipt` background jobs, and replying with receipt acknowledgments.

4. **WebAuthn Conditional UI (Passkey Autofill) (Fixed):**
   - Added `isConditionalMediationAvailable()` check and `opts?: SignInOptions` (`{ conditional, signal }`) in `frontend/src/core/passkey.ts`.
   - Integrated passkey autofill input (`autoComplete="username webauthn"`) in `frontend/src/components/AuthModal.tsx` in `'signin'` mode.
   - Managed silent background conditional mediation lifecycle with `AbortController`, cleanly aborting on modal close, mode transitions, and manual sign-in without user-facing alert popups.
   - Added comprehensive Playwright E2E virtual authenticator tests verifying autofill, cancellation, and manual fallback.

5. **Minor Deferred Edge Cases (Fixed):**
   - `SyncHealth`: Fallback handling added (`|| 'unknown'`) when formatting last sync timestamp in web UI.
   - Morning briefing window: Implemented wrap-around logic for target briefing windows across hour boundaries (`:58`–`:02`) and across midnight.

6. **Review follow-ups left open by the tech-debt pass (Fixed):**
   - `ScheduledAgentTask::allowed_tools()` defaults to `Some(Vec::new())` (fail-closed) and asserted `is_some()` in `every_builtin_whitelist_names_registered_tools`.
   - `resume.rs` checks `tool_policy` before `begin_approved_execution` commits the "started" marker.
   - Agent execution job handler, `chat/llm.rs` (`handle_show_config`), and `http/ai.rs` (`get_ai_config`) resolve via `ResolvedAiConfig::resolve_effective(...)` honoring the dynamic snapshot and credential vault.
   - `grant_admin`/`revoke_admin` audit the CLI/system operator as actor (`None`); account merges that carry over the admin role record an `AdminGranted` audit entry with `reason: account_merge`.
   - `SearchService` caches and reuses the dynamic embedding HTTP client while the custom base URL and model remain unchanged.
   - `resolve_runtime_stt` and `resolve_runtime_tts` log warnings on vault decrypt errors instead of silently dropping them.
   - `POST /agent/approvals/:id/approve` requires `VerifiedUser` (enforcing step-up within 5 minutes) and is registered in `SENSITIVE`.

7. **Web AI assistant streaming, Stop and tool steps (Done, phase 1; spec [`2026-10-18-web-ai-streaming.md`](specs/2026-10-18-web-ai-streaming.md)):**
   - `POST /ai/chat/stream`, `ProgressSink` tool and retraction methods, safe-point cancellation with a stored `_(stopped)_` marker, `ai_messages.tool_outcome` (migration `0012`), and a frontend live-turn store with Stop and tool-step rendering. Reference: [`api.md`](../api.md#streaming-ai-chat), [`llm.md`](../llm.md#progress-streaming-and-stopping). Branch `feat/web-ai-streaming`, awaiting the owner's merge.
   - **Next (phase 2):** conversation list and switching, a non-destructive "New chat" (today it deletes the conversation), and the model chip's hard-coded `gpt-4o-mini` fallback label. Phase 3: regenerate, edit-and-resend, copy.
   - **Known follow-ups:** `llm::MAX_PROMPT_LENGTH` is 16384 while the HTTP handlers cap the prompt at 4000 bytes (pre-existing); the frontend counts characters, so CJK prompts are refused near 1,333 characters. `get_ai_config` merges config in the handler and AI handlers call `llm_repo` directly instead of `LlmService` (AGENTS section 2, rule 2). A turn that panics, or is dropped past the 5 s timeout grace, stores no stopped marker.

### Finance budgets (2026-10, branch `feat/finance-budgets`)

Merged into local master (not pushed): monthly spending limits per category or overall, per currency.

| Piece | Where |
|---|---|
| Domain rules (`Month`, `Threshold` 80/100, `BudgetStatus`, pace) | `domain/finance/budget.rs` |
| Schema `finance_budgets`, `finance_budget_alerts` (`finance_budgets` in `OWNED_TABLES`) | migration `0014_finance_budgets.sql` (renumbered from `0012` at merge, after `0012_ai_message_tool_outcome` and `0013_todo_tags_recurrence_steps`) |
| CRUD, status, evaluation inside the expense write's transaction | `FinanceService` (`application/finance/{budgets,budget_eval,transactions}.rs`) |
| Alert delivery | `finance.budget_alert` job in `interface/jobs/budgets.rs` (shared DM loop in `interface/jobs/dm.rs`) |
| Morning briefing section | `application/briefing/generator.rs` |
| Chat | `!budget` (`interface/chat/finance/budget.rs`) |
| REST | `/api/v1/finance/budgets` ([`api.md`](../api.md)) |
| AI tools | `get_budget_status`, `set_budget`, `remove_budget` (`llm/tools/budgets.rs`) |
| Web | `BudgetPanel` in the finance module |

Design: [`specs/2026-10-18-finance-budgets.md`](specs/2026-10-18-finance-budgets.md); plan: [`plans/2026-10-18-finance-budgets.md`](plans/2026-10-18-finance-budgets.md).

**Follow-ups**

- AI tools in `llm/tools/finance.rs` and the other tool files still return `AppError::to_string()` to the model, which leaks `database error:` text; `budgets::model_error` could be shared and used everywhere.
- Web minors: the budget edit modal sends unchanged fields (can overwrite a concurrent alerts change), a failed delete is unhandled (the row stays in the confirming state), the modal has no focus trap or focus return, and status colours are hard-coded instead of tokens.
- Test thin spots: DST-gap fallback in `Month` bounds; `find_by_scope_for_update` with `Some(id)`; cross-owner update/delete `NotFound` and `BudgetDeleted` event tests; merge test lacks case-insensitive name and currency-mismatch cases; alert handler lacks failed-send-continues, threshold 100 and missing-account cases; REST tests lack limit edge cases and route wiring.
- Behaviour: editing an unrelated field of an expense (e.g. its note) re-evaluates budgets and can fire a late alert when spend already exceeds a budget created after the spend; at most once per budget/month/threshold. Chat unknown-category detection uses `msg.contains`, and a 3-letter all-caps category name is read as a currency.
- REST `POST /finance/budgets` resolves `category_id` → name → id case-insensitively (`owned_category_name` → `resolve_category_in_tx`), so with both "Food" and "food" present a `category_id` can attach to the wrong case-variant. Root cause: `finance_categories` uniqueness is case-sensitive but merge/resolve match on `LOWER(name)`.
- The budget alert job (`interface/jobs/budgets.rs`) formats live numbers at delivery time and does not re-check `alerts_enabled`: a Warning job delivered after spend passed 100 % reads "Budget warning … (105 %)", and turning alerts off between enqueue and delivery still sends the DM. Spec chose live numbers deliberately; revisit if confusing.

---

### Technical debt & quality items

Immediate code quality issues and debt identified across the codebase:

1. **Split `backend/src/domain/finance/budget.rs` (approaching modular limit):**
   - Total file length is 528 lines (335 production lines + 193 unit test lines; [`AGENTS.md`](../../AGENTS.md) §4.3 target is < 300 production lines).
   - Decompose into a `domain/finance/budget/` directory module:
     - `budget/mod.rs`: Domain exports, top-level type aliases, and error conversions.
     - `budget/scope.rs`: `BudgetScope` enum, category normalization, and scope validation.
     - `budget/threshold.rs`: `Threshold`, `BudgetStatus`, pace projection calculation, and minor-unit threshold math.
   - Retain unit tests adjacent to the extracted components without weakening invariants.

2. **Fix multi-currency blind addition in `domain/recurring_bills/due.rs:75`:**
   - In `evaluate_bill_cash_adequacy`, `total_due_minor += b.amount_minor;` sums amounts across varying currencies (e.g. JPY, USD, EUR) without conversion or exponent normalization, and compares against `available_cash_minor` to derive `projected_shortfall_minor`.
   - Fix: Partition analyzed bills by `currency`, compute cash reserve adequacy and shortfall metrics per currency, or require matching account currency before computing shortfall.

3. **~~Fix vector embedding deletion leak in `BookmarkService::delete` and `FootprintService::delete`~~ — done 2026-10-31 (Blueprint 8; bookmarks, footprints and journal now clean up in-transaction, footprints and journal enqueue versioned embed jobs):**
   - `BookmarkService::create` enqueues embedding jobs (`KnowledgeItemType::Bookmark`), but `BookmarkService::delete` deletes the database row without calling `repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, "bookmark", id)` (unlike `NoteService::delete_note`).
   - `FootprintService::delete` similarly lacks symmetric embedding deletion cleanup (and `LifeService::delete_day_log` must also include it once journal entries are embedded).
   - Fix: Call `delete_embeddings_for_item` inside the deletion transaction before commit, keeping vector storage consistent.

---

### Feature roadmap & open backlog (Brainstormed 2026-10)

> **Authoritative Specification:** Complete production-grade architectural blueprints for these capabilities are documented in [`docs/superpowers/specs/2026-10-31-feature-blueprints.md`](specs/2026-10-31-feature-blueprints.md).

High-impact capabilities brainstormed via multi-agent architectural analysis across all domains:

1. **Web AI Assistant Phase 2 & 3:**
   - **Multi-conversation switcher:** Conversation list and switcher dropdown (`GET /ai/conversations`), switching `activeConversationId` without state leakage.
   - **Auto-generated conversation titles:** Summarize first turn or prompt into clean conversation titles (replacing default timestamps or blank titles).
   - **Model badge dynamic reflection:** Replace hardcoded `'gpt-4o-mini'` in `AiAssistantView.tsx` with effective model name from `useQuery(['ai-config'])` and backend `ResolvedAiConfig`.
   - **Non-destructive New Chat:** Replace destructive `DELETE /ai/conversations/:id` in `AiAssistantView.tsx` (which permanently erases the conversation) with a clean conversation unsetting/minting action, preserving prior conversation history.
   - **Phase 3 interactions:** Message regeneration, edit-and-resend prompt forks, and one-click markdown copy to clipboard.

2. **Universal Calendar Feed (RFC 5545 iCalendar / `.ics`):**
   - Read-only Webcal subscription endpoint (`/api/v1/calendar/feed.ics` with token authentication) for calendar clients (Apple Calendar, Google Calendar, Outlook).
   - Aggregates actionable temporal items: todos with `due_at`, recurring bills with calculated next `due_date`, and standalone reminders (`remind_at`).
   - Reminder source deduplication: Ensure reminder records spawned directly from todos or bills (`reminders.source_kind` / `reminders.source_id`) do not duplicate their parent entities in the feed.
   - RFC 5545 compliance: `VEVENT` / `VTODO`, deterministic `UID` generation (`todo-{id}@matrix-bot`), `DTSTAMP`, and reminder alarms (`VALARM`).

3. **Omnichannel Media & Voice STT Parity:**
   - **Telegram Voice Notes:** Ingest voice and audio messages in Telegram webhook/polling worker, dispatch audio payloads to `matrix-bot-inference` Whisper STT, and route transcripts into inbound command dispatcher.
   - **Discord Voice Messages:** Ingest Discord DM voice attachments (`audio/ogg`, etc.) and route through STT pipeline.
   - **Email Inbound Attachment Parsing:** Extract attached receipts and invoices (PDF, PNG, JPEG) from inbound IMAP email messages, persist into `StorageCategory::Receipts`, and enqueue `media.process_receipt` OCR jobs.

4. **Scanned Receipt Preservation:**
   - Ingestion already saves receipts into `StorageCategory::Receipts` and forwards `receipt_storage_key` to `record_expense` (`interface/jobs/media.rs:194`), and the database column `receipt_storage_key` already exists on `transactions` (migration `0008`).
   - Gaps to complete:
     - Expose `receipt_storage_key` on `TransactionResponse` in `interface/http/finance.rs` and frontend `Transaction` type (`frontend/src/modules/finance/types.ts`).
     - Web upload persistence: Persist files uploaded via `/api/v1/ocr/scan` (`ReceiptDropzone.tsx`) to `StorageCategory::Receipts` and link the generated `receipt_storage_key` upon expense creation.
     - Web console UI: Lightbox / receipt viewer modal in `TransactionList.tsx` allowing users to view the stored receipt image alongside transaction details via presigned HMAC read URLs from `StorageService`.

5. **Second Brain Bidirectional Wiki-Links (`[[Note Title]]`):**
   - Pure domain parser in `domain/knowledge/` extracting `[[Note Title]]` references from note Markdown bodies.
   - Persistence & relations: Forward and backlinks join table (`knowledge_note_links`: `source_note_id`, `target_note_id`, `target_title`, `owner_id`).
   - Web UI: Autocomplete popup for `[[` in note editor, clickable inter-note links in rendered view, backlink list in note detail sidebar, and visual force-directed knowledge graph.

6. **Unified Multi-Domain Knowledge Search:**
   - Plumb Journal (`life_day_logs`) and Footprints into vector search: `KnowledgeItemType` already has `Journal` and `Footprint` variants and repository extraction queries in `get_item_content_for_embedding`, but `LifeService::record_day_log` and `FootprintService::record_footprint`/`update_footprint` must enqueue `NewJob::knowledge_embed_item` jobs upon creation/update.
   - Symmetric deletion cleanup: Ensure `BookmarkService::delete`, `FootprintService::delete`, and `LifeService::delete_day_log` invoke `repositories::knowledge::delete_embeddings_for_item` inside the deletion transaction before commit (matching `NoteService::delete_note`).
   - Unified hybrid search query spanning all personal knowledge artifacts (notes, bookmarks, journal, travel history).

7. **Scheduled Morning Briefing Podcast:**
   - Synthesize morning briefing `<audio_script>...</audio_script>` into high-quality Ogg Opus audio via Kokoro TTS on `matrix-bot-inference` (extending the procedural pipeline in `interface/jobs/agent/audio.rs` to `interface/jobs/briefing.rs`).
   - Persist generated audio into object storage (`StorageCategory::Audio`).
   - Web console integration: Dedicated audio player / podcast card on dashboard or briefing view for web-only users, with playback controls and download options.

8. **Autonomous Scheduled Agent Task Controls:**
   - REST API endpoints for agent task lifecycle management: pause, resume, trigger immediate out-of-schedule run, and delete (`PATCH /api/v1/agent/tasks/:id/state`, `POST /api/v1/agent/tasks/:id/trigger`, `DELETE /api/v1/agent/tasks/:id`).
   - Web console UI: Automation management dashboard listing active schedules, last run status, execution duration, pause/resume toggles, and manual run triggers.

9. **Multi-Currency Cash Runway & Subscription Auto-Discovery:**
   - Fix multi-currency blind addition in `domain/recurring_bills/due.rs:75` by grouping calculations per currency or converting against real-time portfolio/forex exchange rates.
   - Subscription auto-discovery: Analytical domain engine scanning transaction history for repeating cadence patterns (same merchant/amount every ~30 days), surfacing candidate subscriptions to the user.

10. **Global Command Palette (`Cmd+K` / `Ctrl+K`):**
    - Omnipresent web console modal accessible via `Cmd+K` / `Ctrl+K`.
    - Three-tier fuzzy search & command palette:
      1. Navigation to any of the 20 web modules.
      2. Quick actions (create todo, record expense, add bookmark, create note, trigger agent run).
      3. Global instant hybrid search across notes, bookmarks, transactions, and footprints.

---

### Next actions

1. **Operator cutover (Track 1):** Execute migration rehearsal and supervised first boot against production backup following `docs/superpowers/runbooks/postgres-foundation-cutover.md`.
2. **Immediate technical debt fixes:**
   - Split `domain/finance/budget.rs` (528 lines, 335 prod lines) into `budget/scope.rs` and `budget/threshold.rs` to reach the < 300-line target.
   - Fix multi-currency blind addition in `domain/recurring_bills/due.rs:75`.
   - ~~Fix vector embedding deletion leak~~ (done, Blueprint 8).
3. **Web AI Assistant Phase 2:** Implement multi-conversation thread switcher, dynamic model badge, and non-destructive new chat in `AiAssistantView.tsx`.
4. **Roadmap execution (2026-10-31):** Phase A done for Blueprints 8 and 10 (plans: `plans/2026-10-31-knowledge-embedding-lifecycle.md`, `plans/2026-10-31-agent-task-controls.md`); Blueprint 1 (Web AI Phase 2/3) done too (plan: `plans/2026-10-31-web-ai-threads.md`), so Phase A is complete; Phase B: Blueprint 5 (receipt viewer) done (plan: `plans/2026-10-31-receipt-preservation-viewer.md`), Blueprint 4 (email receipts) done (plan: `plans/2026-10-31-email-receipt-ingestion.md`); Blueprint 3 (Telegram/Discord voice) done (plan: `plans/2026-10-31-audio-ingress-parity.md`); Blueprint 2 (calendar feed) done and merged (plan: `plans/2026-10-31-calendar-feed.md`); Phase C: Blueprint 7 (wiki-links and graph; plan `plans/2026-10-31-note-wiki-links.md`), Blueprint 6 (briefing podcast; plan `plans/2026-10-31-briefing-podcast.md`) and Blueprint 9 (command palette; plan `plans/2026-11-01-command-palette.md`) done and merged into local `master` (2026-11-01, unpushed); all ten blueprints are implemented. Follow the implementation blueprints in [`docs/superpowers/specs/2026-10-31-feature-blueprints.md`](specs/2026-10-31-feature-blueprints.md) across Phase A (critical data integrity & core usability), Phase B (omnichannel parity & financial automation), and Phase C (personal productivity & knowledge management).

---

## 5. Settled rulings

Format: `Ruling: <decision> — <why> — <cost if wrong>`. New rulings go here in the same commit as the work (AGENTS §4.1).

### Rulings (2026-10-10)

- *Ruling: CLI/SQL-only admin bootstrap (`matrix-bot admin grant|revoke <account_id>`) — eliminates first-user auto-promotion race conditions and requires an enrolled passkey before granting or using admin privileges — cost if wrong: operator runs one CLI command after first passkey enrollment.*
- *Ruling: `AdminSession` for admin `GET`s and `AdminUser` (5-minute step-up window) for admin writes and secret connectivity tests — prevents unnecessary WebAuthn prompts when browsing the Admin Console while gating all mutations and secret operations — cost if wrong: none, secret `GET` returns masked metadata only.*
- *Ruling: Keep only the 20 hot-reloadable settings in `SettingRegistry` and the 6 hot-reloadable secret providers in `SecretProvider` (`OpenAi`, `Anthropic`, `Tavily`, `Stt`, `Tts`, `Generic`), keeping restart-required settings and credentials (channels, `EMBEDDING_MODEL`, and S3) in `.env` — avoids misleading Admin UI settings/secrets that cannot hot-reload and keeps `domain/config/registry.rs` under the 500-line limit — cost if wrong: changing channel adapters, S3, or embedding model requires `.env` edit + restart, which is already required by their lifecycle.*

### Rulings (2026-10-12 — Migration squash)

- *Ruling: squashed migrations 0003–0016 into 0003–0011 (0005 folded into 0004; 0012–0014 into 0010 `knowledge_base`; 0016 into 0011 `configuration_and_secrets`; the rest renumbered) — nothing past `0001` had been released and the production cutover had not happened, so only dev/test databases had applied them, and the squashed files build the identical final schema — a database that already recorded the old versions in `_sqlx_migrations` fails with a checksum/missing-migration error and must be recreated; the notes legacy-derived-column runbook step and the 0014 guard are gone with it. After the first production cutover, committed migrations are immutable again.*

### Rulings (2026-10-12 — Adversarial Review Remediation)

- *Ruling: `AuthUser::never_presented_passkey()` (`last_verified_at <= UNIX_EPOCH`) decides which sessions `AdminSession` refuses — `create_unverified` stores the Unix epoch because `timestamptz` cannot hold `MIN_UTC`, so the old `== MIN_UTC` check let an admin's Matrix access-token session read admin endpoints — a future sentinel must stay at or below the epoch.*
- *Ruling: Passkey enrollment via `/api/v1/account/passkeys/register/*` requires `VerifiedUser` (5-minute step-up window) and rejects MCP/recovery-restricted sessions; recovery enrollment uses `/api/v1/auth/recovery/*` exclusively (`C2`), and Matrix login sessions carry `last_verified_at = NULL` (`H1`) — closes privilege escalation from unverified/MCP/recovery sessions and Matrix tokens — cost if wrong: enrolling an additional passkey from the web console prompts for existing passkey step-up.*
- *Ruling: Room/channel delivery targets (`authorize_delivery`, `H4`/`M12`/`M26`) allow only `web`, an account's cached Matrix room, or a conversation address (`discord:`, `telegram:`, `email:`) matching the account's linked `platform_identities`; synthetic origins (`mcp`, `agent:*`) resolve to the user's saved default room or `web` — prevents cross-user message injection via raw room IDs or foreign channel addresses — cost if wrong: rooms not yet in the user's room cache cannot be targeted by raw ID until synced.*
- *Ruling: All outbound HTTP clients for user-configurable or untrusted URLs (`llm/client.rs`, `web/metadata.rs`, `H2`/`H3`/`M14`) disable automatic redirect following or re-validate each redirect hop and pin validated DNS socket addresses via `reqwest::ClientBuilder::resolve_to_addrs` — prevents SSRF via open redirects, IPv4-mapped IPv6 (`::ffff:127.0.0.1`), and DNS rebinding — cost if wrong: none.*
- *Ruling: Minor-unit currency exponents (`domain::finance::money::exponent_for`, `H7`/`H8`/`H9`/`M5`/`M19`/`H11`/`H13`/`M23`) are enforced end-to-end across recurring bills, footprints, portfolio quotes/alerts, net worth (skipping unconvertible currencies rather than assuming 1.0 EUR), LLM tool inputs, and the SPA — eliminates 100× scaling bugs on zero-decimal (`JPY`) and three-decimal (`KWD`) currencies — cost if wrong: none.*

### Rulings (2026-10-13 — Optional Social Channels & Omnichannel Web Support)

- *Ruling: `MATRIX_USERNAME` and `MATRIX_PASSWORD` are optional at boot (`Config::matrix_enabled()`); `SyncHealth::mark_disabled()` marks `/health/ready`'s `matrix_sync` check vacuous (`ok: true`, `last_sync_at: None`) when Matrix is unconfigured — allows the server to run with 0, 1, or any combination of social channels (`Matrix`, `Telegram`, `Discord`, `Email`, or Web-only) without altering `ReadyResponse` in `openapi.json` — cost if wrong: partial Matrix config (`MATRIX_USERNAME` without `MATRIX_PASSWORD`) still fails closed in `Config::validate()`.*
- *Ruling: `RoomService::default_room`, `RoomService::resolve_target_room`, `GET /api/v1/rooms`, and job delivery handlers filter destinations by `Config::configured_channels()` (`Outbound::is_configured`) so linked identities on disabled channels fall back to `web` instead of deferring forever; `CheckAlertsHandler` checks per-owner readiness (at least one linked identity on a connected channel, or no linked identities on any configured channel) before marking alerts triggered; and browser notifications (`Notification` API) request permission only from user clicks in Settings and deduplicate across tabs via `tag` — ensures non-Matrix and Web-only accounts receive reminders, morning briefings, and price alerts without losing alerts during partial channel outages — cost if wrong: none, unowned or unconfigured cross-channel addresses resolve to `web`.*

### Rulings (2026-10-15 — Tech-Debt & Security/Config Hardening)

- *Ruling: Agent approval endpoint `POST /api/v1/agent/approvals/:id/approve` requires `VerifiedUser` and is registered in `SENSITIVE` — aligns the backend with the frontend's `useSensitiveAction` so sensitive AI actions cannot be approved by stale sessions or MCP tokens without a fresh passkey within 5 minutes — cost if wrong: approving an action prompts for passkey verification if the session was verified over 5 minutes ago.*
- *Ruling: `ScheduledAgentTask::allowed_tools()` defaults to `Some(Vec::new())` (fail-closed) — ensures custom or incomplete task plugins cannot execute unapproved tools in unattended background jobs, and `resume.rs` evaluates tool authorization before committing the started execution marker — cost if wrong: custom tasks must explicitly implement `allowed_tools` if they require tool access.*
- *Ruling: `ResolvedAiConfig::resolve_effective(...)` is the shared runtime entry point across chat, agent jobs, and API — ensures autonomous agents and UI reflect the admin console's active provider, model, and encrypted vault secrets rather than falling back strictly to environment variables — cost if wrong: none.*

### Rulings (2026-10-17 — Quote-Based AI Confirmation Hardening)

- *Ruling: `AiReplyTracker` verifies authentic AI replies via `(room_key, event_id, owner_id)` tuple and 1-hour TTL — guarantees quoting the bot's own AI prompts does not trigger untrusted input taint or trap users in infinite confirmation loops, while preventing cross-room replay, cross-owner privilege escalation, and untracked webhook alert steering — cost if wrong: quotes from restarts after > 1 hour fall back to requiring unquoted confirmation.*

### Rulings (2026-10-17 — Discord Channel Capabilities Expansion)

- *Ruling: Discord slash commands register subcommands (`/todo add`, `/todo done`, etc.) without a root-level `args` string option — Discord's interaction API strictly forbids combining root-level arguments and subcommands on the same command (HTTP 400 `50035`), while inbound parsing maintains backward-compatible `args` fallback — cost if wrong: users typing `/todo` without subcommands see Discord autocomplete guide them to subcommands.*
- *Ruling: Direct image ingestion over Discord is restricted strictly to direct messages (DMs) from linked accounts — attachments in guild channels or from unlinked senders fail closed with immediate return or `UNLINKED_REPLY` without downloading bytes or persisting storage — cost if wrong: guild members must DM the bot to scan receipts.*

### Rulings (2026-10-18 — Deep Procedural Agent Pipelines)

- *Ruling: Deterministic domain calculation engines (`portfolio::rebalance`, `finance::audit`, `recurring_bills::due`, `habits::consistency`) perform exact integer minor-unit math and rule evaluation prior to or during LLM synthesis — prevents floating-point inaccuracies, LLM hallucination in financial/streak math, and guarantees consistent analysis across all 5 built-in agent tasks — cost if wrong: none.*
- *Ruling: Multi-modal Kokoro TTS audio digest compilation extracts `<audio_script>...</audio_script>`, synthesizes Ogg Opus audio via `TtsClient`, attaches it as `AttachmentKind::Audio` to the delivered message, and strips the tag from plain/HTML text with fail-open graceful degradation — gives users audio podcast briefings in supported channels without breaking text delivery or aborting agent runs if TTS is unconfigured or unavailable — cost if wrong: none.*
- *Ruling: Non-execution trade safety invariant enforced across portfolio tools — `evaluate_portfolio_rebalance` calculates mathematical drift and recommended buy/sell rebalancing amounts, explicitly instructing the agent to explain adjustments without executing trades directly — prevents autonomous agent execution of destructive financial transactions without human-in-the-loop approval — cost if wrong: none.*

### Rulings (2026-10-18 — Web AI Streaming)

- *Ruling: keep `POST /ai/chat` unchanged beside the new `POST /ai/chat/stream`, both calling `run_turn` — `/ai/chat` is a public bearer-usable contract in `openapi.json` — cost if wrong: one extra thin handler.*
- *Ruling: Stop is closing the response body (client `AbortController`), with no stop endpoint or server-side turn registry; the turn runs on a tracked task and a drop guard cancels its token — ties the turn's lifetime to its one consumer without a registry — cost if wrong: a proxy that keeps the upstream open after the browser aborts delays the stop until the next failed send.*
- *Ruling: cancellation only at safe points (LLM call, top of each tool round, before the final summary), never inside a running tool round — a stop must not leave a half-done write — cost if wrong: Stop can take as long as the running tool round, and the UI says "Stopping…" until the stored history arrives; the only mid-tool drops are the timeout paths: the streaming endpoint's 180 s limit after its 5 s grace, and `/ai/chat`'s own timeout, which drops `run_turn` at any await.*
- *Ruling: at most 2 concurrent streaming turns per account (`429` before opening), no requests-per-window limit — nothing limited `/ai/chat` before and an in-process counter suffices for one instance — cost if wrong: one more limiter later; the count is per process.*
- *Ruling: nullable `ai_messages.tool_outcome` column written with each tool row, instead of parsing result text — keeps backend result formats out of the frontend — cost if wrong: rows from before migration `0012` carry no outcome and render as neutral steps.*
- *Ruling: the live turn outlives navigation (owner's decision): the stream reader, its `AbortController` and the parts live in a module-level store under `frontend/src/modules/ai/`; only Stop or closing the tab stops a turn, one live turn per tab — cost if wrong: a turn the user forgot about keeps running until it ends or hits 180 s.*
- *Ruling: while a live turn exists the AI view shows only the stored rows it had when the turn began (`LiveTurn.baseline`, empty for a new or different conversation), and the stored rows take over only when the store drops the turn, after `finish()` fetched them — the server stores the prompt, tool rows and tool-round preamble text (`turn.rs`) mid-turn, so neither a row count nor "an assistant row appeared" can tell the final reply has arrived — cost if wrong: a turn whose history was not loaded when it started hides that history until the turn ends.*
- *Ruling: the HTTP prompt cap stays 4000 bytes on both chat endpoints while `llm::MAX_PROMPT_LENGTH` is 16384 — both pre-existing and out of scope here — cost if wrong: CJK prompts are refused near 1,333 characters because the frontend counts characters; fix as a separate change.*

### Rulings (2026-10-18 — Todo tags, recurrence and steps)

- *Ruling: `PATCH /todos/:id` and `set_completed` share one locked completion path in `TodoService` (row `FOR UPDATE`, flip, spawn successor, publish the event in the same transaction, then commit) — a second path would double-spawn or skip the event on done→undo→done — cost if wrong: duplicate or missing recurring successors.*
- *Ruling: `todos.recurrence_anchor` stores the schedule's anchor date, set when recurrence turns on or `due_at` changes on a recurring todo and cleared on `none`; the successor is computed from it, strictly after now — late completion never drifts the schedule — cost if wrong: recurring todos drift to the completion date.*
- *Ruling: `next_todo_id` on the completed row is the spawn-once guard and is returned on the PATCH response; it names the successor, which itself arrives with the next list fetch or SSE refresh — cost if wrong: the web UI cannot jump straight to the successor without a list refresh.*
- *Ruling: the new `TodoItem` fields (`tags`, `recurrence`, `recurrence_anchor`, `next_todo_id`) carry `#[serde(default)]` — queued jobs, outbox events and clients serialized before `0013` must still deserialize — cost if wrong: in-flight payloads fail to parse across the deploy.*
- *Ruling: tag add/remove from chat goes through the locked `TodoService::edit_tags(owner, id, add, remove)`, never an interface-layer read-modify-write — AGENTS §4.3 outranks the spec's interface-layer wording, and concurrent edits across channels otherwise lose tags — cost if wrong: one extra service method.*
- *Ruling: chat `!todo ... due <duration>` is relative-only (`2d`, `3h`); absolute dates stay on REST/MCP/AI tools — the chat grammar stays unambiguous across time zones — cost if wrong: users must use the web UI for a fixed date.*
- *Ruling: a missing step and another owner's todo or step both return `AppError::NotFound` (404), never 400 or 403 — spec §6.2 and no ownership oracle — cost if wrong: HTTP clients see 400 for a missing step.*
- *Ruling: tags share one rule, `domain::tags::Tag` (Unicode letters/digits, `_`, `-`, at most 50 chars, lowercased), used by notes and todos and (de)serialized via `try_from`/`into` String so no stored or wire value bypasses `Tag::parse` — widening is backward compatible for existing note tags — cost if wrong: note and todo tag rules diverge.*

### Rulings (2026-10-18 — Finance budgets)

- *Ruling: inline budget feedback comes from `FinanceService::record_transaction` returning `RecordedTransaction { transaction, budgets }`; `create_transaction` and `create_transaction_in_tx` keep their signatures and drop the statuses — keeps recurring bills, portfolio cash, OCR and REST callers untouched — cost if wrong: a caller that later wants statuses switches to `record_transaction`.*
- *Ruling: account merge step 1b deletes `from`'s budgets whose remapped scope collides with one of `into`'s, then remaps the rest (`reconcile_colliding_owned_rows`, right after the transactions remap) — a single `UPDATE … NOT EXISTS` moved a colliding budget across still pointing at `from`'s duplicate category; the surviving account's budget wins either way — cost if wrong: none.*
- *Ruling: account merge step 1b also deletes `from`'s own budgets that would remap onto one `into` category (case-variant names such as "Food" and "food", same currency), keeping the lowest id — the remap otherwise violated `idx_finance_budgets_scope` and rolled the whole merge back — cost if wrong: the higher-id duplicate budget's limit is dropped.*
- *Ruling: the DM loop and owner-readiness check in `interface/jobs/portfolio.rs` moved to a shared `interface/jobs/dm.rs` used by price and budget alerts — second real use — cost if wrong: none.*
- *Ruling: budget tools live in `llm/tools/budgets.rs`, wired into `executor.rs` and `definitions.rs`, because `finance.rs` was already over 500 lines — cost if wrong: none.*
- *Ruling: the briefing builds `FinanceService::new(pool.clone())` and calls `budget_status` instead of reading repositories, so the status rule lives once — cost if wrong: none.*
- *Ruling: `Month` serialises as the string `"YYYY-MM"` everywhere (events, REST, job payload) — cost if wrong: none.*
- *Ruling: evaluation locks the matching budget rows `FOR UPDATE` (ordered by id; `lock_matching`) before summing month spend — under READ COMMITTED a second concurrent expense then re-reads after the first commits, so a crossing split across concurrent writes is never missed — cost if wrong: concurrent expenses in one owner and currency serialise for one transaction.*
- *Ruling: one `BudgetThresholdCrossed` event and one `finance.budget_alert` job per budget per write, carrying the highest newly inserted threshold — consistent with "only the 100 % alert is enqueued" when one write jumps past both — cost if wrong: an SSE listener never sees the 80 % event when a single write jumps past 100 %.*
- *Ruling: the migration was written as `0012_finance_budgets.sql` (migrations ended at `0011` on the branch) and renumbered to `0014_finance_budgets.sql` when merging into master, which had by then taken `0012_ai_message_tool_outcome` and `0013_todo_tags_recurrence_steps` — numbering follows the merge order — cost if wrong: none once merged.*
- *Ruling: `alerts_enabled` is `Option<bool>` end to end (domain `NewBudget`, service, chat, REST, AI tools): `None` keeps the stored value on update and means `true` on create — `!budget set` must never silently re-enable alerts the user turned off — cost if wrong: a caller wanting "always set" passes `Some(true)`.*
- *Ruling: the web panel says "Limit reached" at exactly 100 % (state `exceeded` begins at the limit; `remaining_minor` is 0 there) — the alert fires at 100 % and spend has not yet gone past the limit — cost if wrong: wording only.*

### Rulings (2026-10-31 — Feature blueprints, Phase A)

- *Ruling: bookmark, footprint and journal deletes drop their `knowledge_embeddings` rows in the same transaction as the row delete; there is no sweep job for legacy orphans (retrieval already filters hits whose item is gone) — cost if wrong: pre-fix orphans linger until their owner re-indexes.*
- *Ruling: footprint create/update and day-log upsert enqueue `knowledge_embed_item_versioned` keyed by `updated_at`; `update_time_allocation` and bookmark edits do not (time allocation is not embedded; bookmark edits are outside the blueprint) — cost if wrong: an edited bookmark keeps stale embeddings until the hourly backfill.*
- *Ruling: `life_repo::delete_day_log` returns `Option<i64>` (the deleted id) so the service can purge embeddings — cost if wrong: none.*
- *Ruling: manual agent task trigger reuses `SystemEvent::AgentRunStarted` instead of the blueprint's new `AgentRunCreated`; only `AgentTaskUpdated` and `AgentTaskDeleted` are new — cost if wrong: none.*
- *Ruling: `set_scheduled_task_state` returns the updated row and takes `next_run_at: Option<_>` (`None` on pause keeps the schedule); resume passes `CronSchedule::next_after(now)` — cost if wrong: none.*
- *Ruling: a paused task can still be run by hand, and a manual run does not move `next_run_at`/`last_run_at` — cost if wrong: the card's "Last run" ignores manual runs.*
- *Ruling: `POST /agent/tasks/{id}/trigger` answers 200 like `POST /agent/runs`, not the blueprint's 201 — cost if wrong: clients expecting 201.*
- *Ruling: Edit & Resend truncates (user decision 2026-10-31): `POST /ai/conversations/{id}/truncate` deletes the edited message and everything after it, then the new prompt is sent; no `parent_message_id`/branching — cost if wrong: the replaced replies are gone, not kept as a branch.*
- *Ruling: a conversation's first reply sets a heuristic title (`title_from_prompt`, ≤ 50 characters at a word boundary) instead of the blueprint's LLM `ai.generate_title` job — no new job type or provider call; a user title is never overwritten — cost if wrong: titles are plain prompt prefixes.*
- *Ruling: conversation create, rename, auto-title and truncate publish the existing `AiConversationUpdated` rather than a new `AiConversationTitleUpdated` — cost if wrong: none.*
- *Ruling: no `effective_model_name` on `GET /account/ai-config`: `model_name` already resolves to the instance default when the user has none, so only the SPA's hardcoded `gpt-4o-mini` fallback went — cost if wrong: none.*
- *Ruling: web "New Chat" creates an empty conversation (a turn without an id resumes the room's latest one for an hour) and is a no-op while the active thread is empty; deleting a thread is a separate, confirmed button — cost if wrong: none.*

### Rulings (2026-10-31 — Feature blueprints, Phase B: receipt viewer)

- *Ruling: the receipt-key ownership rule (`receipt_key_belongs_to`: caller's own account, `receipts` category) is enforced in `FinanceService` for create and `link_receipt`, so chat, jobs and REST all get it; violations are 400 — a foreign key would otherwise let one account presign another's file — cost if wrong: none.*
- *Ruling: `detach_receipt` and `receipt-url` answer 404 both for another owner's transaction and for one without a receipt — no ownership oracle — cost if wrong: clients cannot tell the two apart.*
- *Ruling: `POST /finance/receipts/upload` decides the type from magic bytes (JPEG/PNG/WebP/PDF, 10 MiB), stores first and OCRs images best-effort; PDFs are stored without OCR; oversize is 400 (no 413 constructor) — a failed OCR must not lose the file — cost if wrong: PDF receipts need manual entry.*
- *Ruling: detaching leaves the stored file for the storage retention sweep; no `ReceiptAttached/Detached` events, `TransactionUpdated` carries the row — cost if wrong: orphan files linger until swept.*
- *Ruling: `/ocr/scan` is unchanged; the OCR call moved into shared `ocr::run_ocr` — cost if wrong: none.*

### Rulings (2026-10-31 — Feature blueprints, Phase B: email receipts)

Plan: `plans/2026-10-31-email-receipt-ingestion.md`. Status: done and verified; live IMAP/SMTP untested (no external service is contacted from tests).

- *Ruling: PDF receipts are rasterised inside `inference/` (`ocr/pdf.rs`, pure-Rust `hayro`, first 3 pages, longest side ≤ 2000 px, scale ≤ 4×, so a hostile `MediaBox` or page count costs the same as one receipt) and the page texts are joined; the backend only stopped skipping PDFs, the job and `OcrClient` are unchanged; an unreadable or encrypted PDF is a 400 → "could not be decoded" — cost if wrong: scanned-PDF quality depends on the rasteriser, and a new dependency tree in `inference/`.* (Supersedes the earlier ruling that skipped PDFs.)
- *Ruling: an attachment's type comes from magic bytes (`domain::finance::receipt::sniff_receipt_type`, moved out of `interface/http` so both interfaces share it), never the declared MIME type — cost if wrong: none.*
- *Ruling: an email with any attachment part is a receipt submission; its body is the caption and is never dispatched to a module or the AI, and an empty body is accepted — otherwise a forwarded receipt would start an AI chat — cost if wrong: a command sent with an attachment is not run.*
- *Ruling: parts with a `Content-ID` (images embedded in the HTML body) are ignored silently; at most 5 receipts per message, 10 MiB each, 30 MiB total, so one email cannot fan out into hundreds of OCR jobs — cost if wrong: a sixth receipt in one email needs a resend.*
- *Ruling: job key is `email_receipt:{message_id}:{index}`; a fully duplicate message (IMAP re-fetch after a restart clears `RecentMessages`) stores an orphan object for the retention sweep but queues nothing and sends no second reply — cost if wrong: none.*
- *Ruling: no "Email" origin badge on the web ledger — `source_kind` models only recurring bills and portfolio cash, so a badge needs a new source kind, a migration and an OpenAPI change; the Blueprint 5 paperclip already opens the receipt — cost if wrong: emailed expenses are not visually distinguishable.*
- *Pre-existing, not changed: `ProcessReceiptHandler` parses the OCR amount through `f64` before converting to minor units, against the integer-money rule; it is shared with Matrix and Discord and wants its own fix.*

### Rulings (2026-10-31 — Feature blueprints, Phase B: calendar feed)

Plan: `plans/2026-10-31-calendar-feed.md`. Status: done and verified against the repository, service, handler and Playwright suites; no real calendar client was pointed at it.

- *Ruling: bill UID is `bill-{id}@matrix-bot`, not the spec's `bill-{id}-{due_date}` — the entry carries an `RRULE`, and a UID that changes whenever `next_due_at` advances would leave every past cycle behind as a duplicate — cost if wrong: none.*
- *Ruling: `account_calendar_tokens` is keyed by `account_id` and listed in `ACCOUNT_ID_DISCARDED`, not `OWNED_TABLES` — `OWNED_TABLES` entries carry `owner_id`, and a merge should drop the absorbed account's link rather than move it — cost if wrong: none.*
- *Ruling: mint/revoke are ordinary `AuthUser` routes with no `SystemEvent` (same class as MCP tokens), listed in the step-up test's `ORDINARY` list so that stays deliberate — cost if wrong: a stolen session can mint a link that exposes the agenda, which it could already read through `/todos`.*
- *Ruling: `last_accessed_at` is touched in the resolving statement, at most hourly, instead of the spec's asynchronous update (a detached task is forbidden) — cost if wrong: coarser "last used" timestamp.*
- *Ruling: the token lookup is a hash equality in SQL, not `subtle` constant-time comparison — the token is 256 bits of entropy and only its SHA-256 is compared — cost if wrong: none.*
- *Ruling: the Settings UI lives in the former MCP tab, relabelled "Integrations", because a sixth tab overflows the dialog — cost if wrong: one extra click to find MCP.*
- *Ruling: recurring reminders get an `RRULE`; todos do not (a recurring todo's successor appears as its own row once the current one is completed) — cost if wrong: a recurring todo shows only its next due date.*

### Rulings (2026-10-31 — Feature blueprints, Phase B: audio ingress parity)

Plan: `plans/2026-10-31-audio-ingress-parity.md`. Status: done and verified; live Telegram/Discord downloads untested (no external service is contacted from tests). The gateway routing in `discord/gateway.rs` is not unit-tested (serenity handler).

- *Ruling: parity with Matrix, not the spec's transcript routing — a caption starting `/note` or `!note` saves a note, anything else runs the AI turn with the user's tools; the spec's prefix-in-transcript and command-prefix matching were not built — cost if wrong: a user who wants a note must say so in the caption.*
- *Ruling: direct chats only (Discord already ignored guilds; Telegram groups are ignored because a voice note cannot mention the bot) — cost if wrong: group voice notes are silently ignored.*
- *Ruling: Matrix keeps its own `attachments.rs` flow (stage indicators make it a refactor with no user value here); only `capture_voice_note` gained a `source_channel` argument — cost if wrong: a small known duplication of the note-vs-turn branch.*
- *Ruling: rate limit reuses `interface::http::rate_limit::InMemoryRateLimiter` (fixed window, 5 per 60 s per account) — cost if wrong: a burst straddling a window boundary allows up to 10.*
- *Ruling: no frontend change and no Settings STT badge — `NoteCard` already shows `source_channel`, and voice notes carry the `voice` tag — cost if wrong: STT health is not visible in Settings.*
- *Ruling: `SttUnavailable`/`TranscriptionFailed` are not domain errors; `process_audio` already turns every `SttServiceError` into a reply — cost if wrong: none.*

### Rulings (2026-10-31 — Feature blueprints, Phase C: note wiki-links)

Plan: `plans/2026-10-31-note-wiki-links.md`. Status: done; backend service/parser tests and Playwright pass. Not covered: no HTTP-level test for the two new routes (handlers are thin over `NoteService`), and unsaved-draft switching in the editor (below).

- *Ruling: migration is `0017_knowledge_note_links.sql` (the spec's 0018 was stale), with a unique index on `(source_note_id, LOWER(target_title))`; `knowledge_note_links` is in `OWNED_TABLES` — cost if wrong: none.*
- *Ruling: links are derived data, rewritten by `NoteService` (`application/knowledge/links.rs::sync_links`) in the same transaction as create, update and append; no new `SystemEvent` — `NoteCreated`/`NoteUpdated` already fire — cost if wrong: SSE listeners cannot tell a link change from another edit.*
- *Ruling: titles resolve case-insensitively within the owner; duplicate titles resolve to the lowest id; a link to the note itself (by id or own title) is dropped; dangling links keep `target_note_id NULL` and resolve when a note is created or renamed into that title (`resolve_dangling`), while renaming a target away re-resolves its inbound links (`re_resolve_stale`) — cost if wrong: with duplicate titles a link may bind to the older note.*
- *Ruling: deleting a target leaves inbound links dangling (`ON DELETE SET NULL`) and is not re-resolved to a same-titled duplicate — cost if wrong: such a link stays dangling until its source is next edited.*
- *Ruling: the parser (`domain/knowledge/links.rs`) is a hand-written line scanner, ignores fenced and inline code, caps at 50 distinct targets and 200-character titles — cost if wrong: exotic Markdown (indented code blocks) still yields links.*
- *Ruling: the graph returns non-archived notes only, newest 500, and edges between returned nodes; links from or to archived notes still exist in the table and show in `/notes/{id}/links` — cost if wrong: an archived note's backlinks panel lists notes the graph hides.*
- *Ruling: no new graph dependency; `frontend/src/modules/knowledge/graphLayout.ts` is a deterministic Fruchterman-Reingold layout drawn as SVG with zoom buttons and a title highlight filter (no pan/drag) — cost if wrong: dense graphs are less explorable than with d3.*
- *Ruling: the editor's links panel and `[[` rendering describe the saved note; opening another note from it replaces the draft without a prompt — cost if wrong: unsaved edits are lost on that click. Follow-up: guard on a dirty draft.*

---

### Rulings (2026-10-31 — Feature blueprints, Phase C: briefing podcast)

Plan: `plans/2026-10-31-briefing-podcast.md`. Status: done; fmt, check, clippy `-D warnings`, `sqlx prepare --check`, the full `matrix-bot` suite (2,061 tests against a throwaway Postgres 18), the frontend build and Playwright (briefing-audio, knowledge, navigation, screens) pass. Not run: `live_postgres`, the release build, and the inference crate (untouched). Not covered: no live TTS run (tests use a mock server), no player test of real Ogg playback in Playwright (the fake serves no media).

- *Ruling: there is no `briefing_sent_days` table (the sent day is `user_settings.last_briefing_date`), so migration `0018_briefing_audio.sql` creates `briefing_audio (owner_id, local_date, storage_key)`, unique per owner and day, and it is in `OWNED_TABLES`; on an account merge `into`'s recording wins and `from`'s object is orphaned — cost if wrong: one orphaned file per colliding day.*
- *Ruling: the briefing is template-built, so the spoken script is built from the same data by `application/briefing/audio_script.rs` (Chinese, like the text), capped at 2,000 characters by `domain::briefing::BriefingAudioScript`; it rides on `BriefingPayload.audio_script` and is not in the REST payload or OpenAPI — cost if wrong: none, the contract is unchanged.*
- *Ruling: only the scheduled `briefing.send` job synthesizes; the on-demand trigger and `!briefing` stay text-only — cost if wrong: no audio for manual runs.*
- *Ruling: audio is fail-open end to end (`interface/jobs/briefing_audio.rs`): no TTS, a TTS error, a storage error or a failed row write each only drop the audio (or the record, deleting the unreferenced object); the text is sent and the day recorded as before — cost if wrong: a silent day without audio gives only a log warning.*
- *Ruling: the spec's "storage sweep" does not exist, so the owner's own daily briefing job deletes that owner's recordings older than 30 days (`BriefingService::prune_audio`); an owner whose briefing is switched off keeps old recordings — cost if wrong: unbounded files for owners who stop briefings; a general sweep would fix it.*
- *Ruling: a retried job (channel deferral) re-synthesizes; `record_audio` returns the replaced key and the job deletes that object — cost if wrong: extra TTS cost on retries.*
- *Ruling: `GET /api/v1/briefing/audio?date=` presigns for 3,600 seconds, returns 404 when there is no recording and re-checks that the key belongs to the caller; the dashboard `BriefingCard` hides itself on 404 and asks for today's date in the user's saved zone — cost if wrong: a briefing sent after midnight in that zone is looked up under the new day only when the job's local date matches.*
- *Ruling: the spec's waveform and archive list are not built (plain `<audio controls>`, speed buttons, download link) — cost if wrong: no past-episode browsing, though the endpoint accepts any date.*

---

## 6. Verification

The verification gates are listed once, in [AGENTS.md §8](../../AGENTS.md#8-verification). Run them from the repository root.

A fresh local test PostgreSQL server needs the `vector` and `pg_trgm` extensions created once in `public` before parallel tests run: `CREATE EXTENSION IF NOT EXISTS vector SCHEMA public; CREATE EXTENSION IF NOT EXISTS pg_trgm SCHEMA public;`. Database setup is in [`docs/database.md`](../database.md#test-databases).

### Rulings (2026-11-01 — Blueprint 9: command palette)

- *Ruling: instant search uses the existing `POST /knowledge/search` (200 ms debounce), not the blueprint's nonexistent `GET /api/v1/search/hybrid` — cost if wrong: none; the endpoint already spans notes, bookmarks, journal and footprints.*
- *Ruling: navigation entries reuse the Sidebar's exported `NAV_GROUPS`; quick actions reuse the existing autofocus jump (`onQuickNavigate`) rather than opening Add modals from outside their modules — cost if wrong: one extra click to submit.*
- *Ruling: the palette arms only when signed in; no unit-test runner was added, the pure matcher is covered by `e2e/command-palette-match.spec.ts` — cost if wrong: none.*
