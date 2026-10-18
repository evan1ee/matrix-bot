# Finance Budgets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Monthly spending limits per category and overall, with a status view (spent, remaining, pace), one-time 80 %/100 % alerts, and the same capability from chat, REST, AI/MCP, the web UI and the daily briefing.

**Architecture:** Pure rules (thresholds, month bounds, status, pace) in a new `domain/finance/budget.rs`. Two tables: `finance_budgets` (owner-keyed) and `finance_budget_alerts` (the idempotency record). `FinanceService` owns budget CRUD and status, and evaluates budgets inside the same database transaction as every expense write. A crossing inserts an alert row, publishes `BudgetThresholdCrossed` and enqueues a `finance.budget_alert` job; the job DMs linked identities the way portfolio price alerts do.

**Tech Stack:** Rust (Axum, sqlx compile-checked macros, PostgreSQL 18, utoipa, chrono-tz), React 19 + Vite + Tailwind v4 + TanStack Query, Playwright.

**Spec:** [`docs/superpowers/specs/2026-10-18-finance-budgets.md`](../specs/2026-10-18-finance-budgets.md)

## Global Constraints

- Work on branch `feat/finance-budgets` in a separate worktree (AGENTS.md §9); never touch the main checkout's `.env` or `data/`; never run the bot binary (only `cargo run --bin generate-openapi`).
- Migration number is the next free one at implementation time (`0012` is reserved by the todo plan → expect `0013_finance_budgets.sql`); file is LF.
- Money is integer minor units: `limit_minor BIGINT > 0` with `currency` + `currency_exponent`; parse with `money::parse_amount` / `normalise_currency` / `exponent_for`. No floating point anywhere in budget math; percent comparisons use `i128`.
- Period: calendar month in the owner's timezone (`settings::get(..).unwrap_or_default().tz()`); bounds are local midnights converted to UTC; end is exclusive.
- What counts: `direction = 'expense'`, same currency, `occurred_at` in the month, any `source_kind`. Uncategorised rows count only toward overall budgets.
- Thresholds are exactly **80** and **100**. Alerts only for the owner's **current** month, only when `alerts_enabled`, once per (budget, month, threshold).
- One budget per (owner, category-or-overall, currency): `UNIQUE … NULLS NOT DISTINCT`. Duplicate → `AppError::Conflict`.
- Every mutation publishes a `SystemEvent` in the same transaction via `outbox::publish`.
- `finance_budgets` goes into `OWNED_TABLES`; `.sqlx/` regenerated after every query change.
- No production file in `backend/src/` over 500 lines (`#[cfg(test)]` excluded).
- Chat inputs ≤ 500 chars; every user value in HTML through `html_escape()`; replies via `text_html`. REST payload strings ≤ 500 chars; errors via `ApiError`; another owner's budget → 404.
- Commits: Conventional Commits ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not push, merge or open a PR.

## Rulings made while planning

Recorded in `docs/superpowers/HANDOFF.md` in Task 11.

- Ruling: inline budget feedback comes from a new `FinanceService::record_transaction` returning `RecordedTransaction { transaction, budgets }`; `create_transaction` and `create_transaction_in_tx` keep their signatures and drop the statuses — keeps recurring bills, portfolio cash, OCR and REST callers untouched — cost if wrong: a caller that later wants statuses switches to `record_transaction`.
- Ruling: account merge remaps `finance_budgets.category_id` from `from`'s colliding categories to `into`'s in `reconcile_colliding_owned_rows` (step 1b, right after the transactions remap); `move_rows` then moves the rest and already skips rows that collide on the unique index, and those die with `from`'s account. This replaces the spec's "delete first" wording with the same outcome (surviving account wins) — cost if wrong: a merged user loses a duplicate budget they would have kept anyway.
- Ruling: the DM loop and owner-readiness check in `interface/jobs/portfolio.rs` move to a shared `interface/jobs/dm.rs` used by both price and budget alerts — second real use — cost if wrong: none.
- Ruling: `budget` tools live in a new `llm/tools/budgets.rs` (`finance.rs` is already 546 lines) wired into `executor.rs` and `definitions.rs` — cost if wrong: none.
- Ruling: the briefing builds `FinanceService::new(pool.clone())` and calls `budget_status` rather than reading repositories, so the status rule lives once — cost if wrong: none.
- Ruling: `Month` serialises as the string `"YYYY-MM"` everywhere (events, REST, job payload) — cost if wrong: none.

## Review Focus

