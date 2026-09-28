# Feature survey: modules/ -> domain/application/infrastructure layering

Scope: `backend/src/modules/<feature>/`, `backend/src/accounts/`, `backend/src/db/rooms.rs`,
`backend/src/events.rs`. Excludes `main.rs`, `app.rs`, `interface/http/mod.rs` (concurrent edits).
Schema reference: `backend/migrations/0001_baseline.sql`. Macros: `backend/src/db/executor.rs`.

Legend for macro counts: db_execute / db_fetch_one / db_fetch_optional / db_fetch_all /
db_fetch_*_dyn / db_scalar_* / tx_* (tx_execute, tx_fetch_*, tx_scalar_*).

---

## account (`modules/account/mod.rs`)
1. Files: `mod.rs` 702. Total 702.
2. Macros: db_execute 5. No fetch/scalar, no tx_*, no dyn. No raw `sqlx::query*`.
3. Tables: `matrix_links`, `webauthn_credentials`, `recovery_requests`, `passkey_enrollment_tokens`
   (via `accounts::linking`/`recovery`/`credentials`, not owned by this module's own SQL - account/mod.rs
   itself mostly orchestrates `crate::accounts::*` repositories rather than issuing SQL). Cross-feature:
   heavily dependent on `accounts` (session/credentials/linking/recovery), not the other way around.
4. No `FromRow` structs of its own (delegates to `accounts::*` types).
5. Business rules: `WEB_ONLY` command allow-list (Matrix can't do `!export`/`!sessions`/`!recoverycodes`/
   `!unlink`) at top of file; `!passkey add` first-passkey-only enrollment enforced jointly with
   `EnrollmentRepository::validate_and_consume` (in `accounts/`).
