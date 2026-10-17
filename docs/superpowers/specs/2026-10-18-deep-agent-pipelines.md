# Spec: Deep Procedural Pipelines for Built-in Agent Tasks

## 1. Problem Statement & Motivation

The autonomous agent subsystem currently executes recurring scheduled tasks (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`) via unstructured ReAct prompt loops and basic tool whitelists.

While this allows general LLM reasoning, it exhibits several critical limitations:
1. **Mathematical Inaccuracy & Tool Overhead:** Mathematical calculations (e.g. portfolio asset allocation drift, bill cash-flow adequacy, habit streaks, category spending spikes) are either delegated to the LLM's error-prone floating-point arithmetic or require dozens of round-trip tool steps.
2. **Missing Multi-Modal Output:** Despite local Kokoro-82M TTS being supported in the architecture, scheduled digests (such as the weekly bookmark digest) are delivered exclusively as wall-of-text messages without audio voice message compilation.
3. **Superficial Task Goals:** The 5 built-in task plugins have simplistic prompt formulations that lack structured input parameters (such as target weights, drift thresholds, audit lookback windows, and urgency horizons).

## 2. Design Principles & Architecture

To address these limitations, we implement **Deep Procedural Pipelines** across all 5 built-in agent tasks:
- **Pure Domain Calculations:** Heavy computations (portfolio drift, cash-flow coverage, habit streaks, spending spikes) are implemented as deterministic, pure, synchronous domain functions in `domain/`.
- **Dedicated Analytical Tools:** Domain calculations are exposed via specialized tools in `llm/tools/` so the agent executes deterministic domain math with zero hallucinations in a single step.
- **Multi-Modal TTS Audio Compilation:** `WeeklyBookmarkDigestTask` compiles an audio digest script into an Ogg Opus voice note via `TtsClient` and attaches it to the final `OutgoingMessage` (`AttachmentKind::Audio`).
- **Fail-Safe & Graceful Degradation:** If TTS inference is disabled, unconfigured, or fails, the text digest is delivered cleanly without error.
- **Architectural Purity:** All domain modules remain pure Rust (`std`, `chrono`, `uuid`, `serde`, `thiserror`), interface adapters perform no SQL, and all production files remain under 500 lines.

```text
Scheduled Task Trigger (tasks.rs)
          │
          ▼
Formulate Structured Goal (domain/agent/task.rs)
          │
          ▼
ReAct Step Loop (AgentExecuteRunHandler)
          │
          ├──► Specialized Analytical Tool (llm/tools/)
          │         │
          │         ▼
          │    Pure Domain Math (domain/<feature>/)
          │
          ▼
Final Answer Synthesis
          │
          ├──► Audio Script Extraction (<audio_script>)
          │         │
          │         ▼
          │    Kokoro TTS Synthesis (TtsClient)
          │         │
          │         ▼
          │    Audio Attachment (AttachmentKind::Audio)
          │
          ▼
Delivered OutgoingMessage (Text + Voice Note via Outbound)
```

---

## 3. Detailed Component Specifications

### 3.1 Capability 1: Deterministic Portfolio Drift & Rebalance Engine

- **Module:** `backend/src/domain/portfolio/rebalance.rs`
- **Domain Types:**
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct AssetAllocationDrift {
      pub symbol: String,
      pub current_value_minor: i64,
      pub current_weight_pct: f64,
      pub target_weight_pct: f64,
      pub drift_pct: f64,
      pub rebalance_action: RebalanceAction,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub enum RebalanceAction {
      Buy { amount_minor: i64 },
      Sell { amount_minor: i64 },
      InBand,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct PortfolioDriftReport {
      pub total_value_minor: i64,
      pub allocations: Vec<AssetAllocationDrift>,
      pub max_drift_pct: f64,
      pub needs_rebalance: bool,
  }
  ```
- **Domain Function:**
  `pub fn evaluate_portfolio_drift(holdings: &[Holding], targets: &BTreeMap<String, f64>, threshold_pct: f64) -> Result<PortfolioDriftReport, PortfolioError>`
- **Tool:** `evaluate_portfolio_rebalance` in `llm/tools/portfolio/rebalance.rs` (registered in `llm/tools/portfolio/mod.rs`).
- **Task Plugin Update:** `PortfolioRebalanceAlertTask` in `domain/agent/task.rs`:
  - `allowed_tools`: includes `evaluate_portfolio_rebalance`, `get_my_portfolio_summary`, `get_portfolio_holding`.
  - Config schema: `{ "targets": { "AAPL": 50.0, "BTC": 50.0 }, "drift_threshold_pct": 5.0 }`.
  - Goal formulation: Instructs agent to run `evaluate_portfolio_rebalance` and explain specific trade adjustments without executing direct trades.

---

### 3.2 Capability 2: Direct Kokoro TTS Audio Digest Compilation

- **Task Plugin:** `WeeklyBookmarkDigestTask` in `domain/agent/task.rs`:
  - Config schema: `{ "limit": 10, "tag": "", "audio": true, "voice": "..." }`.
  - Goal formulation: Directs the agent to produce:
    1. A categorized markdown digest of key takeaways.
    2. An audio-ready spoken podcast briefing wrapped inside `<audio_script>...</audio_script>` (conversational, ~150-250 words, free of markdown symbols or URLs).
- **Audio Compilation Pipeline:**
  - In `interface/jobs/agent/mod.rs` (`AgentExecuteRunHandler`):
    - Extract text inside `<audio_script>...</audio_script>` using `extract_audio_script(&final_answer)`.
    - If present (and task is `weekly_bookmark_digest` or `audio: true`), synthesize audio via `TtsClient` using effective configuration (`ResolvedAiConfig` / `services.app_config`).
    - Attach synthesized audio as `Attachment { kind: AttachmentKind::Audio, mime: "audio/ogg", filename: "weekly-digest.ogg", ... }` to `OutgoingMessage`.
    - Deliver both markdown summary and native audio message to destination room/channel.
    - If TTS fails or is unconfigured, logs a warning and delivers text cleanly.

---

### 3.3 Capability 3: Deterministic Financial Audit & Category Spike Detection

- **Module:** `backend/src/domain/finance/audit.rs`
- **Domain Types:**
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct CategorySpendingAnalysis {
      pub category: String,
      pub total_minor: i64,
      pub transaction_count: usize,
      pub percentage_of_total: f64,
      pub is_spike: bool,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct MonthlyAuditReport {
      pub total_spending_minor: i64,
      pub recurring_bills_total_minor: i64,
      pub discretionary_spending_minor: i64,
      pub categories: Vec<CategorySpendingAnalysis>,
      pub flagged_spikes: Vec<String>,
  }
  ```
- **Domain Function:**
  `pub fn evaluate_monthly_audit(transactions: &[Transaction], recurring_bills: &[RecurringBill], spike_threshold_pct: f64) -> MonthlyAuditReport`
- **Tool:** `audit_monthly_finances` in `llm/tools/finance.rs`.
- **Task Plugin Update:** `MonthlyFinancialAuditTask` in `domain/agent/task.rs`:
  - Whitelist includes `audit_monthly_finances`, `list_finance_transactions`, `get_my_recurring_bills`.
  - Config schema: `{ "spike_threshold_pct": 20.0 }`.

---

### 3.4 Capability 4: Deterministic Bill Due & Cash Flow Adequacy Analysis

- **Module:** `backend/src/domain/bills/due.rs`
- **Domain Types:**
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub enum BillUrgency {
      DueToday,
      DueWithin3Days,
      DueWithin14Days,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct BillDueAnalysis {
      pub bill_id: i64,
      pub name: String,
      pub due_date: String,
      pub amount_minor: i64,
      pub urgency: BillUrgency,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct CashFlowAdequacyReport {
      pub total_due_minor: i64,
      pub available_cash_minor: i64,
      pub projected_shortfall_minor: i64,
      pub bills: Vec<BillDueAnalysis>,
      pub has_shortfall: bool,
  }
  ```
- **Domain Function:**
  `pub fn evaluate_bill_cash_adequacy(bills: &[RecurringBill], cash_balances: &[CashBalance], today: NaiveDate, days_ahead: i64) -> CashFlowAdequacyReport`
- **Tool:** `evaluate_bill_due_adequacy` in `llm/tools/bills.rs`.
- **Task Plugin Update:** `BillDueDigestTask` in `domain/agent/task.rs`:
  - Whitelist includes `evaluate_bill_due_adequacy`, `get_my_recurring_bills`.
  - Config schema: `{ "days_ahead": 14 }`.

---

### 3.5 Capability 5: Deterministic Habit Consistency & Streak Review

- **Module:** `backend/src/domain/habits/consistency.rs`
- **Domain Types:**
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub enum HabitTrend {
      Improving,
      Stable,
      AtRisk,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct HabitConsistencyItem {
      pub habit_id: i64,
      pub name: String,
      pub weekly_completion_rate_pct: f64,
      pub current_streak: i64,
      pub longest_streak: i64,
      pub trend: HabitTrend,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct HabitConsistencyReport {
      pub total_active: usize,
      pub average_weekly_rate_pct: f64,
      pub items: Vec<HabitConsistencyItem>,
      pub slipping_habits: Vec<String>,
      pub top_streaks: Vec<String>,
  }
  ```
- **Domain Function:**
  `pub fn evaluate_habit_consistency(habits: &[HabitWithStatus]) -> HabitConsistencyReport`
- **Tool:** `evaluate_habit_consistency` in `llm/tools/habits.rs`.
- **Task Plugin Update:** `HabitConsistencyReviewTask` in `domain/agent/task.rs`:
  - Whitelist includes `evaluate_habit_consistency`, `get_my_habits`.
  - Config schema: `{ "days": 7 }`.

---

## 4. Verification & Testing Strategy

1. **Unit Tests (Pure Domain):**
   - `domain/portfolio/rebalance.rs`: Verify calculation of target weights, drift percentages, and buy/sell actions with integer minor units.
   - `domain/finance/audit.rs`: Verify category aggregation, recurring bill reconciliation, and spike detection.
   - `domain/bills/due.rs`: Verify urgency grouping, cash balance summation, and shortfall detection.
   - `domain/habits/consistency.rs`: Verify streak tracking, at-risk categorization, and average rate calculation.
   - `domain/agent/task.rs`: Verify goal formulations with various custom config payloads.
2. **Tool Execution Tests:**
   - Unit tests for tools `evaluate_portfolio_rebalance`, `audit_monthly_finances`, `evaluate_bill_due_adequacy`, `evaluate_habit_consistency`.
3. **Audio Compilation Integration Tests:**
   - In `interface/jobs/agent/mod.rs`: Verify `<audio_script>` extraction, TTS synthesis mock, and audio `Attachment` attachment to `OutgoingMessage`.
4. **Architectural & Safety Gates:**
   - Zero SQL outside `infrastructure/postgres/repositories/` (`interface_runs_no_sql` and `architecture.rs`).
   - Max 500 lines per production file.
   - Clippy compiles with 0 warnings.