1. **A budget for a currency the owner has no spend in** (`!budget set 100 JPY`) — status shows 0 spent, 0 %, Ok, and remaining = limit; no division by zero in pace. Test: Task 1 `status_with_no_spend_is_ok`.
2. **Overall budget plus category budget hit by one expense** — both evaluated, each alerts once, and the chat reply lists both lines. Test: Task 4 `one_expense_can_alert_overall_and_category_budgets`.
3. **Updating an expense's currency or category away from a budget** — evaluates only the new values; no alert for the old budget and no error. Test: Task 4 `update_moving_expense_to_other_currency_does_not_alert_old_budget`.
4. **First day of the month at 00:30 local in a zone ahead of UTC** (e.g. `Asia/Shanghai`, UTC still the previous day) — the expense belongs to the new local month. Test: Task 1 `month_containing_uses_owner_zone_not_utc`.
5. **`!budget set` with a category name differing only in case** (`Food` vs `food`) — resolves to the existing category the way `-c` does, never creates one. Test: Task 7 `budget_set_resolves_category_case_insensitively`.

---

## File Structure

| File | Responsibility |
|---|---|
| `backend/src/domain/finance/budget.rs` (new) | `BudgetScope`, `Budget`, `NewBudget`, `BudgetPatch`, `Threshold`, `reached_thresholds`, `Month`, `BudgetState`, `Pace`, `BudgetStatus`, `status`, `RecordedTransaction` |
| `backend/src/domain/finance/mod.rs` | `pub mod budget;`, new `FinanceError` variants |
| `backend/migrations/0013_finance_budgets.sql` (new) | DDL from spec §4.1 |
| `backend/src/domain/accounts/mod.rs` | `finance_budgets` in `OWNED_TABLES` |
| `backend/src/infrastructure/postgres/repositories/finance/budgets.rs` (new) | budget CRUD, month spend, alert record |
| `backend/src/infrastructure/postgres/merge.rs` | step 1b budget category remap |
| `backend/src/domain/events.rs` | `BudgetCreated`, `BudgetUpdated`, `BudgetDeleted`, `BudgetThresholdCrossed` |
| `backend/src/application/finance/budgets.rs` (new) | CRUD, `set_budget`, `resolve_budget`, `budget_status`, `status_for_budget` |
| `backend/src/application/finance/budget_eval.rs` (new) | `evaluate_budgets_in_tx` |
| `backend/src/application/finance/transactions.rs` | `record_transaction`, evaluation in create/update |
| `backend/src/application/finance/mod.rs` | modules, `From<FinanceError>` arms |
| `backend/src/domain/jobs.rs` | `JobType::BudgetAlertSend`, `BudgetAlertPayload`, `NewJob::budget_alert` |
| `backend/src/interface/jobs/dm.rs` (new) | `owner_ready`, `dm_linked_identities` |
| `backend/src/interface/jobs/budgets.rs` (new) | `BudgetAlertHandler`, `format_budget_alert` |
| `backend/src/interface/jobs/{mod,portfolio}.rs` | register handler; use `dm.rs` |
| `backend/src/application/briefing/{generator,models}.rs` | budget lines, `budgets_attention_count` |
| `backend/src/interface/chat/finance/budget.rs` (new) | `!budget` commands, inline lines |
| `backend/src/interface/chat/finance/{handler,mod}.rs` | route `budget`, use `record_transaction`, help |
| `backend/src/interface/http/finance_budgets.rs` (new) | REST handlers + DTOs |
| `backend/src/interface/http/{routes,openapi}.rs` | routes, OpenAPI paths/schemas |
| `backend/src/llm/tools/budgets.rs` (new) + `executor.rs`, `definitions.rs`, `finance.rs` | AI tools |
| `frontend/src/modules/finance/components/{BudgetPanel,BudgetModal}.tsx` (new) | web UI |
| `frontend/src/modules/finance/{FinanceMainView.tsx,index.tsx,types.ts}` | mount panel, SSE invalidation, types |
| `frontend/e2e/{fake-api.ts,finance.spec.ts}` | mock routes, Playwright |
| `docs/superpowers/HANDOFF.md`, `docs/api.md` | rulings, status, endpoints |

---

### Task 1: Budget domain

**Files:**
- Create: `backend/src/domain/finance/budget.rs`
- Modify: `backend/src/domain/finance/mod.rs`