6. Adapters: HTTP under `interface/http/account/` (separate subdir - verify at migration time), Matrix
   handler is `account/mod.rs` itself (`BotModule` at line 62), no AI tool file, no MCP tool (account
   actions aren't exposed to the LLM).
7. Transactions: none directly (delegates to `accounts` repos, see below).
8. Depends on: `crate::accounts` (heavy). Nothing in `modules/` depends on `account`.
9. Tests: 11, `#[tokio::test]` against `test_pool()` - repository/handler-level, DB-backed.
10. Type mismatches: none found (delegates persistence to `accounts::*`).

## accounts (`backend/src/accounts/*`, not under `modules/` but the shared identity substrate)
1. Files: credentials.rs 438, linking.rs 536, merge.rs 1266, mod.rs 321, principal.rs 182,
   recovery.rs 563, repository.rs 275, security_events.rs 251, session.rs 468. Total 4300.
2. Macros: db_execute 68, db_fetch_all 6, db_fetch_optional 4, db_scalar_optional 4, tx_execute 14.
   No `_dyn` macros, but `merge.rs` builds table/column names into SQL text via `format!` and executes
   them through `db_scalar_one!`/`db_execute!`/`tx_execute!` - a different kind of "dynamic SQL" than the
   allow-listed `_dyn` macros (values are still bound as `?` params; only identifiers are interpolated,
   and only from the compile-time `OWNED_TABLES` constant).
   Dynamic-SQL sites (identifier interpolation, all in `merge.rs`): lines 167, 185, 233, 247, 302, 312,
   338, 346, 365, 682, 694, 747, 1203 (`format!("... {table} ...")` / `{table}`/`{column}` in SQL text).
3. Tables: virtually all owner-keyed tables in the schema - the full `OWNED_TABLES` list (see below) plus
   auth-only tables `matrix_links`, `webauthn_credentials`, `security_events`, `account_rooms`, `sessions`,
   `recovery_codes`, `recovery_requests`, `mcp_tokens`, `passkey_enrollment_tokens`,
   `matrix_link_challenges`, and `accounts` itself. This is the single most cross-cutting file in the
   codebase by table coupling.
4. FromRow structs: `credentials.rs` (passkey/credential row), `linking.rs` (link row), `mod.rs` (Account,
   AccountStatus row), `security_events.rs` (SecurityEvent - has `ToSchema`, so persistence == transport
   here too), `session.rs` (session row). Only `security_events.rs`'s struct carries `ToSchema`; the
   others are internal-only structs, not serialized directly over HTTP (HTTP layer maps them).
5. Business rules: `OWNED_TABLES` constant (`accounts/mod.rs:63-121`) is the authoritative list of every
   owner-keyed table a merge must rewrite - "any new owned table must be added here or a merge silently
   drops its rows" (also stated in AGENTS.md). Merge savepoint/rollback logic in `merge.rs:164-371`.
   First-passkey-only enrollment enforced in `credentials.rs`/`account/mod.rs`. Time-fenced recovery in
   `recovery.rs`. Step-up/verified-session logic in `session.rs`/`principal.rs`.
6. Adapters: HTTP under `interface/http/account/` and `interface/http/auth/` (confirm exact split at
   migration time). No AI tool, no MCP tool - identity/auth is never LLM- or MCP-exposed. No scheduler.
7. Transactions: `merge.rs:270` (`pool.begin()`) wraps the whole account merge (moving every owned table's
   rows, deleting the losing account) - protects against a merge partially applying if interrupted midway
   (uses nested SAVEPOINTs per-table and per-row inside that outer transaction for granular rollback,
   `merge.rs:164-371`). `db/rooms.rs:51` (separate feature, see below) is unrelated. Three
   `pool.begin().await.unwrap()` in `merge.rs` are test-only (lines 1032, 1170, 1233).
8. Depends on: nothing in `modules/` (this is the shared substrate `modules/*` depend on, not vice versa).
9. Tests: 79, `#[tokio::test]` against `test_pool()` - repository-level DB tests (merge scenarios, session
   lifecycle, credential CRUD).
10. Type mismatches: none found in the FromRow structs read directly.

**Migration-planning note on `accounts::merge.rs`**: this file cannot be mechanically rewritten to
`query!` one call at a time the way other repositories can - it iterates `OWNED_TABLES` at runtime and
builds `SELECT/DELETE/UPDATE ... FROM {table} WHERE {column} = ?` for an arbitrary table name, exactly the
shape `sqlx::query!` cannot support (it needs the full SQL text at compile time). This needs either (a)
staying on `db_execute!`/`tx_execute!` behind `AssertSqlSafe` permanently, with the allow-listed-identifier
discipline AGENTS.md documents, or (b) being restructured into one compile-checked arm per owned table (a
big match/dispatch, ~20+ arms). Treat this as its own migration task, not a byproduct of any single
feature's move.

## ai (`modules/ai/*`)
1. Files: alert.rs 129, config_cmd.rs 194, error.rs 59, handler.rs 1209, mod.rs 117, models.rs 372,
   prompt.rs 146, repository.rs 406, resolved_config.rs 299, retrospective.rs 592, search.rs 500,
   stream.rs 65, plus `tools/`: bills.rs 286, bookmarks.rs 255, definitions.rs 21, executor.rs 116,
   finance.rs 431, habits.rs 250, life.rs 416, mod.rs 666, portfolio/{alerts.rs 214, cash.rs 142,
   mod.rs 430, net_worth.rs 53}, reminder.rs 378, settings.rs 94, todo.rs 399, webhooks.rs 76.
   Total (ai/*.rs + tools/**): ~7250 lines - the largest feature by far, and `handler.rs` (1209) already
   exceeds the 500-line file-size rule (pre-existing debt, worth flagging separately).
2. Macros (ai/ core, excludes tools/*, which call other features' repositories instead of issuing their
   own SQL): db_execute 5, db_fetch_all 2, db_fetch_one 1, db_fetch_optional 3. No tx_*, no `_dyn`, no raw
   `sqlx::query*`.
3. Tables: `ai_conversations`, `ai_messages`, `ai_user_configs` (own); the `tools/*` files touch every
   other feature's tables directly (bills -> `recurring_bills`; bookmarks -> `bookmarks`; finance ->
   `transactions`/`finance_categories`; habits -> `habits`/`habit_logs`; life ->
   `life_profiles`/`life_day_logs`; portfolio -> `portfolio_*`; reminder -> `reminders`; settings ->
   `user_settings`; todo -> `todos`; webhooks -> `webhooks`/`webhook_outbox`). This makes `ai` the
   broadest cross-feature-coupled module after `accounts`.
4. FromRow: `models.rs` (`AiUserConfig`, `AiConversation`, `AiMessage` - all also `ToSchema`, so
   persistence == transport for all three).
5. Business rules: SSRF guard for per-user AI base URLs (`models.rs`, `validate_ai_base_url`, private-host
   checks starting ~line 47), 500-char caps on API key/model name (`models.rs:159-201`), max-history-turns
   bounds (1-100), web-search intent gating (`search.rs`, `should_enable_web_search`), tool-call
   concurrency via `join_all` (documented in AGENTS.md, implemented in `handler.rs`).
6. Adapters: HTTP `interface/http/ai.rs` (9 handlers), Matrix handler `ai/handler.rs` (1209 lines - the
   catch-all conversational entry point), AI tool files themselves ARE `modules/ai/tools/*.rs` (this
   feature *is* the tool-execution layer for every other feature), MCP surface is a thin pass-through
   (`modules/mcp/adapter.rs` calls `ai::tools::executor::execute` and
   `ai::tools::definitions::all_tool_definitions` directly) - i.e. **MCP tools and AI tools are the same
   code path**, not two adapters to convert separately. No scheduler of its own.
7. Transactions: none (`ai` core has no `begin()`/`tx_*!`).
8. Depends on (`crate::modules::X` from `ai`): bookmarks, finance, footprints, habits, life, portfolio,
   recurring_bills, reminder, settings, todo, webhook - i.e. everything. `ai` is a downstream consumer of
   nearly every other feature's repository, so it should migrate **after** the features its tools call, or
   the tool layer needs a temporary shim calling old-repo-shaped functions during the transition.
9. Tests: 50, `#[tokio::test]`, DB-backed.
10. Type mismatches: none found in `models.rs` (`updated_at: String` correctly matches
    `ai_user_configs.updated_at TEXT`, `ai_conversations`/`ai_messages` `created_at`/`updated_at TEXT`).

## bookmarks (`modules/bookmarks/*`)
1. Files: handler.rs 325, metadata.rs 516 (**already over the 500-line cap**), mod.rs 105, models.rs 65,
   repository.rs 461. Total 1472.
2. Macros: db_execute 1, db_fetch_all 2, db_fetch_all_dyn 2, db_fetch_one 8, db_fetch_optional 1.
   Dynamic-SQL sites: `repository.rs:143` (`db_fetch_all_dyn!` for filtered listing), `repository.rs:205`
   (`db_fetch_one_dyn!` for the matching count) - both build a `WHERE` clause with `push_str` but bind
   every value, per the AGENTS.md-documented safe pattern.
3. Tables: `bookmarks` only (leaf feature, no FK-level coupling to other feature tables).
4. FromRow: `models.rs::Bookmark` - also `ToSchema`, persistence == transport.
5. Business rules: 500-char tag-length cap (`handler.rs:42-43`).
6. Adapters: HTTP `interface/http/bookmarks.rs` (9 handlers), Matrix handler `bookmarks/handler.rs`, AI
   tool `modules/ai/tools/bookmarks.rs`, reachable via MCP through the shared `ai::tools` executor
   (see `ai` section), no scheduler.
7. Transactions: none.
8. Depends on: nothing in `modules/` (leaf). `ai` depends on it.
9. Tests: 8, DB-backed.
10. Type mismatches: none (`created_at`/`updated_at: String` matches `bookmarks.created_at/updated_at TEXT`).

## briefing (`modules/briefing/*`)
1. Files: generator.rs 307, handler.rs 222, mod.rs 82, models.rs 33, scheduler.rs 154, ticker.rs 254.
   Total 1052.
2. Macros: none directly - briefing is a pure aggregator/renderer over other features' repositories
   (reads via `crate::modules::{habits,portfolio,recurring_bills,settings,todo}`), so it has no SQL of
   its own to convert; it will migrate almost for free once its dependencies expose application-service
   APIs.
3. Tables: none directly; reads `accounts` (owner iteration) transitively and every dependency's tables
   indirectly through their repositories.
4. FromRow: none in `models.rs` (plain DTOs for the rendered briefing).
5. Business rules: briefing composition/ordering logic in `generator.rs`; scheduling window/dedupe in
   `scheduler.rs`/`ticker.rs`.
6. Adapters: HTTP `interface/http/briefing.rs` (2 handlers), Matrix handler `briefing/handler.rs`, no AI
   tool, no MCP tool, background scheduler `briefing/scheduler.rs` + `ticker.rs` (per-user daily send).
7. Transactions: none.
8. Depends on: habits, portfolio, recurring_bills, settings, todo.
9. Tests: 8, mixed - some are pure (formatting), most exercise the aggregation against `test_pool()`.
10. Type mismatches: N/A (no direct persistence types).

## core (`modules/core/*`)
1. Files: help.rs 50, mod.rs 123. Total 173.
2. Macros: none (help/command-listing only, no persistence).
3. Tables: none.
4. FromRow: none.
5. Business rules: none (static help text/command registry).
6. Adapters: Matrix handler only (`core/mod.rs`, `BotModule`); no HTTP, no AI tool, no MCP tool, no
   scheduler.
7. Transactions: none.
8. Depends on: nothing.
9. Tests: 2, pure.
10. N/A.

## finance (`modules/finance/*`)
1. Files: analytics.rs 476, fx.rs 233, handler.rs 632 (**over cap**), mod.rs 326, models.rs 139,
   money.rs 259, net_worth.rs 270, repository.rs 624 (**over cap**), service.rs 554 (**over cap**).
   Total 3513.
2. Macros: db_execute 8, db_fetch_all 10, db_fetch_one 3, db_fetch_optional 5, db_scalar_optional 1.
   No tx_*. Dynamic-SQL: `repository.rs:218` and `:242` build a `WHERE`-clause `format!` string
   (category/date filters) passed to `db_fetch_all!`/`db_scalar_optional!` - same bind-everything pattern,
   not a `_dyn` macro but functionally identical risk profile; verify at conversion time.
3. Tables: `finance_categories`, `transactions`, `finance_shares` (own); reads `portfolio_cash_balances`/
   `portfolio_holdings` (via `portfolio`) and `user_settings` (via `settings`) for net-worth aggregation.
4. FromRow: `models.rs` (`FinanceCategory`, `Transaction` - both `ToSchema`), `analytics.rs`
   (`FinanceShare` - `ToSchema`). Persistence == transport throughout.
5. Business rules: currency/money arithmetic and rounding in `money.rs` (minor-unit conversions, exponent
   handling - a natural domain-layer candidate), FX conversion in `fx.rs`, net-worth aggregation rules in
   `net_worth.rs`, category validation in `service.rs`.
6. Adapters: HTTP `interface/http/finance.rs` (9), `finance_analytics.rs` (6), `finance_net_worth.rs` (1);
   Matrix handler `finance/handler.rs`; AI tool `modules/ai/tools/finance.rs`; MCP via shared executor; no
   scheduler.
7. Transactions: none (no `begin()`/`tx_*!` in this feature despite being money-handling - worth a closer
   look during migration: multi-row category+transaction writes may currently rely on single-statement
   atomicity rather than an explicit transaction).
8. Depends on: portfolio (net worth), settings (currency default).
9. Tests: 44, DB-backed.
10. Type mismatches: none found (BIGINT minor-unit fields as `i64`, `TIMESTAMPTZ` as `DateTime<Utc>`,
    `finance_shares.created_at/updated_at TIMESTAMPTZ` as `Option<DateTime<Utc>>`).

## footprints (`modules/footprints/*`)
1. Files: handler.rs 224, mod.rs 134, models.rs 89, repository.rs 317. Total 764.
2. Macros: db_execute 1, db_fetch_all 1, db_fetch_all_dyn 1, db_fetch_one 2, db_fetch_optional 3.
   Dynamic-SQL: `repository.rs:127` (`db_fetch_all_dyn!`, filtered listing).
3. Tables: `footprints` only (leaf).
4. FromRow: `models.rs::Footprint` - `ToSchema`, persistence == transport.
5. Business rules: 500-char cap on title/note (`handler.rs:126-144`), stay-duration calculation
   (`models.rs::calculate_stay_duration`).
6. Adapters: HTTP `interface/http/footprints.rs` (7), Matrix handler `footprints/handler.rs`; absent from
   `modules/ai/tools/*`, meaning footprints is not currently LLM/MCP-exposed. No scheduler.
7. Transactions: none.
8. Depends on: nothing (leaf). `accounts::merge` and `life` (aggregated dashboard) depend on it.
9. Tests: 2, DB-backed.
10. Type mismatches: none (`created_at`/`updated_at: DateTime<Utc>` matches `TIMESTAMPTZ`).

## habits (`modules/habits/*`)
1. Files: handler.rs 326, mod.rs 81, models.rs 146, repository.rs 350. Total 903.
2. Macros: db_execute 3, db_fetch_all 2, db_fetch_one 1, db_fetch_optional 3, db_scalar_optional 1. No
   dyn, no tx_*.
3. Tables: `habits`, `habit_logs` (own).
4. FromRow: `models.rs::Habit` (ToSchema), `HabitLog` (no ToSchema - internal only), `HeatmapDay`
   (ToSchema). Habit/HeatmapDay share persistence+transport; HabitLog does not leak to HTTP directly.
5. Business rules: streak calculation (`models.rs::calculate_streaks`, pure and already unit-tested - a
   good domain-layer candidate as-is), 500-char cap enforced in `habits/handler.rs`.
6. Adapters: HTTP `interface/http/habits.rs` (7), Matrix handler `habits/handler.rs`, AI tool
   `modules/ai/tools/habits.rs`, MCP via shared executor, no scheduler.
7. Transactions: none.
8. Depends on: nothing (leaf). `briefing`, `ai`, `mcp` depend on it.
9. Tests: 3 in `modules/habits/*` (plus `calculate_streaks`'s own unit test) - DB-backed for repository,
   pure for streak math.
10. Type mismatches: none (`created_at`/`updated_at: Option<String>` matches `habits.created_at/updated_at TEXT`).

## journal (`modules/journal/*`)
1. Files: handler.rs 369, mod.rs 67. Total 436.
2. Macros: none found - journal appears to have no dedicated table/repository of its own; it composes
   `life`/`settings` data (confirm at migration time whether it writes through `life_day_logs` via the
   `life` module's repository).
3. Tables: none directly (through `life`/`settings`).
4. FromRow: none.
5. Business rules: none identified beyond formatting/command parsing in `handler.rs`.
6. Adapters: Matrix handler only (`journal/handler.rs`, `BotModule` at `mod.rs:24`); no HTTP file, no AI
   tool, no MCP tool, no scheduler. Looks like a Matrix-only convenience wrapper over `life`.
7. Transactions: none.
8. Depends on: life, settings.
9. Tests: 1.
10. N/A.

## life (`modules/life/*`)
1. Files: activity.rs 89, activity_collectors.rs 432, handler.rs 258, mod.rs 74, models.rs 88,
   repository.rs 465. Total 1406.
2. Macros: db_execute 2, db_fetch_all 11, db_fetch_one 4, db_fetch_optional 3. No dyn, no tx_*.
3. Tables: `life_profiles`, `life_day_logs` (own); `activity_collectors.rs` reads across
   `bookmarks`, `footprints`, `habit_logs`, `todos`, `transactions`, `reminders`,
   `portfolio_audit_log`, plus its own `activity_counts`/`all_dates`/`user_activities` (these last three
   read like ad-hoc/virtual names from a UNION query rather than real tables - they do not appear in
   `0001_baseline.sql` as table names; verify against the actual SQL text). This makes `life` the second
   broadest cross-table reader after `ai`, entirely for its "activity feed"/dashboard aggregation.
4. FromRow: `models.rs::LifeProfile`, `LifeDayLog`, `LifeHorizonDay` (all `ToSchema`); 7 more
   `#[derive(sqlx::FromRow)]` structs scoped inside `activity_collectors.rs`, internal to that file, not
   `ToSchema` (good separation there already).
5. Business rules: mood normalization (`models.rs::normalize_mood`), age/day-count math for the "life in
   days" horizon view (`repository.rs`/`handler.rs`).
6. Adapters: HTTP `interface/http/life.rs` (9), `life_activity.rs` (2); Matrix handler `life/handler.rs`;
   AI tool `modules/ai/tools/life.rs`; MCP via shared executor; no scheduler.
7. Transactions: none.
8. Depends on: bookmarks, footprints, habits (habit_logs), portfolio (audit log), reminder, todo, finance
   (transactions) - all read-only cross-table SELECTs for the activity feed, not FK relationships. This is
   the module most likely to break if a dependency's table/columns move before `life` is converted, since
   its queries reach directly into other features' tables rather than calling their repositories.
9. Tests: 4, DB-backed.
10. Type mismatches: none in the named structs (`LifeProfile`/`LifeDayLog` timestamps `String` matching
    `TEXT`).

## mcp (`modules/mcp/*`)
1. Files: adapter.rs 66, handler.rs 203, mod.rs 68, models.rs 92, prompts.rs 74, protocol.rs 279,
   repository.rs 170, resources.rs 151, service.rs 263, stdio.rs 60. Total 1426.
2. Macros: db_execute 6, db_fetch_all 1, db_fetch_optional 1. No dyn, no tx_*.
3. Tables: `mcp_tokens` (own), `accounts` (lookup for token owner).
4. FromRow: `models.rs::McpToken` type family - none carry `ToSchema` (MCP has its own protocol
   serialization, separate from the REST `ToSchema` surface - good separation already).
5. Business rules: token hashing/prefix generation, expiry checks (`models.rs::is_expired`).
6. Adapters: HTTP `interface/http/mcp.rs` (6 - token management endpoints, not tool calls); Matrix
   `BotModule` for token-related Matrix commands only (`mod.rs:32`); tool execution is fully delegated to
   `ai::tools` (see `ai` section) - **`mcp` itself has no per-feature tool files to move**, its only
   migration work is the `mcp_tokens` repository. No scheduler.
7. Transactions: none.
8. Depends on: ai (tool definitions/executor), finance/habits/life/portfolio/todo (transitively, only
   through `ai`'s tool executor, not directly).
9. Tests: 12, mixed (protocol/service tests are pure; repository tests are DB-backed).
10. Type mismatches: none found (`created_at`/`last_used_at`/`expires_at: DateTime<Utc>` match
    `mcp_tokens`'s `TIMESTAMPTZ` columns).

## ocr (`modules/ocr/*`)
1. Files: mod.rs 259, models.rs 58, prompt.rs 79. Total 396.
2. Macros: none - pure inference-adapter feature (calls `infrastructure/inference`), no database table.
3. Tables: none.
4. FromRow: none.
5. Business rules: none beyond prompt construction.
6. Adapters: HTTP `interface/http/ocr.rs` (1); imports `ai`, suggesting OCR dispatches through the AI
   conversational flow; no AI tool file, no MCP tool, no scheduler.
7. Transactions: none.
8. Depends on: ai.
9. Tests: 6, pure (prompt formatting).
10. N/A.

## portfolio (`modules/portfolio/*`)
1. Files: alerts.rs 298, alerts_handler.rs 175, audit.rs 555 (**over cap**), cash.rs 308,
   cash_repo.rs 178, handler.rs 324, history.rs 402, history_feed.rs 378, mod.rs 498,
   models.rs 376, price_feed.rs 504 (**over cap**), repository.rs 845 (**well over cap**),
   scheduler.rs 146, service.rs 504 (**over cap**), trade.rs 411, trade_cmd.rs 419, view.rs 136.
   Total 5457 - second-largest feature after `ai`, and the one with the most transactional complexity.
2. Macros: db_execute 11, db_fetch_all 8, db_fetch_one 2, db_fetch_optional 5, db_scalar_optional 1,
   tx_execute 15, tx_fetch_all 1, tx_fetch_one 7, tx_fetch_optional 4. No `_dyn` macros, but heavy use of
   `format!` to interpolate a compile-time column-list constant into SELECT text (not user input):
   `audit.rs:154`, `cash_repo.rs:136`, `trade.rs:79,118,237,356` (all `&format!("SELECT {COLUMNS} FROM
   ... WHERE id = ?")`-shaped, safe but still needs converting to literal `query_as!` strings).
3. Tables: `portfolio_holdings`, `portfolio_cash_balances`, `portfolio_audit_log`,
   `portfolio_price_alerts`, `portfolio_daily_snapshots` (own). Read cross-feature by `finance`
   (net worth), `life` (activity feed via `portfolio_audit_log`), `ai`/`mcp` tools, `briefing`.
4. FromRow: `models.rs` (`Holding`, `CashBalance` - both `ToSchema`), `alerts.rs::PriceAlert`
   (`ToSchema`), `audit.rs::AuditLogEntry` (`ToSchema`), `history.rs::PortfolioSnapshot` (`ToSchema`).
   Every persisted portfolio row is also the HTTP wire type.
5. Business rules - this is the feature the prompt calls out by name: **"cash is a precondition for
   buying, never for selling"** lives in `trade.rs` (`BuyOutcome::InsufficientCash` returned at
   `trade.rs:60` and `:68`, i.e. the buy path checks/decrements cash before touching holdings; the sell
   path, elsewhere in the same file, credits cash without a precondition). Audit-chain hash-linking
   invariant (tamper-evidence) lives in `audit.rs` (prev_hash/entry_hash chain, further enforced by DB
   triggers `prevent_portfolio_audit_update`/`_delete` in the migration itself - a rare case where a
   business invariant is partly enforced in the database, not just application code). 500-char note cap
   in `handler.rs:194-195`.
6. Adapters: HTTP `portfolio.rs` (9), `portfolio_alerts.rs` (4), `portfolio_cash.rs` (3),
   `portfolio_history.rs` (2), `portfolio_prices.rs` (9) - 27 HTTP handlers, the most of any feature;
   Matrix handlers `handler.rs` + `trade_cmd.rs` + `alerts_handler.rs`; AI tools split across
   `modules/ai/tools/portfolio/{alerts.rs,cash.rs,mod.rs,net_worth.rs}`; MCP via shared executor;
   background scheduler `portfolio/scheduler.rs` (price refresh / daily snapshot / alert evaluation, fed
   by `price_feed.rs` and `history_feed.rs`).
7. Transactions - the richest set in the codebase:
   - `trade.rs:28` (`pool.begin()`, buy path) - protects the joint cash-decrement + holding-upsert so a
     buy never partially applies (holding created/updated but cash not deducted, or vice versa).
   - `trade.rs:179` (`pool.begin()`, deposit/cash-adjustment path) - protects cash-balance creation and
     the paired audit-log write.
   - `trade.rs:301` (`pool.begin()`, sell path) - protects holding-decrement + cash-credit + audit write
     as one atomic sell.
   - `audit.rs:186-280` (`tx_scalar_one!`/`tx_execute!`/`tx_fetch_all!` inside a caller-supplied
     connection) - protects the audit-chain rehash used when absorbing one account's audit log into
     another's during a merge (called from `accounts::merge`), keeping the hash chain contiguous.
8. Depends on: finance, settings (currency defaults). Depended on by: ai, mcp, life, briefing, finance,
   accounts (merge + audit absorption).
9. Tests: 79 - largest test suite of any feature, DB-backed (transaction/invariant-heavy, matches the
   business-rule density).
10. Type mismatches: none found; `currency_exponent`, `quantity_minor`, `avg_cost_minor` etc. consistently
    `i64`/`BIGINT`.

## recurring_bills (`modules/recurring_bills/*`)
1. Files: due_reminder.rs 283, handler.rs 274, mod.rs 75, models.rs 84, repository.rs 312. Total 1028.
2. Macros: db_execute 3, db_fetch_all 1, db_fetch_optional 1. No dyn, no tx_*. One
   `sqlx::Error::Protocol(format!(...))` at `repository.rs:227` - error-message formatting, not SQL text,
   harmless.
3. Tables: `recurring_bills` (own), `reminders` (creates linked reminders - see cross-feature note).
4. FromRow: `models.rs::RecurringBill` - `ToSchema`, persistence == transport, including the
   `nullable()`-deserialized `remind_before_minutes: Option<Option<i64>>` PATCH-semantics field (a
   pattern worth preserving explicitly when this becomes a domain type, since PATCH-null-vs-absent is a
   real invariant, not incidental).
5. Business rules: due-date/frequency advancement logic in `due_reminder.rs`/`repository.rs`
   (installment counting, `paid_installments`/`total_installments` bounds).
6. Adapters: HTTP `interface/http/recurring_bills.rs` (6), Matrix handler `recurring_bills/handler.rs`, AI
   tool `modules/ai/tools/bills.rs`, MCP via shared executor; `due_reminder.rs` participates in the shared
   due-reminder sweep (see `modules/due_reminder.rs` below), no dedicated scheduler file of its own.
7. Transactions: none of its own (relies on `reminder`'s writes for the linked reminder).
8. Depends on: `due_reminder` (top-level shared module), finance (category alignment), reminder (creates
   reminders for upcoming bills).
9. Tests: 6, DB-backed.
10. Type mismatches: none (`due_day`/`total_installments`: `Option<i32>` matches `INTEGER`;
    `paid_installments: i32` matches `INTEGER NOT NULL DEFAULT 0`; `next_due_at`/`created_at`/
    `updated_at: String`/`Option<String>` match `TEXT`).

## reminder (`modules/reminder/*`)
1. Files: handler.rs 437, mod.rs 172, models.rs 195, repository.rs 691 (**over cap**),
   scheduler.rs 367, service.rs 458. Total 2320.
2. Macros: db_execute 9, db_fetch_all 4, db_fetch_one 3, db_fetch_optional 2. No dyn, no tx_*.
   Dynamic-SQL (column-list/filter interpolation, values still bound): `repository.rs:61,203` (`&format!
   ("SELECT {COLUMNS} FROM reminders WHERE id = ?")`-shaped) and `:272,277,303,308` (status-filter
   `WHERE` clause built with `format!`, same safe-bind pattern as `finance`/`todo`).
3. Tables: `reminders` (own; also written by `recurring_bills` and `todo` for linked reminders via
   `source_kind`/`source_id`).
4. FromRow: `models.rs::Reminder` - `ToSchema`.
5. Business rules: recurrence pattern enum + advancement (`models.rs::RecurrencePattern`,
   `handler.rs:212-265` parses `daily `/`weekly ` prefixes into it), retry-count bounding for delivery
   failures (`scheduler.rs`).
6. Adapters: HTTP `interface/http/reminders.rs` (5), Matrix handler `reminder/handler.rs`, AI tool
   `modules/ai/tools/reminder.rs`, MCP via shared executor, background scheduler `reminder/scheduler.rs`
   (due-reminder delivery sweep) - this is the scheduler the top-level `modules/due_reminder.rs` and
   `modules/todo/due_reminder.rs`/`modules/recurring_bills/due_reminder.rs` all funnel into via
   `source_kind`/`source_id`.
7. Transactions: none directly (single-statement `db_execute!`/`db_fetch_one!` calls; no `begin()`).
8. Depends on: settings (timezone for trigger-time display).
9. Tests: 26, DB-backed.
10. Type mismatches: none (`trigger_at`/`created_at: DateTime<Utc>`/`Option<DateTime<Utc>>` match
    `TIMESTAMPTZ`).

## rooms (`modules/rooms/mod.rs`) and `db/rooms.rs`
1. Files: `modules/rooms/mod.rs` 193; `db/rooms.rs` 259 (the actual repository lives outside `modules/`
   per AGENTS.md's explicit rule: "room caching/resolution lives only in `db/rooms.rs`"). Total 452.
2. Macros (`db/rooms.rs`): db_fetch_all 1, db_scalar_optional 6, tx_execute 2. `modules/rooms/mod.rs`
   itself has no macro calls (thin wrapper/`BotModule` over `db/rooms.rs`).
3. Tables: `account_rooms` (own), reads `user_settings` (dev-room lookups). `room_id` columns across
   `todo`/`reminder`/`ai_conversations`/`transactions` historically depended on this table per the
   omitted-data-statements at the bottom of the baseline migration (room_id backfills) - i.e. `rooms` is
   a cross-cutting lookup nearly every feature's `room_id` column implicitly depends on, even without a
   formal FK.
4. FromRow: `db/rooms.rs::JoinedRoomInfo` - not `ToSchema` (internal only, mapped to an HTTP DTO
   elsewhere, presumably in the excluded `interface/http/rooms.rs`/`mod.rs`).
5. Business rules: "most-recently-active DM" and "most-recently-active room" resolution order
   (`db/rooms.rs:84,113` - `ORDER BY is_direct DESC, updated_at DESC`).
6. Adapters: HTTP `interface/http/rooms.rs` (2), Matrix `modules/rooms/mod.rs` (`BotModule`), no AI tool,
   no MCP tool, no scheduler. `webhook` module imports `crate::modules::rooms`.
7. Transactions: `db/rooms.rs:51` (`pool.begin()`) - protects the room-list bulk-upsert (delete-then-
   reinsert or upsert-many) against a partial write leaving stale/duplicate room rows.
8. Depends on: nothing. Depended on by: `webhook` (only cross-feature import found), plus implicit
   `room_id` consumers everywhere.
9. Tests: 1 (`db/rooms.rs`); `modules/rooms/mod.rs` has none directly.
10. Type mismatches: none in `JoinedRoomInfo` (`updated_at: Option<String>` matches
    `account_rooms.updated_at TEXT`).

**Placement note**: `db/rooms.rs` is explicitly called out by AGENTS.md as not to be coupled to HTTP
interface modules - it already behaves like an `infrastructure/postgres/repositories/rooms.rs` in spirit.
Treat "rooms" as a small, mostly self-contained shared feature to migrate early, not a leaf to defer
indefinitely, because almost every room_id-bearing table implicitly assumes its resolution semantics.

## settings (`modules/settings/*`)
1. Files: handler.rs 618 (**over cap**), mod.rs 187, models.rs 239, repository.rs 280. Total 1324.
2. Macros: db_execute 3, db_fetch_optional 1, db_scalar_optional 1. No dyn, no tx_*.
3. Tables: `user_settings` (own).
4. FromRow: `models.rs` struct (confirmed present at line 44) - `ToSchema`, persistence == transport.
5. Business rules: timezone/currency/briefing-time parsing and validation (`models.rs::parse_timezone`,
   `parse_briefing_time`, `parse_tts_voice`), max-history-turns bounds (1-100), "a missing row means
   defaults, never an error" (AGENTS.md-documented invariant - verify the repository actually returns a
   default struct rather than erroring on `NULL`/absent row, since that's easy to regress under `query!`'s
   stricter `Option` handling).
6. Adapters: HTTP `interface/http/settings.rs` (3), Matrix handler `settings/handler.rs`, AI tool
   `modules/ai/tools/settings.rs`, MCP via shared executor, no scheduler.
7. Transactions: none.
8. Depends on: finance (currency list), todo (some settings interplay per import graph - confirm exact
   use at migration time). Depended on by: nearly everything (finance, portfolio, reminder, todo, tts,
   voice, journal, briefing, ai indirectly) - this is the most fanned-in "shared" feature besides accounts
   and rooms.
9. Tests: 32, DB-backed.
10. Type mismatches: none identified from the visible fields.

## todo (`modules/todo/*`)
1. Files: due_reminder.rs 263, handler.rs 357, mod.rs 267, models.rs 74, mutate.rs 516 (**over cap**),
   repository.rs 520 (**over cap**), service.rs 560 (**over cap**), view.rs 112. Total 2669.
2. Macros: db_execute 9, db_fetch_all 5, db_fetch_optional 2. No dyn, no tx_*. Dynamic-SQL (column-list/
   filter interpolation, bind-everything pattern): `repository.rs:36,57,192,210,316,329`.
3. Tables: `todos` (own; also writes linked `reminders` via `source_kind`/`source_id`).
4. FromRow: `models.rs::Todo` - `ToSchema`.
5. Business rules: completion/priority state transitions (`mutate.rs`), due-reminder linkage
   (`due_reminder.rs`), 500-char text cap (consistent with the other features' pattern; verify exact site
   in `handler.rs`).
6. Adapters: HTTP `interface/http/todos.rs` (5), Matrix handler `todo/handler.rs`, AI tool
   `modules/ai/tools/todo.rs`, MCP via shared executor, its own due-reminder sweep participant
   (`todo/due_reminder.rs`) feeding into `reminder/scheduler.rs`.
7. Transactions: none directly.
8. Depends on: `due_reminder` (top-level shared module), reminder, settings.
9. Tests: 28, DB-backed.
10. Type mismatches: none (`due_at: Option<DateTime<Utc>>` matches `TIMESTAMPTZ`;
    `remind_before_minutes: Option<i64>` matches `BIGINT`).

## tts (`modules/tts/*`)
1. Files: mod.rs 391, text.rs 87. Total 478.
2. Macros: none - pure inference-adapter (Kokoro TTS), no table of its own.
3. Tables: none directly (reads `user_settings.tts_voice` via `settings`).
4. FromRow: none.
5. Business rules: 500-char speak-input cap (`mod.rs:29`, explicitly mirrors the other features'
   convention).
6. Adapters: no HTTP file identified distinctly (voice output likely returned inline from Matrix
   handler), Matrix handler is `tts/mod.rs` itself (`BotModule` at line 169), no AI tool, no MCP tool, no
   scheduler.
7. Transactions: none.
8. Depends on: settings.
9. Tests: 15, mixed (mostly pure text-processing; some may need the Kokoro model per the AGENTS.md Windows
   gotcha).
10. N/A.

## voice (`modules/voice/*`)
1. Files: mod.rs 248, models.rs 104, prompt.rs 57. Total 409.
2. Macros: none - inference-adapter (STT), no table of its own.
3. Tables: none.
4. FromRow: none.
5. Business rules: none beyond prompt/transcription formatting.
6. Adapters: HTTP `interface/http/voice.rs` (1), Matrix dispatch presumably via `ai` (imports `ai`), no
   AI tool file of its own (transcription front-end feeding into the AI conversational flow), no MCP
   tool, no scheduler.
7. Transactions: none.
8. Depends on: ai.
9. Tests: 7, pure.
10. N/A.

## webhook (`modules/webhook/*`)
1. Files: handler.rs 227, mod.rs 94, models.rs 189, renderer.rs 282, repository.rs 414,
   scheduler.rs 149. Total 1355.
2. Macros: db_execute 8, db_fetch_all 3, db_fetch_one 2, db_fetch_optional 4. No dyn, no tx_*.
3. Tables: `webhooks`, `webhook_outbox` (own).
4. FromRow: `models.rs::Webhook`, `WebhookOutboxItem` - both `ToSchema`.
5. Business rules: token hashing/prefix, retry/backoff bounds (`max_attempts`), payload rendering per
   target format (Telegram vs generic) in `renderer.rs`.
6. Adapters: HTTP `interface/http/webhooks.rs` (9), `webhooks_telegram.rs` (2); Matrix handler
   `webhook/handler.rs`; AI tool `modules/ai/tools/webhooks.rs`; MCP via shared executor; background
   scheduler `webhook/scheduler.rs` (outbox delivery/retry sweep).
7. Transactions: none.
8. Depends on: rooms (default_room_id resolution/validation).
9. Tests: 5, DB-backed.
10. Type mismatches: none (`total_delivered_count`/`failure_count`/`attempts`/`max_attempts: i64` match
    `BIGINT`; all timestamp fields `DateTime<Utc>`/`Option<DateTime<Utc>>` match `TIMESTAMPTZ`).

## due_reminder (`modules/due_reminder.rs`, top-level, not a feature directory)
1. Files: 134 lines.
2. Macros: none directly visible at top level - this file appears to be the shared dispatch point that
   `todo/due_reminder.rs` and `recurring_bills/due_reminder.rs` both feed into/from, alongside
   `reminder/scheduler.rs`. Confirm the exact call graph before treating it as belonging to any one
   feature; it is a cross-feature scheduling seam, not itself a feature.
3-10: not independently applicable - fold into the `reminder`/`todo`/`recurring_bills` write-ups above at
   migration time; do not move it in isolation.

## events.rs (`backend/src/events.rs`)
1. Files: 327 lines.
2. Macros: none - `SystemEvent` enum + `EventBroadcaster`, no persistence.
3. Tables: none.
4. FromRow: none.
5. Business rules: **every `SystemEvent` variant must return a meaningful `owner_id()`** (AGENTS.md-hard
   rule) - this is the one file every feature's application-service layer will need an adapter call into
   after each mutation; worth confirming the variant list still has 1:1 coverage with the features above
   as each one moves (a missed variant during refactor breaks SSE delivery for that feature silently).
6-9: N/A (cross-cutting infrastructure, not a feature).
10: N/A.

---

## Cross-feature dependency graph (crate::modules::X imported from inside modules::Y)

```
ai        -> bookmarks, finance, footprints, habits, life, portfolio, recurring_bills, reminder, settings, todo, webhook
briefing  -> habits, portfolio, recurring_bills, settings, todo
finance   -> portfolio, settings
journal   -> life, settings
mcp       -> ai, finance, habits, life, portfolio, todo   (all transitively via ai tool executor)
ocr       -> ai
portfolio -> finance, settings
recurring_bills -> due_reminder, finance, reminder
reminder  -> settings
settings  -> finance, todo
todo      -> due_reminder, reminder, settings
tts       -> settings
voice     -> ai
webhook   -> rooms

crate::accounts imported from: account, ai, bookmarks, footprints, mcp, portfolio, rooms, webhook
  (many more features use accounts transitively via owner_id resolution at the HTTP/Matrix boundary,
  which lives in the excluded files - this list is only modules/ internal imports.)
```

Leaves (no modules:: dependents besides ai/mcp tool-layer reach, and no modules:: dependencies of
their own): bookmarks, footprints, habits. Near-leaves with one outgoing dependency: rooms (none
outgoing), voice/ocr/tts (depend only on ai/settings, but themselves not depended on by other feature
modules).

Heavily-depended-on shared features: settings (fanned into by finance, portfolio, reminder, todo, tts,
voice indirectly, journal, briefing), accounts (fanned into by nearly everything for owner_id), and
rooms (implicit room_id semantics almost everywhere, explicit import only from webhook).

ai/mcp are best treated as a single downstream tool/orchestration layer that must migrate last, since it
calls into every other feature repository and its own AI-tool files are effectively adapters that need
to be repointed at each feature new application service as that feature moves.

## Suggested migration order

1. Foundational and shared, migrate first: rooms (small, self-contained, but semantically load-bearing -
   get it right early since room_id resolution touches nearly every table), settings (fanned-into by many
   features but itself depends only on finance/todo for minor lookups - worth breaking that small coupling
   first), then accounts in a dedicated, larger effort (the merge dynamic-SQL problem below needs its own
   design decision before or during this step, not deferred).
2. True leaves next (safe, low-blast-radius practice runs for the new layering and the query! conversion
   pattern): bookmarks, footprints, habits, webhook (only depends on rooms).
3. Small-to-medium features with shallow dependencies: recurring_bills, todo, reminder (these three are
   entangled via due_reminder/source_kind/source_id linking - migrate them as one coordinated batch, not
   independently, or the shared due-reminder sweep breaks mid-migration).
4. Money-handling features: finance, then portfolio (portfolio is larger and has the richest
   transactional and invariant surface - the audit-hash-chain and buy/sell transactions deserve dedicated
   review; do it once finance simpler patterns are proven).
5. Aggregators: life (reads across nearly every other feature table directly - must follow all of its
   read targets, not precede them), journal (thin wrapper over life), briefing (thin wrapper over habits,
   portfolio, recurring_bills, settings, todo).
6. Adapter-only features (little or no SQL, migrate opportunistically alongside whatever they depend on):
   ocr, voice, tts.
7. Tool and orchestration layer last: ai (and by extension mcp, which is just ai executor exposed over a
   different protocol) - repoint each modules/ai/tools/<feature>.rs file at the corresponding feature new
   application/<feature>/ service as that feature moves, rather than waiting for every feature to finish
   before touching ai at all; otherwise ai becomes one enormous final PR.
8. accounts::merge.rs deserves explicit sequencing of its own: decide the query!-compatible replacement
   for its dynamic per-table SQL (see risk below) before or in parallel with step 1, since every
   subsequent feature migration needs to know what shape its OWNED_TABLES entry must take.

## Three largest risks

1. accounts::merge.rs dynamic identifier interpolation has no direct query! equivalent. It iterates the
   OWNED_TABLES constant at runtime and builds SELECT/UPDATE/DELETE ... {table} ... {column} ... SQL text
   (13 sites, merge.rs:167-1203). sqlx::query!/query_as! require the full SQL text to be a compile-time
   literal, so this code cannot be mechanically converted one call at a time the way every other feature
   straight-line CRUD can. Any plan that assumes rewrite every db_*!/tx_*! call as query! needs an
   explicit design decision for this file before the mass conversion starts, or it will either be silently
   left on the interim macros indefinitely (contradicting the stated goal) or rewritten into an unwieldy
   twenty-plus-arm match/dispatch that itself needs careful review to keep the allow-listed-identifier
   safety property AGENTS.md documents.
2. life cross-feature aggregation reads other features tables directly, not through their repositories,
   and touches names (activity_counts, all_dates, user_activities) that do not correspond to real tables
   in 0001_baseline.sql - meaning life/repository.rs SQL likely uses CTEs or subquery aliases pulling from
   bookmarks, footprints, habit_logs, todos, transactions, reminders, and portfolio_audit_log in a handful
   of large UNION-style queries. Converting these to query! will require the full literal SQL text for a
   query spanning up to seven other features tables at once, so it can only be finalized after every one
   of those tables shapes is stable post-migration - any schema-adjacent change to those tables
   mid-migration risks silently breaking life without its own tests catching it unless its queries are
   read closely first (not just its declared FromRow structs, which look fine).
3. Widespread persistence and transport type fusion (FromRow plus Serialize/ToSchema on the same struct)
   across nearly every feature (bookmarks, finance, footprints, habits, life, mcp, portfolio,
   recurring_bills, reminder, settings, todo, webhook, and even accounts::security_events) means the
   planned domain/infrastructure split is not just a file-move - every one of these roughly twenty structs
   needs a real domain type plus a From/mapping impl at the repository edge, touching every HTTP handler,
   AI tool file, and Matrix formatter that currently constructs or matches on the row type directly. This
   is the single largest source of touched call-sites per feature, larger than the SQL-macro conversion
   itself, and is easy to under-estimate when sizing by line count or macro count alone - size each
   feature partly by how many external call-sites construct or consume its FromRow struct, not just by
   its own file size.

Secondary risks worth tracking but not top three: several files already exceed the 500-line cap
(ai/handler.rs 1209, portfolio/repository.rs 845, finance/repository.rs 624, finance/handler.rs 632,
todo/repository.rs and service.rs roughly 520-560, reminder/repository.rs 691, bookmarks/metadata.rs 516,
portfolio/audit.rs, price_feed.rs, service.rs 500-555) and will need splitting as part of, not incidental
to, their layer moves; and todo/reminder/recurring_bills shared due_reminder linkage (source_kind and
source_id) needs one coordinated migration rather than three independent ones or the sweep breaks for
whichever feature moves first.