**Interfaces:**
- Produces:
  - `enum BudgetScope { Overall, Category { id: i64, name: String } }` (Serialize/Deserialize, `#[serde(tag = "kind", rename_all = "snake_case")]`), `fn label(&self) -> &str` ("Overall" or the name).
  - `struct Budget { pub id: i64, pub owner_id: AccountId, pub scope: BudgetScope, pub currency: String, pub currency_exponent: i64, pub limit_minor: i64, pub alerts_enabled: bool, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }`.
  - `struct NewBudget { category: Option<String>, currency: String, currency_exponent: i64, limit_minor: i64, alerts_enabled: bool }` with getters; `NewBudget::new(category: Option<&str>, currency: &str, limit_minor: i64, alerts_enabled: bool) -> Result<NewBudget, FinanceError>` (trims category; empty → `None`; > `MAX_CATEGORY_LEN` → `CategoryNameInvalid`; currency via `normalise_currency`; exponent via `exponent_for`; `limit_minor <= 0` → `BudgetLimitInvalid`).
  - `struct BudgetPatch { pub limit_minor: Option<i64>, pub alerts_enabled: Option<bool> }`, `fn validate(&self) -> Result<(), FinanceError>`.
  - `enum Threshold { Warning, Exceeded }`, `fn pct(self) -> i16` (80/100), `fn from_pct(i16) -> Option<Threshold>`.
  - `fn reached_thresholds(spent_minor: i64, limit_minor: i64) -> Vec<Threshold>` (ascending).
  - `struct Month { year: i32, month: u32 }` — `containing(at: DateTime<Utc>, tz: Tz) -> Month`, `parse(&str) -> Result<Month, FinanceError>` (`BudgetMonthInvalid`), `first_day(&self) -> NaiveDate`, `start_utc(&self, tz: Tz) -> DateTime<Utc>`, `end_utc(&self, tz: Tz) -> DateTime<Utc>`, `days(&self) -> u32`, `Display` as `YYYY-MM`; serde as that string.
  - `enum BudgetState { Ok, Warning, Exceeded }`, `struct Pace { pub expected_minor: i64, pub projected_minor: i64 }`.
  - `struct BudgetStatus { pub budget: Budget, pub month: Month, pub spent_minor: i64, pub remaining_minor: i64, pub percent_used: i64, pub state: BudgetState, pub pace: Option<Pace> }`.
  - `fn status(budget: Budget, spent_minor: i64, month: Month, now: DateTime<Utc>, tz: Tz) -> BudgetStatus`.
  - `struct RecordedTransaction { pub transaction: Transaction, pub budgets: Vec<BudgetStatus> }`.
  - `FinanceError` gains `BudgetExists` ("You already have a budget for that category and currency"), `BudgetNotFound` ("Budget not found or access denied"), `BudgetLimitInvalid` ("Budget limit must be greater than zero"), `BudgetMonthInvalid` ("Month must look like 2026-10"), `BudgetAmbiguous { scope: String }` ("You have {scope} budgets in several currencies; name the currency").

- [ ] **Step 1: Write the failing tests** in `budget.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test] fn reached_thresholds_boundaries() {
    assert!(reached_thresholds(7_999, 10_000).is_empty());
    assert_eq!(reached_thresholds(8_000, 10_000), vec![Threshold::Warning]);
    assert_eq!(reached_thresholds(9_999, 10_000), vec![Threshold::Warning]);
    assert_eq!(reached_thresholds(10_000, 10_000), vec![Threshold::Warning, Threshold::Exceeded]);
    assert_eq!(reached_thresholds(15_000, 10_000).len(), 2);
    assert_eq!(reached_thresholds(i64::MAX, i64::MAX).len(), 2); // no overflow
}
#[test] fn month_containing_uses_owner_zone_not_utc() {
    // 2026-10-31T16:30Z is 2026-11-01 00:30 in Asia/Shanghai
    let at = Utc.with_ymd_and_hms(2026, 10, 31, 16, 30, 0).unwrap();
    assert_eq!(Month::containing(at, chrono_tz::Asia::Shanghai).to_string(), "2026-11");
    assert_eq!(Month::containing(at, chrono_tz::UTC).to_string(), "2026-10");
}
#[test] fn month_bounds_across_dst_and_year_end() {
    let m = Month::parse("2026-03").unwrap();
    let tz = chrono_tz::Europe::Berlin;
    assert_eq!(m.start_utc(tz), Utc.with_ymd_and_hms(2026, 2, 28, 23, 0, 0).unwrap());
    assert_eq!(m.end_utc(tz), Utc.with_ymd_and_hms(2026, 3, 31, 22, 0, 0).unwrap());
    assert_eq!(Month::parse("2026-12").unwrap().end_utc(chrono_tz::UTC),
               Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap());
    assert_eq!(Month::parse("2028-02").unwrap().days(), 29);
    assert!(Month::parse("2026-13").is_err() && Month::parse("26-1").is_err());
}
#[test] fn status_with_no_spend_is_ok() { /* limit 10_000 JPY exp 0, spent 0, now = first instant of month:
    state Ok, percent 0, remaining 10_000, pace Some { expected_minor: 322 (10_000*1/31), projected_minor: 0 } */ }
#[test] fn status_over_limit() { /* spent 10_300 of 10_000 → Exceeded, percent 103, remaining -300 */ }
#[test] fn status_pace_only_for_current_month() { /* now in November, month October → pace None */ }
#[test] fn status_pace_on_day_12_of_31() { /* limit 31_000, spent 15_000, now Oct 12 local:
    expected 12_000, projected 38_750 */ }
#[test] fn new_budget_validation() {
    assert!(matches!(NewBudget::new(None, "EUR", 0, true), Err(FinanceError::BudgetLimitInvalid)));
    assert!(NewBudget::new(None, "EURO", 100, true).is_err());
    let b = NewBudget::new(Some("  "), "kwd", 1_000, true).unwrap();
    assert_eq!((b.category(), b.currency(), b.currency_exponent()), (None, "KWD", 3));
}
#[test] fn month_serialises_as_string() { /* serde_json::to_string(&Month::parse("2026-10")) == "\"2026-10\"" and round-trips */ }
```

- [ ] **Step 2: Run** `cargo test -p matrix-bot domain::finance::budget` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the Interfaces above. `reached_thresholds`: `spent as i128 * 100 >= limit as i128 * pct`. `percent_used = floor(spent*100/limit)` in `i128`. `state` from `reached_thresholds(..).last()`. Pace only when `Month::containing(now, tz) == month`: `elapsed = local day of now` (today counts), `expected = limit * elapsed / days`, `projected = spent * days / elapsed`, all `i128` truncated. Add `pub mod budget;` and the error variants to `mod.rs`.
- [ ] **Step 4: Run** `cargo test -p matrix-bot domain::finance::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(finance): budget domain rules`.

### Task 2: Schema, repository and merge

**Files:**
- Create: `backend/migrations/0013_finance_budgets.sql`, `backend/src/infrastructure/postgres/repositories/finance/budgets.rs`
- Modify: `backend/src/infrastructure/postgres/repositories/finance/mod.rs` (`pub mod budgets;`), `backend/src/domain/accounts/mod.rs`, `backend/src/infrastructure/postgres/merge.rs`

**Interfaces:**
- Consumes: Task 1 `Budget`, `BudgetScope`, `NewBudget`, `BudgetPatch`, `Month`, `Threshold`.
- Produces (all `pub async fn … (ex: impl PgExecutor<'_>, …) -> sqlx::Result<…>`; every read joins `finance_categories` for the name):
  - `insert(ex, owner: &AccountId, category_id: Option<i64>, b: &NewBudget) -> Budget`
  - `find_owned(ex, owner, id: i64) -> Option<Budget>`
  - `find_by_scope_for_update(ex, owner, category_id: Option<i64>, currency: &str) -> Option<Budget>` (`category_id IS NOT DISTINCT FROM $2 … FOR UPDATE OF b`)
  - `list(ex, owner) -> Vec<Budget>` (overall first, then category name)
  - `list_for_scope(ex, owner, category_id: Option<i64>) -> Vec<Budget>` (all currencies; for chat `rm`/`alerts` resolution)
  - `list_matching(ex, owner, currency: &str, category_id: Option<i64>) -> Vec<Budget>` (`alerts_enabled AND currency = $2 AND (category_id IS NULL OR category_id = $3)`)
  - `update(ex, owner, id, patch: &BudgetPatch) -> Option<Budget>` (`COALESCE` per field, `updated_at = now()`)
  - `delete(ex, owner, id) -> bool`
  - `month_spend(ex, owner, from: DateTime<Utc>, to: DateTime<Utc>, currency: Option<&str>) -> Vec<SpendRow>`; `pub struct SpendRow { pub category_id: Option<i64>, pub currency: String, pub spent_minor: i64 }` grouped by `(category_id, currency)`, expenses only, `occurred_at >= from AND occurred_at < to`.
  - `record_alert(ex, budget_id: i64, month: Month, threshold: Threshold) -> bool` (`INSERT … ON CONFLICT DO NOTHING RETURNING budget_id`, `true` when inserted).

- [ ] **Step 1: Write the migration** exactly as spec §4.1, and add `"finance_budgets"` to `OWNED_TABLES` after `"finance_categories"`.
- [ ] **Step 2: Write failing repository tests** in `budgets.rs` (use `test_pool()`, `run_migrations_unlocked`, `seed_account`):
  - `insert_and_find_round_trip` (overall and category scope; name joined).
  - `second_overall_budget_same_currency_is_unique_violation` (and a USD overall alongside EUR succeeds).
  - `budgets_are_owner_isolated` (`find_owned`/`update`/`delete` with another owner → `None`/`false`).
  - `month_spend_counts_only_expenses_in_bounds` (income, other currency, `occurred_at == to`, and `occurred_at == from - 1µs` excluded; uncategorised row returned with `category_id: None`; a row with `source_kind = 'recurring_bill'` included).
  - `deleting_category_cascades_its_budgets`.
  - `record_alert_is_idempotent` (`true` then `false`).
- [ ] **Step 3: Run** `cargo test -p matrix-bot repositories::finance::budgets` — Expected: FAIL.
- [ ] **Step 4: Implement** the repository with compile-checked macros and a private `BudgetRow` converted to `Budget` at the edge.
- [ ] **Step 5: Merge.** Add failing test `merge_remaps_budget_category_and_keeps_into_on_collision` beside the existing category-merge test in `merge.rs`: `from` has budgets {food/EUR, travel/EUR, overall/EUR}, `into` has {food/EUR, overall/EUR} with category `food`; after `reconcile_colliding_owned_rows` + `move_rows` for `finance_categories` and `finance_budgets`, `into` has exactly food/EUR (its own limit), travel/EUR, overall/EUR (its own limit). Then add step 1b to `reconcile_colliding_owned_rows`: `UPDATE finance_budgets b SET category_id = fc_into.id FROM finance_categories fc_from JOIN finance_categories fc_into ON fc_into.owner_id = $1 AND LOWER(fc_into.name) = LOWER(fc_from.name) WHERE b.owner_id = $2 AND fc_from.owner_id = $2 AND b.category_id = fc_from.id AND NOT EXISTS (SELECT 1 FROM finance_budgets o WHERE o.owner_id = $2 AND o.category_id = fc_into.id AND o.currency = b.currency)`, and update the doc comment list.
- [ ] **Step 6: Run** `cargo test -p matrix-bot repositories::finance::budgets merge::` and the schema catalog test `cargo test -p matrix-bot db::` — Expected: PASS.
- [ ] **Step 7: Regenerate** `.sqlx/` (`cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` against a migrated scratch DB) and **commit** `feat(finance): budgets schema and repository`.

### Task 3: Events and budget service (CRUD + status)

**Files:**
- Create: `backend/src/application/finance/budgets.rs`
- Modify: `backend/src/application/finance/mod.rs`, `backend/src/domain/events.rs`

**Interfaces:**
- Consumes: Task 1, Task 2 repository.
- Produces:
  - `SystemEvent::BudgetCreated(Budget)`, `BudgetUpdated(Budget)`, `BudgetDeleted { id: i64, owner_id: AccountId }`, `BudgetThresholdCrossed { owner_id: AccountId, budget_id: i64, month: Month, threshold: i16, spent_minor: i64, limit_minor: i64, currency: String, currency_exponent: i64, scope_name: String }`; `owner_id()` and `event_type()` arms (`"BudgetCreated"` …); `destinations()` unchanged.
  - On `FinanceService`:
    - `create_budget(&self, owner, b: NewBudget) -> Result<Budget, AppError>`
    - `set_budget(&self, owner, b: NewBudget) -> Result<(Budget, bool /*created*/), AppError>` — one transaction: resolve category (`resolve_category_in_tx(.., create_missing = false)`), `find_by_scope_for_update`, update limit/alerts or insert; publishes Created or Updated.
    - `update_budget(&self, owner, id, patch: BudgetPatch) -> Result<Budget, AppError>`
    - `delete_budget(&self, owner, id) -> Result<(), AppError>`
    - `list_budgets(&self, owner) -> Result<Vec<Budget>, AppError>`
    - `resolve_budget(&self, owner, category: Option<&str>, currency: Option<&str>) -> Result<Budget, AppError>` — exactly one match or `BudgetNotFound` / `BudgetAmbiguous`.
    - `budget_status(&self, owner, month: Option<Month>) -> Result<Vec<BudgetStatus>, AppError>` — overall first, then `percent_used` descending.
    - `status_for_budget(&self, owner, id, month: Month) -> Result<Option<BudgetStatus>, AppError>`
  - `From<FinanceError> for AppError`: `BudgetNotFound` → `NotFound`, `BudgetExists` → `Conflict`, rest → `Validation`. A unique violation from `insert` maps to `BudgetExists`.

- [ ] **Step 1: Write failing service tests** in `application/finance/tests.rs` (or a new `budget_tests.rs` included from `mod.rs` if `tests.rs` passes ~500 lines):
  - `create_budget_publishes_budget_created` (outbox types `["BudgetCreated"]`).
  - `create_budget_twice_is_conflict`.
  - `create_budget_for_unknown_category_is_validation_and_creates_no_category`.
  - `set_budget_updates_existing_scope` (second call returns `created == false`, new limit, events `["BudgetCreated","BudgetUpdated"]`).
  - `resolve_budget_ambiguous_without_currency` (overall EUR + overall USD, `currency: None` → `Validation`).
  - `budget_status_sums_category_and_overall` (food 300.00 EUR budget, overall 1000.00 EUR; expenses food 120.00, uncategorised 50.00, income 999.00, USD 40.00 → food spent 12_000, overall spent 17_000; order overall then food).
  - `budget_status_for_other_owner_budget_is_none`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot application::finance::` — Expected: FAIL.
- [ ] **Step 3: Implement** the events and service methods. Status: one `month_spend(.., None)` call; overall spent = sum of rows with that currency; category spent = row with that `(category_id, currency)`; owner tz read once.
- [ ] **Step 4: Run** `cargo test -p matrix-bot application::finance:: domain::events` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(finance): budget service and events`.

### Task 4: Evaluation on expense writes and the alert job type

**Files:**
- Create: `backend/src/application/finance/budget_eval.rs`
- Modify: `backend/src/application/finance/transactions.rs`, `backend/src/domain/jobs.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `JobType::BudgetAlertSend` (`"finance.budget_alert"`, appended to `ALL`, array length 9); `JobPayload::BudgetAlert(BudgetAlertPayload)`; `pub struct BudgetAlertPayload { pub budget_id: i64, pub month: Month, pub threshold: i16 }`; `NewJob::budget_alert(owner: AccountId, budget_id: i64, month: Month, threshold: Threshold) -> NewJob` (key `budget:{budget_id}:{month}:{pct}`, 3 attempts, 60 s backoff).
  - `FinanceService::evaluate_budgets_in_tx(&self, dbtx: &mut sqlx::Transaction<'_, Postgres>, owner: &AccountId, tx: &Transaction) -> Result<Vec<BudgetStatus>, AppError>` — spec §5.3 steps 1–4; returns non-`Ok` statuses of evaluated budgets.
  - `FinanceService::record_transaction(&self, owner, input: NewTransaction<'_>) -> Result<RecordedTransaction, AppError>`; `create_transaction` and `create_transaction_in_tx` delegate and return `.transaction`; `update_transaction` calls `evaluate_budgets_in_tx` before commit and discards the result.

- [ ] **Step 1: Write failing tests** (service level, outbox and `jobs` table inspected through existing test helpers):
  - `crossing_80_records_alert_publishes_event_and_enqueues_job` (limit 100.00, expenses 50.00 then 31.00 → after the second: one `finance_budget_alerts` row (80), event `BudgetThresholdCrossed`, one `finance.budget_alert` job; `RecordedTransaction.budgets[0].state == Warning`).
  - `one_write_crossing_both_records_both_and_enqueues_only_100`.
  - `concurrent_crossings_alert_once` (two `tokio::join!` `record_transaction` calls that each cross 80 % → exactly one alert row and one job).
  - `backdated_expense_never_alerts` (last month, over limit → no alert row, no job; `budget_status(Some(last_month))` shows `Exceeded`).
  - `alerts_disabled_budget_never_alerts`.
  - `one_expense_can_alert_overall_and_category_budgets`.
  - `update_moving_expense_to_other_currency_does_not_alert_old_budget`.
  - `failed_outbox_publish_rolls_back_expense_and_alert` (use `outbox::fail_every_publish`; no transaction row, no alert row).
  - `recurring_bill_payment_triggers_evaluation` (`services.recurring_bills.mark_paid` crossing a budget enqueues the job).
  - `new_job_budget_alert_dedupe_key` (domain unit test in `jobs.rs`).
- [ ] **Step 2: Run** `cargo test -p matrix-bot application::finance:: domain::jobs` — Expected: FAIL.
- [ ] **Step 3: Implement.** Evaluation reads the owner tz with `settings::get(&mut **dbtx, owner)`; early return unless `Expense` and `Month::containing(tx.occurred_at, tz) == Month::containing(now, tz)`; for each `list_matching` budget, `month_spend(.., Some(currency))` gives spend (overall = sum, category = its row); for each reached threshold `record_alert`; if any inserted, publish one `BudgetThresholdCrossed` and enqueue one job for the highest inserted threshold.
- [ ] **Step 4: Run** `cargo test -p matrix-bot application::finance:: application::recurring_bills:: application::portfolio::` — Expected: PASS (existing tests unchanged).
- [ ] **Step 5: Commit** `feat(finance): evaluate budgets on expense writes`.

### Task 5: Alert delivery job

**Files:**
- Create: `backend/src/interface/jobs/dm.rs`, `backend/src/interface/jobs/budgets.rs`
- Modify: `backend/src/interface/jobs/mod.rs`, `backend/src/interface/jobs/portfolio.rs`

**Interfaces:**
- Consumes: Task 4 `BudgetAlertPayload`, `JobType::BudgetAlertSend`; Task 3 `status_for_budget`.
- Produces:
  - `pub(crate) async fn owner_ready(services: &AppServices, outbound: &dyn Outbound, owner: &AccountId) -> Result<bool, AppError>` and `pub(crate) async fn dm_linked_identities(outbound: &dyn Outbound, identities: Vec<LinkedIdentity>, plain: &str, html: &str)` — moved verbatim from `CheckAlertsHandler::is_owner_ready` / `notify_identities`, which now call them.
  - `pub struct BudgetAlertHandler { services, outbound }`, `new(services, outbound)`; `JobHandler::run`: `Unavailable(CHANNEL_RETRY)` unless `owner_ready`; budget gone → `Ok(())`; else DM.
  - `pub(crate) fn format_budget_alert(s: &BudgetStatus, threshold: Threshold, today: NaiveDate) -> (String, String)`.

- [ ] **Step 1: Write failing tests** in `budgets.rs` with `FakeOutbound`:
  - `format_warning_matches_spec_copy`: Food, 24_600 of 30_000 EUR, 2026-10-19 → plain `Budget warning: Food — 246.00 EUR of 300.00 EUR used (82 %) with 12 days left.` (days left = days in month − today's day, i.e. today counts as used).
  - `format_exceeded_matches_spec_copy`: Overall, 153_000 of 150_000 → `Budget exceeded: Overall — 1530.00 EUR of 1500.00 EUR (102 %).` (`format_amount` does no thousands grouping; the spec's `€1,530.00` was illustrative).
  - `format_escapes_category_name` (`<b>x</b>` category → escaped in html).
  - `sends_to_every_connected_identity`, `defers_while_disconnected`, `deleted_budget_completes_silently`.
  - Existing `portfolio.rs` tests still pass after the move.
- [ ] **Step 2: Run** `cargo test -p matrix-bot interface::jobs::` — Expected: FAIL.
- [ ] **Step 3: Implement** and register `BudgetAlertHandler` in `handlers()`.
- [ ] **Step 4: Run** `cargo test -p matrix-bot interface::jobs::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(finance): deliver budget alerts by DM`.

### Task 6: Briefing

**Files:** Modify `backend/src/application/briefing/generator.rs`, `backend/src/application/briefing/models.rs`

**Interfaces:**
- Consumes: Task 3 `budget_status`.
- Produces: `BriefingSummary.budgets_attention_count: usize` (`#[serde(default)]`); `format_briefing_content` gains `budgets: &[BudgetStatus]`.

- [ ] **Step 1: Failing tests** in `generator.rs`: `briefing_lists_budgets_needing_attention` (one Warning, one Ok → one line `Food: 82 % of 300.00 EUR used`, count 1) and `briefing_omits_budget_section_when_all_ok`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot application::briefing::` — Expected: FAIL.
- [ ] **Step 3: Implement**: `FinanceService::new(pool.clone()).budget_status(owner_id, None)`, keep non-`Ok`; on error log and use empty (as the other sections do with `unwrap_or_default`). Section placed after bills.
- [ ] **Step 4: Run** the same — Expected: PASS. **Step 5: Commit** `feat(briefing): show budgets needing attention`.

### Task 7: Chat commands

**Files:**
- Create: `backend/src/interface/chat/finance/budget.rs`
- Modify: `backend/src/interface/chat/finance/handler.rs`, `backend/src/interface/chat/finance/mod.rs`

**Interfaces:**
- Consumes: Task 3 service methods, Task 4 `record_transaction`.
- Produces: `pub async fn handle_budget(services: &AppServices, owner: &AccountId, rest: &str, default_currency: &str) -> String`; `pub fn budget_lines(statuses: &[BudgetStatus]) -> String` (one line per status, `Food budget: 85 % of 300.00 EUR used`, html-escaped).

- [ ] **Step 1: Failing tests** (parsing as unit tests; flows with `test_pool`):
  - `parse_budget_set` table: `set 300` → overall/default currency; `set 300 EUR food` → category food; `set 300 food and drink` → category "food and drink"; `set abc` → error; `set 0` → error.
  - `budget_set_resolves_category_case_insensitively` (category `Food` exists; `!budget set 300 food` attaches to it; category count unchanged).
  - `budget_set_unknown_category_points_to_category_add` (reply contains `!category add`).
  - `budget_status_reply_lists_budgets` (`!budget` shows name, spent/limit, percent; `!budget 2026-09` has no pace text).
  - `budget_rm_requires_currency_when_ambiguous`, `budget_alerts_off`.
  - `expense_add_appends_budget_line_at_80_percent`.
  - Input over 500 chars → the existing too-long reply.
- [ ] **Step 2: Run** `cargo test -p matrix-bot interface::chat::finance::` — Expected: FAIL.
- [ ] **Step 3: Implement**: in `handle_finance_command` add a `budget` keyword branch before `category` calling `handle_budget`; the add path calls `record_transaction` and appends `budget_lines`; add the five `CommandHelp` entries from spec §6.1.
- [ ] **Step 4: Run** — Expected: PASS. **Step 5: Commit** `feat(chat): budget commands`.

### Task 8: REST API

**Files:**
- Create: `backend/src/interface/http/finance_budgets.rs`
- Modify: `backend/src/interface/http/routes.rs` (`routes_finance`), `backend/src/interface/http/openapi.rs`, `backend/openapi.json`, `frontend/src/` generated client, `docs/api.md`

**Interfaces:**
- Consumes: Task 3.
- Produces DTOs (`ToSchema`, separate from domain, `From<&BudgetStatus>`): `BudgetDto { id, scope: "overall"|"category", category_id: Option<i64>, category_name: Option<String>, currency, currency_exponent, limit_minor, alerts_enabled }`, `BudgetStatusDto { budget: BudgetDto, month: String, spent_minor, remaining_minor, percent_used, state: "ok"|"warning"|"exceeded", pace: Option<PaceDto> }`, `CreateBudgetRequest { category_id: Option<i64>, currency: String, limit: String, alerts_enabled: Option<bool> }`, `UpdateBudgetRequest { limit: Option<String>, alerts_enabled: Option<bool> }`. Handlers `list_budgets` (`?month=`), `create_budget` (201), `update_budget`, `delete_budget` (204) on `/finance/budgets` and `/finance/budgets/:id`.

- [ ] **Step 1: Failing tests** in the file's test module (follow `finance_categories.rs` tests): create → 201 and list shows it; duplicate → 409; other owner's id on PUT/DELETE → 404; `limit: "0"` → 400; `month=2026-13` → 400; `category_id` of another owner → 404; string > 500 chars → 400.
- [ ] **Step 2: Run** `cargo test -p matrix-bot interface::http::finance_budgets` — Expected: FAIL.
- [ ] **Step 3: Implement** handlers (category id resolved via the existing `owned_category` pattern, then name passed to `NewBudget::new`); register routes and OpenAPI paths/schemas.
- [ ] **Step 4: Run** tests, then `cargo run --bin generate-openapi`, `npm --prefix frontend run generate-api`; add the endpoints to `docs/api.md`. Expected: tests PASS, `openapi.json` diff is additive only.
- [ ] **Step 5: Commit** `feat(api): budget endpoints`.

### Task 9: AI tools and MCP

**Files:**
- Create: `backend/src/llm/tools/budgets.rs`
- Modify: `backend/src/llm/tools/{mod,executor,definitions,finance}.rs`

**Interfaces:**
- Consumes: Tasks 3–4.
- Produces: `budgets::tool_definitions() -> Vec<ToolDefinition>` with `get_budget_status { month?: string pattern ^\d{4}-\d{2}$ }`, `set_budget { amount: string (required), currency?: string, category?: string, alerts_enabled?: boolean }`, `remove_budget { category?: string, currency?: string }` (`additionalProperties: false`); `budgets::execute_tool(services, owner_id, name, args) -> Option<Result<String, String>>`. `record_finance_entry` uses `record_transaction` and adds `"budgets"` (non-Ok statuses) to its JSON result.

- [ ] **Step 1: Failing tests**: `set_budget_tool_creates_budget_for_owner`; `set_budget_tool_rejects_numeric_amount_type_and_bad_currency` (malformed → `Err` string, nothing written); `remove_budget_tool_ambiguous_is_error`; `get_budget_status_tool_bad_month_is_error`; `record_finance_entry_reports_budgets`; `all_tool_definitions` contains the three names exactly once.
- [ ] **Step 2: Run** `cargo test -p matrix-bot llm::tools::` — Expected: FAIL.
- [ ] **Step 3: Implement** and wire into `executor.rs` after finance and `definitions.rs` after `finance::tool_definitions()`. Owner only from the `owner_id` parameter.
- [ ] **Step 4: Run** — Expected: PASS. **Step 5: Commit** `feat(llm): budget tools`.

### Task 10: Web UI

**Files:**
- Create: `frontend/src/modules/finance/components/BudgetPanel.tsx`, `frontend/src/modules/finance/components/BudgetModal.tsx`
- Modify: `frontend/src/modules/finance/{FinanceMainView.tsx,index.tsx,types.ts}`, `frontend/e2e/fake-api.ts`, `frontend/e2e/finance.spec.ts`

**Interfaces:**
- Consumes: Task 8 endpoints via `core/apiClient.ts` (`fetchApi`), generated types.
- Produces: query key `['finance-budgets', month]`; `handleSseEvent` invalidates it on `Transaction*`, `BudgetCreated|BudgetUpdated|BudgetDeleted|BudgetThresholdCrossed`, `CategoryDeleted`.

- [ ] **Step 1: Failing Playwright spec** in `finance.spec.ts`: `budgets: create, see progress, edit, remove` — open ledger view, "Add budget", choose "Overall", limit `500`, submit with `Enter`; row shows `0.00 / 500.00` and "On track"; fake API seeded with a 450.00 expense shows `90 % used`; edit limit to `400` → "Over by 50.00"; previous-month button changes the heading; remove asks to confirm and the row disappears; `Escape` closes the modal. Add mock routes for the four endpoints in `fake-api.ts`.
- [ ] **Step 2: Run** `npm --prefix frontend run test:e2e -- finance.spec.ts` — Expected: FAIL.
- [ ] **Step 3: Implement** per spec §6.4 and Design.md (monochrome, Geist Mono for amounts, text states not colour-only, `transition-colors duration-150`, visible labels and focus). Month formatting uses the user's saved zone.
- [ ] **Step 4: Run** `npm --prefix frontend run build` and the e2e spec — Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): finance budgets panel`.

### Task 11: Docs and full verification

**Files:** Modify `docs/superpowers/HANDOFF.md`, `docs/architecture.md` (finance section: budgets and alert path, one paragraph)

- [ ] **Step 1:** Add the planning rulings above and a status row for budgets to `HANDOFF.md`.
- [ ] **Step 2: Run every gate** (AGENTS.md §8) and record actual results:

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo test -p matrix-bot --all-targets
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run build
npm --prefix frontend run test:e2e
git diff --check
```

Expected: all pass; no production file over 500 lines (`wc -l` the touched files, excluding test modules).
- [ ] **Step 3: Commit** `docs(superpowers): record finance budgets`.
