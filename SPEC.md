# Matrix Bot — Complete Architecture Design & Implementation Specification

> **Version:** 2.4.0  
> **Status:** Roadmap specification. Sub-project 1 (phases 1–3) is designed in [`docs/superpowers/specs/2026-09-24-postgres-foundation-design.md`](docs/superpowers/specs/2026-09-24-postgres-foundation-design.md); later phases each get their own design before implementation.  
> **Target Database:** PostgreSQL 16+ is the only application database (SQLite removed). The Matrix SDK's own end-to-end-encryption store remains an SDK-managed SQLite file — see §2.3.  
> **Interaction Interfaces:** Matrix E2EE (Active / Implemented), Discord & Telegram ([TODO] Planned Features — Extensible Channel Framework), Web UI (React 19 SPA), MCP (Model Context Protocol), External Agents (Cursor, Claude Desktop)  
> **Agent Execution Scope:** Scheduled Specific Tasks Exclusively ([TODO] Planned Feature — Extensible Task Plugin Architecture); Live User Input Handled Exclusively by Direct LLM + Tool Calling  

---

## 0. Revision Notes (2.3.0 → 2.4.0)

Changes made while reviewing this document against the codebase:

1. **Matrix crypto store:** matrix-sdk 0.7 has no maintained PostgreSQL store; its encryption state stays an SDK-owned SQLite file. "PostgreSQL only" applies to the application database (§2.3, §16).
2. **Schema (§4):** the previous §4 described an invented schema that dropped about 16 existing tables and renamed others. §4 now lists the real tables by domain; `backend/migrations/*.sql` is the single source of truth for columns.
3. **Identifiers:** row primary keys stay `BIGINT` (chat commands show them, e.g. `!todo done 3`); `accounts.id` and every `owner_id` become native `UUID` with foreign keys (§4.1).
4. **Tool names:** OpenAI function names must match `^[a-zA-Z0-9_-]+$`; dotted names (`finance` + `.` + `record_expense`) are invalid and are now `finance_record_expense` (§7.4).
5. **Live LLM path:** "single turn, <1s" was not achievable — a read tool needs a second completion to answer, and web search alone takes seconds. The rule is now a **bounded tool-calling loop** (a fixed maximum of rounds, no persistent planner, no background state) (§1.3, §7.1).
6. **Outbox:** one `published` flag cannot track fan-out to several destinations; delivery is now tracked per destination in `outbox_deliveries`, which replaces `webhook_outbox` (§6).
7. **Job queue:** added `dedupe_key` (a scheduled occurrence is enqueued once) and a heartbeat-based reaper (a long job is not stolen) (§5).
8. **Queries:** compile-time-checked `sqlx::query!` macros with a committed `.sqlx/` offline cache, on sqlx 0.8 (§2.2).
9. **Consistency fixes:** Redis removed from §9.5; duplicate §7.3 renumbered; §9 heading level; §13 aligned with the approved layout; §16 criteria made testable.

---

## 1. Executive Summary & Architectural Vision

The platform is evolving from a Matrix-only bot into an **Omnichannel Personal Life Workspace**. The system provides a unified backend powering daily productivity, personal finance, investment tracking, knowledge management, location logging, and natural language orchestration across multiple chat platforms and web interfaces.

### 1.1 Component Status & Implementation State

To maintain engineering discipline and prevent scope creep, system capabilities are strictly categorized into **Active** and **TODO / Planned** features, with all planned features designed around **extensible plugin abstractions**:

| Subsystem / Feature | Implementation Status | Implementation Architecture | Target Milestone | Extensibility Model |
|---|---|---|---|---|
| **Matrix Channel (E2EE)** | **Active / Implemented** | Native `matrix-sdk` with crypto store | Shipped; refactor to `ChannelAdapter` in Phase 4 | Native driver for `ChannelAdapter` |
| **Discord Channel** | **[TODO] Planned Feature** | `serenity` Gateway + Slash Commands | Phase 4 (Omnichannel milestone) | Pluggable `ChannelAdapter` plugin |
| **Telegram Channel** | **[TODO] Planned Feature** | `teloxide` Axum Webhook + Bot API | Phase 4 (Omnichannel milestone) | Pluggable `ChannelAdapter` plugin |
| **Web Console (React 19)** | **Active / Implemented** | Axum REST + Server-Sent Events (SSE) | Shipped; align with outbox events in Phase 6 | Modular React 19 micro-views |
| **MCP Server** | **Active / Implemented** | Stdio & SSE JSON-RPC protocol | Shipped; align with Tool Registry in Phase 4 | Tool Registry schema provider |
| **User Input Gateway** | **Active / Implemented** | Deterministic CLI + Direct LLM + Tools | Phase 2 (Refactored domain services) | Dynamic `ToolDefinition` registry |
| **Scheduled Autonomous Agent** | **[TODO] Planned Feature** | Background worker ReAct runtime | Phase 5 (Scheduled Automation milestone) | Pluggable `ScheduledAgentTask` trait |

### 1.2 Core Architectural Principle

> **Omnichannel Chat (Matrix, Discord, Telegram, etc.) is the primary conversational interaction interface; Web is the visual management and analytics interface; MCP is the external developer interface for LLMs and tools; LLMs act as the natural language understanding and direct tool-calling layer for all live user input; Scheduled Autonomous Agents [TODO Feature] orchestrate recurring multi-step background workflows; all planned extensions (Omnichannel Adapters, Scheduled Agent Tasks, Domain Tools) follow open-closed, extensible trait-based plugin architectures; Domain Services encapsulate all business invariants; and PostgreSQL is the sole, authoritative source of truth for persistent state, durable jobs, and transactional outbox events.**

### 1.3 Execution Strategy: Interactive User Input (LLM + Tools) vs. Scheduled Autonomous Agent [TODO Feature]

A foundational architectural rule establishes clear boundaries between real-time user interaction and scheduled background automation:

> **Live user input is handled exclusively via Deterministic CLI or Direct LLM + Tool Calling (a bounded tool loop — no planner, no persisted run state). The Agent Runtime is NEVER invoked on interactive user input; it is strictly reserved for specific scheduled tasks as an extensible [TODO Feature].**

```text
┌─────────────────────────────────────────────────────────┐   ┌─────────────────────────────────────────────────────────┐
│        INTERACTIVE USER INPUT (LIVE CHAT & WEB)         │   │        SCHEDULED AUTONOMOUS TASKS [TODO FEATURE]        │
│                                                         │   │                                                         │
│               User Chat Message / Web Request           │   │              PostgreSQL Cron / Timer Trigger            │
│                              │                          │   │                              │                          │
│                              ▼                          │   │                              ▼                          │
│                Intent Gateway / Syntax Check            │   │              Durable Job Claim (SKIP LOCKED)            │
│                              │                          │   │                              │                          │
│               ┌──────────────┴──────────────┐           │   │                              ▼                          │
│               ▼                             ▼           │   │                  Scheduled Agent Worker                 │
│      [ CLI Syntax / Slash ]      [ Natural Language ]   │   │                              │                          │
│               │                             │           │   │                              ▼                          │
│               ▼                             ▼           │   │               Load `ScheduledAgentTask` Plugin          │
│        Direct Command               LLM + Tool Calling  │   │                              │                          │
│     ┌─────────────────────┐       ┌───────────────────┐ │   │                              ▼                          │
│     │ • Sub-millisecond   │       │ • Bounded rounds  │ │   │                     Stateful ReAct Loop                 │
│     │ • 0 tokens          │       │ • Direct tool call│ │   │             ┌──────────────────────────────────┐        │
│     │ • Deterministic     │       │ • Immediate reply │ │   │             │ Think ──► Act ──► Observe ──►... │        │
│     │ • Domain service tx │       │ • No run state    │ │   │             └────────────────┬─────────────────┘        │
│     └─────────────────────┘       └───────────────────┘ │   │                              │                          │
│                                                         │   │                              ▼                          │
│            NO AGENT RUNTIME FOR USER INPUT              │   │             Multi-Channel Outbox Notification           │
└─────────────────────────────────────────────────────────┘   └─────────────────────────────────────────────────────────┘
```

1. **Interactive User Input (Deterministic CLI & Direct LLM + Tool Calling):**
   * **Rule:** **No Agent runtime is introduced to handle live user input.**
   * **Mechanism:** When a user types a message or command from Matrix, Web, Discord, Telegram, or MCP:
     * If the input matches a command prefix (`!todo add`, `/expense`, slash commands), it executes directly via the application service layer (0ms LLM latency, 0 tokens).
     * If the input is freeform natural language, the request is sent to the `LlmToolGateway` along with schemas from the Unified Tool Registry. The LLM selects the matching domain tool (e.g. `finance_record_expense`), the application service executes within a PostgreSQL transaction, and an immediate confirmation is returned.
   * **Invariants:** A bounded tool-calling loop (at most `MAX_TOOL_ROUNDS` = 5 completions per message, a code constant), predictable token expenditure, no persisted planner state, and the answer returned in the same request.

2. **Scheduled Autonomous Agent Runtime ([TODO Feature] — Extensible Background Automation):**
   * **Rule:** Agent execution is **exclusively reserved for specific scheduled tasks** triggered by timers or cron schedules.
   * **Mechanism:** The scheduler instantiates an autonomous Agent run in `agent_runs` to complete a predefined multi-step objective (e.g., weekly digest compilation, monthly financial audit, scheduled portfolio rebalance check). The Agent executes a bounded ReAct (Reasoning + Acting) loop using allowed domain tools, logs all steps to `agent_steps` in PostgreSQL, and dispatches the final synthesized brief to the user's linked chat channels via the Transactional Outbox.
   * **Status:** Tracked as a planned future milestone (Phase 5).

### 1.3.1 User Input Routing Pipeline (CLI vs. LLM + Tool Calling)

When an incoming prompt is received from any chat interface (Matrix, Discord, Telegram) or the Web UI, the system routes the request using a **2-Stage User Input Pipeline**:

```text
                           Incoming User Prompt
                                    │
                                    ▼
              ┌───────────────────────────────────────────┐
              │ Stage 0: Deterministic Syntax Fast-Path   │
              │ (Command prefix: !, /, slash commands)    │
              └─────────────────────┬─────────────────────┘
                                    │
                  ┌─────────────────┴─────────────────┐
                  ▼ (Exact command matched)           ▼ (Freeform natural language)
         [ Direct Command ]                  ┌────────────────────────────────────────┐
      Bypasses LLM completely                │ Stage 1: Direct LLM + Tool Calling     │
      0ms latency, 0 tokens                  │ Bounded tool loop (max N rounds)       │
      (e.g. /todo add Buy milk)              └───────────────────┬────────────────────┘
                                                                 │
                                         ┌───────────────────────┴───────────────────────┐
                                         ▼ (Tool call selected)                          ▼ (Conversational / QA)
                              ┌──────────────────────┐                       ┌──────────────────────┐
                              │ Domain Tool Execution│                       │ Direct Natural Reply │
                              │ (Immediate DB Tx)    │                       │ (Streaming Response) │
                              └──────────┬───────────┘                       └──────────┬───────────┘
                                         │                                              │
                                         └───────────────────────┬──────────────────────┘
                                                                 ▼
                                                  Immediate Confirmation to User
                                                   (No Agent Loop • Zero Delay)
```

1. **Stage 0 — Deterministic Syntax Fast-Path (0 Tokens, Sub-millisecond):**
   * If the prompt uses CLI syntax (`!todo add`, `/expense`, or a native Discord Slash Command), the system bypasses all LLMs. It directly deserializes the parameters into an Application Command and executes the domain logic.
2. **Stage 1 — Direct LLM + Tool Calling (Bounded Loop):**
   * Freeform natural language is sent to the LLM with the relevant domain tools from the Unified Tool Registry. A write tool usually completes in one tool round; a read tool needs a second completion to phrase the answer. The loop stops at `MAX_TOOL_ROUNDS`.
   * The LLM emits a direct domain tool call (e.g. `finance_record_expense(amount=15, category="food")`), which executes immediately against PostgreSQL and returns a confirmation to the user.
   * If the user asks a conversational question or requests information, the LLM provides an instant direct answer.
   * **Invariants:** No agent state machines, background planners or persisted run state are involved in handling user input.

```text
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              INTERACTION SURFACES                               │
│                                                                                 │
│  ┌──────────────────┐  ┌──────────────────┐  ┌────────────────┐  ┌───────────┐  │
│  │  Matrix Clients  │  │  Discord Clients │  │ Telegram Client│  │  Web UI   │  │
│  │ (Element X/E2EE) │  │ (Guilds / DMs)   │  │(Bot API / Apps)│  │ (React 19)│  │
│  └────────┬─────────┘  └────────┬─────────┘  └───────┬────────┘  └─────┬─────┘  │
│           │                     │                    │                 │        │
│           │                     │                    │        MCP      │        │
│           │                     │                    │   ┌─────────────┴─────┐  │
│           │                     │                    │   │Claude / Cursor/etc│  │
│           │                     │                    │   └─────────────┬─────┘  │
│           │                     │                    │                 │        │
└───────────┼─────────────────────┼────────────────────┼─────────────────┼────────┘
            │                     │                    │                 │
            ▼                     ▼                    ▼                 ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                         INTERFACE ADAPTER LAYER (HEXAGONAL)                     │
│                                                                                 │
│   ┌───────────────────┐ ┌──────────────────┐ ┌───────────────┐ ┌─────────────┐ │
│   │  Matrix Adapter   │ │ Discord Adapter  │ │Telegram Adapter│ │Web API (SSE)│ │
│   │   (matrix-sdk)    │ │    (serenity)    │ │   (teloxide)   │ │  MCP Server │ │
│   └─────────┬─────────┘ └────────┬─────────┘ └───────┬───────┘ └──────┬──────┘ │
└─────────────┼────────────────────┼───────────────────┼────────────────┼─────────┘
              │                    │                   │                │
              └────────────────────┼───────────────────┘                │
                                   ▼                                    │
┌───────────────────────────────────────────────────────────────────────┼─────────┐
│                      UNIFIED CHAT ROUTER & CONTEXT                    │         │
│   IncomingMessage Normalize • Account Identity Resolution • Permissions         │
└──────────────────────────────────┬────────────────────────────────────┼─────────┘
                                   │                                    │
                                   ▼                                    ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                          LLM & AGENT SUBSYSTEM                                  │
│                                                                                 │
│   ┌─────────────────────────────────────────────────────────────────────────┐   │
│   │                          Unified Tool Registry                          │   │
│   │          (OpenAI Function Schemas, Permission Gates, Dispatch)          │   │
│   └──────────────────────┬──────────────────────────────────┬───────────────┘   │
│                          │                                  │                   │
│              [Live User Input]                        [Scheduled Tasks (Cron)]   │
│                          ▼                                  ▼                   │
│               ┌──────────────────────┐           ┌──────────────────────┐       │
│               │   LLM Tool Gateway   │           │Scheduled Agent Task  │       │
│               │ (Direct Tool Calling)│           │Runtime [TODO Feature]│       │
│               └──────────┬───────────┘           └──────────┬───────────┘       │
└──────────────────────────┼──────────────────────────────────┼───────────────────┘
                           │                                  │
                           └─────────────────┬────────────────┘
                                             ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                            APPLICATION SERVICE LAYER                            │
│                                                                                 │
│       Use Case Handlers  •  Command / Query Bus  •  Tx Orchestration            │
└────────────────────┬───────────────────────────────────────────────┬────────────┘
                     │                                               │
                     ▼                                               ▼
┌─────────────────────────────────────┐               ┌───────────────────────────┐
│            DOMAIN LAYER             │               │    DURABLE JOB SYSTEM     │
│                                     │               │                           │
│  Todos & Reminders                  │               │  STT & Voice Processing   │
│  Finance & Expenses                 │               │  TTS Audio Synthesis      │
│  Portfolio & Trading                │               │  OCR Receipt Extraction   │
│  Recurring Bills                    │               │  Daily Briefings & News   │
│  Bookmarks & Reading List           │               │  Scheduled Reminders      │
│  Habits & Routines                  │               │  Scheduled Agent Tasks    │
│  Journal & Footprints               │               │  Transactional Outbox     │
└────────────────────┬────────────────┘               └──────────────┬────────────┘
                     │                                               │
                     └───────────────────────┬───────────────────────┘
                                             ▼
┌─────────────────────────────────────────────────────────────────────────────────┐
│                          DATA & INFRASTRUCTURE LAYER                            │
│                                                                                 │
│   ┌─────────────────────────────────────────────────────────────────────────┐   │
│   │                          PostgreSQL 16+ (Only)                          │   │
│   │    Domain Tables  •  Jobs Queue (SKIP LOCKED)  •  Outbox Events         │   │
│   │    Platform Identities • Agent State • Audit • Sessions • Full-Text     │   │
│   └────────────────────────────────┬────────────────────────────────────────┘   │
│                                    │                                            │
│                      ┌─────────────┴─────────────┐                              │
│                      ▼                           ▼                              │
│           ┌──────────────────────┐    ┌──────────────────────┐                  │
│           │    Object Storage    │    │   External / Local   │                  │
│           │  (S3 / R2 / Local)   │    │   Inference Engine   │                  │
│           │ Audio, Images, OCR   │    │ (Kokoro, Sherpa STT) │                  │
│           └──────────────────────┘    └──────────────────────┘                  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Database Decision: PostgreSQL Only (Complete Removal of SQLite)

### 2.1 The Architectural Decision

**SQLite support is completely removed from the project.** PostgreSQL 16+ is the sole supported database backend across development, testing, and production.

### 2.2 Rationale and Technical Debt Elimination

Maintaining a dual-dialect database abstraction layer created massive incidental complexity and hindered system capabilities:

1. **Elimination of the Dual-Dialect Macro Layer:**
   The project previously maintained over 700 lines of complex macros in `backend/src/db/executor.rs` (`db_fetch_all!`, `db_fetch_one!`, `db_fetch_optional!`, `db_scalar_*!`, `db_execute!`, `db_fetch_all_dyn!`, `tx_*`) and the `DbPool` enum (`Sqlite(SqlitePool)` vs `Postgres(PgPool)`). Removing SQLite eliminates this layer entirely in favor of idiomatic, type-safe, direct `sqlx::PgPool` and `sqlx::query!` / `sqlx::query_as!` usage. The macros are checked at build time against a committed `.sqlx/` offline cache (`cargo sqlx prepare`), so builds and CI need no live database (`SQLX_OFFLINE=true`), and CI fails on a stale cache (`cargo sqlx prepare --check`). sqlx is upgraded from 0.7 to 0.8 first.
2. **Elimination of Runtime Query Rewriting:**
   `dialect::render_placeholders` dynamically parsed and rewrote every query from SQLite `?` syntax to PostgreSQL `$1..$n` at runtime. Eliminating this provides zero runtime translation overhead, unlocks native prepared statement caching, and allows direct SQL compilation.
3. **Removal of Token-Substituted DDL Hackery:**
   `db/ddl.rs` and macro tokens (`{{PK}}`, `{{NOW_TEXT}}`, `{{TIMESTAMPTZ}}`) are deleted. Database migrations become native, standard, transactional PostgreSQL DDL.
4. **Unlocking Advanced PostgreSQL Capabilities:**
   * **Durable Job Queues:** Native `SELECT ... FOR UPDATE SKIP LOCKED` enables a high-throughput, multi-worker persistent job queue without external dependencies like Redis or RabbitMQ.
   * **Transactional Outbox:** Atomic writes of domain state changes and outbox notification records in a single database transaction.
   * **Strict Typing:** Native `UUID`, `TIMESTAMPTZ` (microsecond precision), `BOOLEAN`, `JSONB`, `NUMERIC`, and `BIGINT` without dialect impedance mismatches or string fallbacks.
   * **Advisory Locks (`pg_advisory_xact_lock`):** Cluster-safe single-flight operations, scheduler leader election, and atomic critical sections.
   * **Full-Text & Vector Search:** Native `tsvector`/`tsquery` for search across bookmarks, notes, and journal entries, with seamless future adoption of `pgvector` for RAG.
   * **Listen / Notify (`LISTEN` / `NOTIFY`):** Instant outbox event dispatch and SSE wakeups without polling latency.

### 2.3 SQLite Deprecation & Removal Inventory

| Component | Target Action | Consequence |
|---|---|---|
| `backend/src/db/executor.rs` | **Delete** dual-dialect macros and `DbPool` enum | Replace with direct `sqlx::PgPool` and repository methods |
| `backend/src/db/dialect.rs` | **Delete** entirely | No runtime placeholder rewriting (`?` -> `$1..$n`) |
| `backend/src/db/dialect_probe.rs` | **Delete** entirely | No dual `FromRow` probe required |
| `backend/src/db/ddl.rs` | **Delete** entirely | Native PostgreSQL DDL replaces token substitution |
| `backend/src/db/backfill.rs` | **Delete** | Its one-time backfills are already applied to the live database; the baseline migration captures the result |
| `backend/src/db/pool.rs` | **Refactor** | Retain only `PgPoolOptions` configuration and health check |
| In-memory SQLite tests | **Replace** | Tests run against a transient PostgreSQL database / testcontainers |
| Unversioned `schema*.rs` | **Replace** | Forward-only versioned migrations (`backend/migrations/*.sql`) run by `sqlx::migrate!()`; `0001_baseline.sql` is today's schema written idempotently so the live PostgreSQL database adopts it in place |
| `data/bot_data.db` default `DATABASE_URL` | **Remove** | `DATABASE_URL` is required and must be a `postgres://` URL; anything else refuses to boot |
| matrix-sdk `sqlite_store` (crypto store) | **Keep** | SDK-owned end-to-end-encryption state, not application data; matrix-sdk has no maintained PostgreSQL store |

---

## 3. System Architecture & Six-Layer Model

The system is structured as a strict **Layered & Hexagonal Architecture** to enforce separation of concerns, testability, and multi-interface reuse.

```text
┌────────────────────────────────────────────────────────────────────────┐
│ 1. INTERFACE LAYER                                                     │
│    Chat Adapters (Matrix, Discord, Telegram) • Axum REST/SSE • MCP    │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ Calls use cases
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│ 2. APPLICATION LAYER                                                   │
│    Commands  •  Queries  •  Workflows  •  Permissions                  │
└───────────────────────┬──────────────────────────┬─────────────────────┘
                        │                          │
                        ▼                          ▼
┌───────────────────────────────────┐   ┌────────────────────────────────┐
│ 3. LLM & AGENT LAYER              │   │ 4. DOMAIN LAYER                │
│    Unified Tool Registry          │   │    Entities & Invariants       │
│    LLM Tool Gateway (User Prompts)│   │    Domain Services             │
│    Scheduled Agent Runtime [TODO] │   │    Value Objects               │
│    Context Assemblers             │   │    Domain Events               │
└───────────────────────┬───────────┘   └──────────┬─────────────────────┘
                        │                          │
                        └─────────────┬────────────┘
                                      │ Persists & Schedules
                                      ▼
┌────────────────────────────────────────────────────────────────────────┐
│ 5. JOB & EVENT SUBSYSTEM                                               │
│    Durable Jobs (SKIP LOCKED)  •  Transactional Outbox                 │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ Implements via adapters
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│ 6. DATA & INFRASTRUCTURE LAYER                                         │
│    PostgreSQL Repositories  •  S3 Storage  •  LLM & Chat Clients       │
└────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Layer Responsibilities & Invariants

1. **Interface Layer (`src/interface/`):**
   * Translates incoming platform-specific payloads (Matrix sync events, Discord gateway interactions, Telegram bot updates, HTTP REST calls, MCP JSON-RPC) into normalized **Application Commands and Queries**.
   * Formats domain responses into channel-specific formats (Matrix HTML, Discord Embeds/Components, Telegram MarkdownV2/Keyboards, Axum JSON, SSE payloads).
   * **Rule:** Zero business logic; zero direct SQL execution.
2. **Application Layer (`src/application/`):**
   * Implements specific use cases (e.g., `CreateTodoCommand`, `RecordExpenseCommand`, `ExecuteTradeCommand`).
   * Manages transaction boundaries (`sqlx::Transaction<'_, Postgres>`).
   * Dispatches domain events to the Transactional Outbox.
   * Enforces cross-domain orchestration (e.g., receipt image uploaded via Telegram -> enqueue OCR job -> parse expense -> notify user on Telegram/Matrix).
3. **LLM & Agent Layer (`src/llm/`; the scheduled agent later adds `src/llm/agent/`):**
   * **Live User Input:** Handled strictly via the **LLM Tool Gateway** for fast, single-turn intent extraction and direct tool execution via the Unified Tool Registry. Interactive user input never invokes an Agent runtime.
   * **Scheduled Autonomous Tasks [TODO Feature]:** An extensible **Scheduled Agent Runtime** executing multi-step background workflows (e.g. weekly digest, monthly audit) on configured cron schedules.
   * Maintains the **Unified Tool Registry**, exposing domain capabilities to LLMs and scheduled Agents as standard schemas.
   * Assembles dynamic system prompts combining user preferences, room context, and permission filters.
4. **Domain Layer (`src/domain/`):**
   * Encapsulates enterprise business rules, entities, and value objects (e.g., `Money`, `Holding`, `RecurrenceSchedule`).
   * Expresses operations as pure domain methods enforcing business invariants (e.g., "cannot buy securities without sufficient cash balance").
   * **Rule:** Completely decoupled from web frameworks, chat SDKs, and database libraries.
5. **Job & Event Subsystem (`src/jobs/` and `src/events/`):**
   * Houses the durable PostgreSQL job queue, worker pool orchestration, retry policies, and dead-letter handling.
   * Implements the Transactional Outbox pattern with `LISTEN` / `NOTIFY` and poll fallback for guaranteed event delivery.
6. **Data & Infrastructure Layer (`src/infrastructure/`):**
   * Implements repositories as concrete PostgreSQL modules (free functions over `impl PgExecutor`, compile-checked `query!`). There is one database, so there is no repository trait; traits exist only where a second implementation exists or is planned (LLM, STT, TTS, OCR, `ChannelAdapter`, `StorageService`).
   * Manages external clients: S3/R2 storage, Matrix homeserver client, Discord Serenity client, Telegram Teloxide client, LLM providers, and local/remote STT/TTS/OCR engines.

---

## 4. PostgreSQL Database Architecture & Schema Specification

All relational, unstructured (JSONB), and queuing state resides in PostgreSQL 16+.

### 4.1 Primary Database Standards

* **Source of truth:** `backend/migrations/*.sql`. This section describes intent and conventions; it deliberately does not duplicate column lists, which drift.
* **Row identifiers:** `BIGINT GENERATED BY DEFAULT AS IDENTITY` (existing `BIGSERIAL` columns are equivalent). Chat commands display these ids (`!todo done 3`), so they stay short integers.
* **Account identifiers:** `accounts.id` and every `owner_id` are native `UUID`, each `owner_id` a foreign key to `accounts(id) ON DELETE CASCADE`. In Rust this is `AccountId(Uuid)`, which crosses the HTTP/MCP boundary as the existing `acct_<uuid>` string, so clients see no change. The portfolio audit hash chain keeps hashing the `acct_<uuid>` form so existing chains still verify.
* **Timestamps:** `TIMESTAMPTZ`, read as `chrono::DateTime<Utc>`. Remaining `TEXT` timestamp columns are converted one at a time, each in its own migration alongside the code that reads it.
* **Money:** `BIGINT` minor units plus a `VARCHAR(3)` ISO 4217 code — already the case for `transactions.amount_minor`, `portfolio_*.*_minor` and `recurring_bills`.
* **Booleans / payloads:** native `BOOLEAN`; `JSONB` with `GIN` indexes where queried.
* **Ownership invariant:** every owner-keyed table is listed in `OWNED_TABLES` (`accounts/merge.rs`) so an account merge moves it; `portfolio_audit_log` is moved by its dedicated routine.

### 4.2 Table Inventory

| Domain | Tables | Notes |
|---|---|---|
| Accounts & auth | `accounts`, `matrix_links`, `matrix_link_challenges`, `webauthn_credentials`, `sessions`, `passkey_enrollment_tokens`, `recovery_codes`, `recovery_requests`, `security_events`, `web_users` | Session cookies stored as SHA-256 only; `matrix_links` generalises to `platform_identities` in Phase 4 |
| Settings | `user_settings` | Missing row means defaults |
| Rooms & chat state | `account_rooms`, `bot_message_targets` | Room resolution lives only in the rooms repository |
| Todos & reminders | `todos`, `reminders` | Reminders become `reminder.fire` jobs (§5) |
| Finance | `transactions`, `finance_categories`, `finance_shares`, `recurring_bills` | `amount_minor` is positive; `direction` carries the sign |
| Portfolio | `portfolio_holdings`, `portfolio_cash_balances`, `portfolio_audit_log`, `portfolio_price_alerts`, `portfolio_daily_snapshots` | Audit log is append-only (triggers) and hash-chained |
| Knowledge & life | `bookmarks`, `habits`, `habit_logs`, `life_profiles`, `life_day_logs`, `footprints` | |
| AI | `ai_user_configs`, `ai_conversations`, `ai_messages` | |
| Integrations | `mcp_tokens`, `webhooks` | `webhook_outbox` is replaced by `outbox_deliveries` (§6) |
| Platform (new) | `jobs`, `outbox_events`, `outbox_deliveries` | Sub-project 1 (§5, §6) |
| Agent (new, Phase 5) | `scheduled_agent_tasks`, `agent_runs`, `agent_steps` | §8.2 |

### 4.3 Migration Rules

* Forward-only SQL files, embedded with `sqlx::migrate!()` and applied at boot before any task starts.
* Never edit a migration that has shipped; add a new one.
* Any migration that rewrites data or types is rehearsed against a restored `pg_dump` of production (`MIGRATE_TEST_DATABASE_URL=… cargo test migrates_a_real_database -- --ignored`), asserting unchanged row counts and verified audit chains.
* A data-dependent migration starts with a guard (`DO $$ … RAISE EXCEPTION`) that names the offending rows instead of failing halfway.

---

## 5. Durable PostgreSQL Job Queue Engine

The system uses PostgreSQL's native row locking (`FOR UPDATE SKIP LOCKED`) to implement a robust, persistent, multi-worker background job queue. External queues (Redis/Celery/RabbitMQ) are strictly avoided.

### 5.1 Job Schema

```sql
CREATE TABLE jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID REFERENCES accounts(id) ON DELETE CASCADE,
    job_type        TEXT NOT NULL,
    payload         JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending' 
                    CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    priority        SMALLINT NOT NULL DEFAULT 0,
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    backoff_seconds INTEGER NOT NULL DEFAULT 10,
    available_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_at       TIMESTAMPTZ,
    locked_by       TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    heartbeat_at    TIMESTAMPTZ,           -- renewed every 30s while a handler runs
    dedupe_key      TEXT,                  -- e.g. '<account>:2026-09-24' for a daily briefing
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_jobs_dedupe UNIQUE (job_type, dedupe_key)
);

CREATE INDEX idx_jobs_dequeue ON jobs (priority DESC, available_at ASC)
WHERE status = 'pending';

CREATE INDEX idx_jobs_running ON jobs (heartbeat_at)
WHERE status = 'running';
```

### 5.2 Atomic Job Dequeue & Worker Protocol

Workers poll or wake on `LISTEN` and acquire jobs using an atomic query:

```sql
WITH next_job AS (
    SELECT id
    FROM jobs
    WHERE status = 'pending'
      AND available_at <= NOW()
    ORDER BY priority DESC, available_at ASC
    FOR UPDATE SKIP LOCKED
    LIMIT 1
)
UPDATE jobs
SET status = 'running',
    locked_at = NOW(),
    locked_by = $1, -- worker identifier (hostname:pid:uuid)
    started_at = NOW(),
    attempts = attempts + 1,
    updated_at = NOW()
FROM next_job
WHERE jobs_id = next_job.id
RETURNING jobs_*;
```

### 5.3 Error Handling, Backoff, and Dead-Letter Protocol

* **Retryable Failure:** If `attempts < max_attempts`, apply exponential backoff:
  `available_at = NOW() + (backoff_seconds * 2^(attempts - 1))`. Set `status = 'pending'`, clear lock.
* **Terminal Failure:** If `attempts >= max_attempts`, set `status = 'failed'`, set `completed_at = NOW()`, record `error`, and emit an alert event via the Transactional Outbox.
* **Stuck Worker Reaper:** A periodic background task reclaims jobs where `status = 'running'` and `heartbeat_at < NOW() - INTERVAL '2 minutes'`. A heartbeat, not a fixed `locked_at` cutoff, so a long TTS job is never taken from a live worker.
* **Idempotent enqueue:** scheduled work is enqueued with a `dedupe_key` and `ON CONFLICT (job_type, dedupe_key) DO NOTHING`, so a scheduler tick that runs twice enqueues once.
* **Idempotent handlers:** a handler may run more than once (crash after a side effect, before commit); it receives the job id to use as an idempotency key.
* **Scheduler leadership:** the loop that enqueues recurring work runs only while holding `pg_try_advisory_lock`; so does the Matrix sync loop. A second replica serves HTTP and runs workers without double-sending.

---

## 6. Transactional Outbox Pattern & Omnichannel Notification Subsystem

To guarantee that state changes reliably produce Matrix messages, Discord alerts, Telegram updates, Web SSE events, and audit logs without distributed transaction inconsistency, the system implements the **Transactional Outbox Pattern**.

### 6.1 Outbox Schema

```sql
CREATE TABLE outbox_events (
    id              BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    event_type      TEXT NOT NULL,
    payload         JSONB NOT NULL,          -- serialised SystemEvent
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- One row per durable destination; each retried independently.
CREATE TABLE outbox_deliveries (
    id              BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    event_id        BIGINT NOT NULL REFERENCES outbox_events(id) ON DELETE CASCADE,
    destination     TEXT NOT NULL,           -- 'webhook:<id>', 'matrix:<room>', later 'discord:<channel>'
    status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'failed')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 8,
    available_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivered_at    TIMESTAMPTZ,
    last_error      TEXT,
    CONSTRAINT uq_delivery UNIQUE (event_id, destination)
);

CREATE INDEX idx_outbox_deliveries_pending ON outbox_deliveries (available_at) WHERE status = 'pending';
```

* **SSE is not a delivery row.** Every instance `LISTEN`s on `outbox`, loads the committed event and pushes it to its own connected SSE clients for that `account_id`. A browser that misses events re-fetches on reconnect.
* **Durable destinations** (webhooks, chat notifications) each get a delivery row, claimed with `SKIP LOCKED` and retried with exponential backoff. The delivery id is sent as `X-Delivery-Id` so receivers can deduplicate.
* **Retention:** delivered events older than 30 days are deleted by a daily scheduled job.

### 6.2 Omnichannel Outbox Processing Lifecycle

```text
┌────────────────────────────────────────────────────────────────────────┐
│                      Application Service Mutation                      │
│                                                                        │
│  BEGIN TRANSACTION;                                                    │
│    1. INSERT INTO transactions (...) VALUES (...);                     │
│    2. UPDATE portfolio_cash_balances SET ...;                         │
│    3. INSERT INTO outbox_events (                                      │
│         account_id, event_type, payload, target_platform               │
│       ) VALUES (...);                                                  │
│    4. PERFORM pg_notify('outbox_channel', id::text);                   │
│  COMMIT;                                                               │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ (Zero chance of lost event)
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                        Outbox Dispatcher Daemon                        │
│                                                                        │
│  1. Woken by LISTEN outbox_channel (or 1s timer fallback)              │
│  2. SSE: load event by id, push to this instance's clients (no row)    │
│     Durable: SELECT ... FROM outbox_deliveries WHERE status='pending'  │
│     FOR UPDATE SKIP LOCKED LIMIT 50;                                   │
│  3. Multi-Channel Fan-out:                                             │
│     ├── Broadcast to Axum Web SSE channel                              │
│     ├── If Matrix linked: Send HTML message to Matrix DM/Room          │
│     ├── If Discord linked: Send Embed to Discord DM/Channel            │
│     ├── If Telegram linked: Send Markdown to Telegram Chat             │
│     └── Trigger external webhooks                                      │
│  4. Per delivery: status='delivered', or backoff / 'failed'            │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 7. Unified Tool Registry & Execution Architecture: Interactive User Input (LLM + Tools) vs. Scheduled Autonomous Agent [TODO Feature]

The system decouples tool execution from specific LLM providers and user interfaces. A single **Unified Tool Registry** serves Matrix, Discord, Telegram, the Web LLM console, MCP servers, and scheduled autonomous Agent workflows.

### 7.1 Interactive User Input Execution: Direct LLM + Tool Calling (Active)

The platform strictly separates real-time conversational messaging from autonomous agent loops:

> **Live user input is handled exclusively through Deterministic CLI commands or Direct LLM + Tool Calling (bounded tool loop). The Agent Runtime is NEVER invoked on interactive user input.**

| Dimension | Interactive User Input (Direct LLM + Tools) | Scheduled Autonomous Tasks (Agent Runtime [TODO]) |
|---|---|---|
| **Trigger Source** | Live user chat message (Matrix, Discord, Telegram, Web, MCP) | Automated PostgreSQL cron / timer schedule |
| **Execution Engine** | `LlmToolGateway` (single-pass tool selection) | `ScheduledAgentRuntime` (multi-step ReAct loop worker) |
| **Turns / Latency** | Bounded tool loop (≤ `MAX_TOOL_ROUNDS`), seconds at most | Multi-turn iterations (seconds to minutes, background) |
| **State Tracking** | Ephemeral request context; direct domain write | Persistent in `agent_runs` & `agent_steps` (PostgreSQL) |
| **Token Cost** | Single prompt + single completion (predictable, minimal) | Multi-turn prompt accumulation (budget-capped) |
| **Failure Mode** | Returns explicit domain error message to user | Autonomous retry, alternate tool selection, or error notification |
| **Primary Use Cases** | Adding an expense, creating a task, setting a reminder, QA | Weekly digest briefing, monthly financial audit, portfolio rebalance check |

#### 7.1.1 Interactive User Input Flow (Direct LLM + Tool Calling)
For all real-time user requests (e.g. *"I spent $15 on lunch"*, *"What's on my agenda today?"*, *"Mark task 3 done"*, *"What is the stock price of AAPL?"*):

```text
User Message: "Add 15 dollars for lunch"
  │
  ▼
1. Chat Router extracts message envelope and resolves Account Identity
  │
  ▼
2. LLM Tool Gateway receives prompt + domain tool schemas from Tool Registry
  │
  ▼
3. LLM emits single direct tool call: finance_record_expense(amount=15, category="food")
  │
  ▼
4. ToolHandler invokes Application Service: FinanceService::create_expense(...)
  │
  ▼
5. PostgreSQL Transaction: INSERT INTO transactions + INSERT INTO outbox_events
  │
  ▼
6. Immediate Confirmation: "Recorded $15.00 under Food." (one request, no background state)
```

#### 7.1.2 Rationale for Omitting Agent Loops from Live User Input
1. **Low Latency:** Chat users expect rapid, deterministic confirmations. Multi-turn autonomous agent loops incur multi-second to minute-long delays.
2. **Determinism & Safety:** Routine personal management actions (e.g., logging an expense, ticking a task) require strict ACID guarantees, not probabilistic multi-turn reasoning chains.
3. **Transparent Error Handling:** When a business invariant fails (e.g., insufficient cash for a trade), returning a clear, deterministic domain error allows the user to decide the next step, avoiding unprompted autonomous actions.
4. **Predictable Token Economics:** Eliminates unbounded token consumption on open-ended conversational prompts.

---

### 7.2 Scheduled Autonomous Tasks: Extensible Agent Runtime [TODO Feature]

The Agent Runtime is **exclusively reserved for specific scheduled tasks** that require autonomous multi-step planning, data aggregation across multiple domains, background worker job coordination, and synthesis.

> **[TODO Feature — Phase 5 Roadmap]**  
> The Scheduled Agent Runtime is a planned background worker architecture. It is triggered by time-based schedules in PostgreSQL, not by interactive chat messages.

#### 7.2.1 Scheduled Agent Workflow Pattern
For recurring background objectives (e.g. *"Every Sunday at 9:00 PM, compile my unread bookmarks, generate a synthesized brief, enqueue a Kokoro TTS audio note, and message it to me"*):

```text
PostgreSQL Cron Schedule Triggers: "weekly_bookmark_digest"
  │
  ▼
1. Durable Job Worker claims scheduled task from `scheduled_agent_tasks` (SKIP LOCKED)
  │
  ▼
2. Worker instantiates ScheduledAgentRuntime with allowed task tools
  │
  ▼
3. Step 1 (Act): Tool Call -> bookmarks_list(unread=true, since="7 days ago")
   Result: [Bookmark A, Bookmark B, Bookmark C] logged to `agent_steps`
  │
  ▼
4. Step 2 (Think & Act): LLM Call -> Synthesize themes into concise 2-minute digest
   Result: Digest text logged to `agent_steps`
  │
  ▼
5. Step 3 (Act): Tool Call -> jobs_enqueue_tts(text=digest_script, voice="default")
   Result: Enqueued durable TTS job in PostgreSQL jobs table
  │
  ▼
6. Step 4 (Await): Worker waits for Kokoro TTS audio generation -> Audio saved to Storage
  │
  ▼
7. Step 5 (Notify): Write Outbox Event -> Dispatches voice note + markdown summary to user's Matrix/Discord/Telegram
  │
  ▼
8. AgentRun marked 'completed' in `agent_runs` table
```

#### 7.2.2 Extensible Scheduled Task Plugin Model
New scheduled autonomous workflows can be added as modular plugins implementing the `ScheduledAgentTask` trait without modifying core scheduling logic:
* **Weekly Digest Agent:** Gathers week's accomplishments and unread bookmarks -> voice briefing.
* **Monthly Financial Audit Agent:** Audits spending categories, flags budget overruns, and drafts savings targets.
* **Portfolio Rebalance Alert Agent:** Evaluates asset allocations against target weights and posts rebalancing alerts.
* **System Maintenance Agent:** Audits disk consumption, prunes expired tokens, and cleans up orphaned media.

---

### 7.3 Tool Definition Specification

Every tool is defined using a standard Rust contract adhering to the OpenAI Tool Specification:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolParameterSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    pub description: String,
    pub properties: serde_json::Map<String, serde_json::Value>,
    pub required: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameterSchema,
    pub is_sensitive: bool,          // Requires passkey step-up or explicit confirmation
    pub allowed_interfaces: Vec<InterfaceKind>, // Matrix, Discord, Telegram, Web, MCP, Agent
}

#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    async fn execute(
        &self,
        context: &ToolExecutionContext,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolExecutionError>;
}
```

### 7.4 Standardized Tool Modules

Tool names use `<namespace>_<action>` because OpenAI-compatible APIs reject `.` in function names.


| Namespace | Tool Name | Description | Sensitivity |
|---|---|---|---|
| `todo` | `todo_create` | Create a new task with due date and priority | Low |
| `todo` | `todo_list` | List tasks with filters (pending, completed, tags) | Low |
| `todo` | `todo_complete` | Mark a task as completed | Low |
| `finance` | `finance_record_expense` | Log an expense with amount, currency, and category | Low |
| `finance` | `finance_get_summary` | Aggregate spending by category for a time period | Low |
| `portfolio` | `portfolio_get_holdings` | Retrieve current asset positions and values | Low |
| `portfolio` | `portfolio_execute_trade` | Buy or sell securities/crypto | **High (Requires Confirmation)** |
| `reminders` | `reminders_schedule` | Create a time-based or recurring reminder | Low |
| `bookmarks` | `bookmarks_save` | Save and summarize a URL | Low |
| `bookmarks` | `bookmarks_search` | Search saved bookmarks with semantic or full-text | Low |
| `habits` | `habits_log` | Check in a habit completion | Low |
| `journal` | `journal_write` | Record a journal entry or thought | Low |
| `media` | `media_synthesize_speech` | Convert text into speech audio | Low |
| `search` | `search_web` | Live web search via Tavily / DuckDuckGo | Low |

---

## 8. Scheduled Autonomous Agent Runtime [TODO Feature] & Extensible Task Framework

> **[TODO Feature — Phase 5 Roadmap]**  
> The Agent Runtime is a **planned background automation feature** reserved **exclusively for scheduled, specific tasks**. It is **never** invoked to handle live user input. Live conversational interactions are handled strictly via deterministic CLI commands or direct LLM + Tool Calling (bounded tool loop).

### 8.1 Extensible Scheduled Task Architecture (`ScheduledAgentTask` Plugin Model)

The scheduled agent architecture is built around an open-closed plugin model. New automated background workflows can be created and registered without modifying the core scheduler, database schemas, or chat adapters:

```rust
#[derive(Clone, Debug)]
pub struct ScheduledTaskContext {
    pub account_id: Uuid,
    pub task_id: String,
    pub config: serde_json::Value,
    pub pool: sqlx::PgPool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub summary: String,
    pub steps_executed: usize,
    pub total_tokens: usize,
    pub artifacts: Vec<TaskArtifact>,
}

#[async_trait::async_trait]
pub trait ScheduledAgentTask: Send + Sync {
    /// Unique machine identifier for this task (e.g. "weekly_bookmark_digest")
    fn task_id(&self) -> &'static str;

    /// Human-readable display name
    fn name(&self) -> &'static str;

    /// Default cron expression if not overridden by the user (e.g. "0 21 * * SUN")
    fn default_cron(&self) -> &'static str;

    /// Permitted domain tools from the Unified Tool Registry (least-privilege sandbox)
    fn allowed_tools(&self) -> Vec<&'static str>;

    /// Synthesize initial planning prompt and goal based on current user context
    async fn build_goal(&self, ctx: &ScheduledTaskContext) -> Result<String, DomainError>;

    /// Post-process execution results and dispatch outbox notifications to linked channels
    async fn on_completed(
        &self,
        ctx: &ScheduledTaskContext,
        result: &AgentRunResult,
    ) -> Result<(), DomainError>;
}
```

#### 8.1.1 Pluggable Task Registry (`ScheduledTaskRegistry`)
The system provides a central registry where task plugins register at boot:

```rust
pub struct ScheduledTaskRegistry {
    tasks: HashMap<&'static str, Arc<dyn ScheduledAgentTask>>,
}

impl ScheduledTaskRegistry {
    pub fn new() -> Self {
        let mut registry = Self { tasks: HashMap::new() };
        // Pluggable built-in task registrations
        registry.register(Arc::new(WeeklyBookmarkDigestTask));
        registry.register(Arc::new(MonthlyFinancialAuditTask));
        registry.register(Arc::new(PortfolioRebalanceAlertTask));
        registry.register(Arc::new(SystemMaintenanceAuditTask));
        registry
    }

    pub fn register(&mut self, task: Arc<dyn ScheduledAgentTask>) {
        self.tasks.insert(task.task_id(), task);
    }

    pub fn get(&self, task_id: &str) -> Option<Arc<dyn ScheduledAgentTask>> {
        self.tasks.get(task_id).cloned()
    }
}
```

#### 8.1.2 Standard Scheduled Task Plugins
| Task Plugin | Default Schedule | Allowed Tools | Purpose |
|---|---|---|---|
| `WeeklyBookmarkDigestTask` | Every Sunday 21:00 UTC | `bookmarks_*`, `media_synthesize_speech` | Analyzes unread links from past 7 days, generates an audio briefing via Kokoro TTS, and posts to Matrix/Discord/Telegram. |
| `MonthlyFinancialAuditTask` | 1st of month 08:00 UTC | `finance_*`, `portfolio_*` | Aggregates prior month expenses, checks budget caps, flags anomalies, and drafts financial recommendations. |
| `PortfolioRebalanceAlertTask` | Every Monday 09:00 UTC | `portfolio_get_holdings`, `search_web` | Compares current holding allocations with user targets and posts rebalancing suggestions if drift exceeds 5%. |
| `SystemMaintenanceAuditTask` | Daily 03:00 UTC | *Internal DB queries* | Prunes expired sessions, purges stale OCR cache, and optimizes PostgreSQL tables. |

---

### 8.2 Scheduled Agent Persistence Schema

All scheduled task configurations, execution runs, and ReAct steps are persisted in PostgreSQL:

```sql
-- 1. User-configured or system-default scheduled task definitions
CREATE TABLE scheduled_agent_tasks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    task_id         TEXT NOT NULL, -- Matches ScheduledAgentTask::task_id()
    cron_expression TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    config          JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_run_at     TIMESTAMPTZ,
    next_run_at     TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_account_task UNIQUE (account_id, task_id)
);

CREATE INDEX idx_scheduled_agent_tasks_due 
    ON scheduled_agent_tasks(next_run_at) 
    WHERE enabled = TRUE;

-- 2. Stateful execution runs for scheduled tasks
CREATE TABLE agent_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scheduled_task_id   UUID REFERENCES scheduled_agent_tasks(id) ON DELETE SET NULL,
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    task_name           TEXT NOT NULL,
    goal                TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'planning'
                        CHECK (status IN ('planning', 'executing', 'completed', 'failed', 'cancelled')),
    max_steps           INTEGER NOT NULL DEFAULT 10,
    current_step        INTEGER NOT NULL DEFAULT 0,
    total_tokens        INTEGER NOT NULL DEFAULT 0,
    error               TEXT,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_runs_account ON agent_runs(account_id, started_at DESC);

-- 3. Granular ReAct steps (Reasoning + Acting + Observing)
CREATE TABLE agent_steps (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    step_number     INTEGER NOT NULL,
    thought         TEXT,
    tool_call       TEXT,
    tool_input      JSONB,
    tool_output     JSONB,
    status          TEXT NOT NULL CHECK (status IN ('success', 'failed', 'blocked')),
    tokens_used     INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_run_step UNIQUE (run_id, step_number)
);

CREATE INDEX idx_agent_steps_run ON agent_steps(run_id, step_number ASC);
```

### 8.3 Background Worker Execution Lifecycle

```text
PostgreSQL Scheduler Tick (Every 60s)
  │
  ▼
1. Query Due Tasks:
   SELECT * FROM scheduled_agent_tasks
   WHERE enabled = TRUE AND next_run_at <= NOW()
   FOR UPDATE SKIP LOCKED;
  │
  ▼
2. Resolve Task Plugin from ScheduledTaskRegistry via `task_id`
  │
  ▼
3. Call `task.build_goal(&ctx)` -> Insert row into `agent_runs`
  │
  ▼
4. Run ReAct Loop (Think -> Act -> Observe):
   ├── Check step limit (max_steps <= 10)
   ├── Send conversation history + allowed tool schemas to LLM
   ├── Parse tool call, invoke domain repository within PostgreSQL transaction
   ├── Store step in `agent_steps`
   └── Repeat until task goal is satisfied or step limit reached
  │
  ▼
5. On Completion:
   ├── Invoke `task.on_completed(&ctx, &result)`
   ├── Insert Outbox Event (`outbox_events`) to broadcast briefing to user's chat channels
   ├── Compute next cron instant and UPDATE `scheduled_agent_tasks.next_run_at`
   └── UPDATE `agent_runs SET status = 'completed', completed_at = NOW()`
```

---

## 9. Interface Layer Specifications: Omnichannel Architecture

The Interface Layer is fully decoupled from the core business logic using an **Omnichannel Chat Adapter Architecture**. Each chat platform (Matrix, Discord, Telegram) translates incoming network events into a normalized message envelope, and renders outgoing domain models into platform-native presentations.

```text
┌────────────────────────────────────────────────────────────────────────┐
│                        Omnichannel Message Bus                         │
│                                                                        │
│   IncomingMessage  ──────►  Chat Router  ──────►  Application Services │
│          ▲                        │                        │           │
│          │                        ▼                        ▼           │
│   Channel Adapters        OutgoingMessage      Transactional Outbox    │
└──────────┬────────────────────────┬────────────────────────┬───────────┘
           │                        │                        │
     ┌─────┴──────────┐       ┌─────┴──────────┐       ┌─────┴──────────┐
     ▼                ▼       ▼                ▼       ▼                ▼
┌─────────┐      ┌─────────┐┌─────────┐   ┌─────────┐┌─────────┐   ┌─────────┐
│ Matrix  │      │ Discord ││Telegram │   │ Matrix  ││ Discord │   │Telegram │
│ Inbound │      │ Inbound ││ Inbound │   │Outbound ││Outbound │   │Outbound │
└─────────┘      └─────────┘└─────────┘   └─────────┘└─────────┘   └─────────┘
```

### 9.1 Unified Chat Channel Contract

All chat platforms implement a common abstraction in `src/interface/channels/`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatPlatform {
    Matrix,
    Discord,
    Telegram,
    Slack,
}

#[derive(Clone, Debug)]
pub struct IncomingMessage {
    pub platform: ChatPlatform,
    pub platform_user_id: String,     // MXID, Discord Snowflake, Telegram User ID
    pub channel_id: String,          // Room ID, Guild/DM Channel ID, Chat ID
    pub message_id: String,
    pub content: String,
    pub sender_display_name: Option<String>,
    pub is_direct_message: bool,
    pub is_bot_mentioned: bool,
    pub quoted_message: Option<QuotedContext>,
    pub attachments: Vec<MediaAttachment>,
}

#[derive(Clone, Debug)]
pub struct OutgoingMessage {
    pub text: String,
    pub formatted_html: Option<String>,
    pub discord_embed: Option<DiscordEmbedPayload>,
    pub telegram_parse_mode: Option<TelegramParseMode>,
    pub reply_to_message_id: Option<String>,
    pub attachments: Vec<MediaAttachment>,
    pub interactive_components: Vec<ComponentRow>,
}

#[async_trait::async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn platform(&self) -> ChatPlatform;
    async fn start(&self, shutdown: tokio::sync::broadcast::Receiver<()>) -> Result<(), ChannelError>;
    async fn send_message(&self, channel_id: &str, message: OutgoingMessage) -> Result<String, ChannelError>;
    async fn send_voice(&self, channel_id: &str, voice_payload: VoicePayload) -> Result<String, ChannelError>;
    async fn edit_message(&self, channel_id: &str, message_id: &str, message: OutgoingMessage) -> Result<(), ChannelError>;
    async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<(), ChannelError>;
}

/// Extensible Channel Registry: dynamic registration of chat adapters
pub struct ChannelRegistry {
    adapters: HashMap<ChatPlatform, Arc<dyn ChannelAdapter>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self { adapters: HashMap::new() }
    }

    pub fn register(&mut self, adapter: Arc<dyn ChannelAdapter>) {
        self.adapters.insert(adapter.platform(), adapter);
    }

    pub fn get(&self, platform: &ChatPlatform) -> Option<Arc<dyn ChannelAdapter>> {
        self.adapters.get(platform).cloned()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn ChannelAdapter>> {
        self.adapters.values()
    }
}
```

#### 9.1.1 Extensible Channel Plugin Architecture
The system treats every chat platform as an independent, modular plugin:
* Adding a new platform (e.g. Discord, Telegram, Slack, WhatsApp, Signal) requires only:
  1. Defining the platform in `ChatPlatform`.
  2. Implementing the `ChannelAdapter` trait.
  3. Registering the adapter into `ChannelRegistry` at boot.
* **Invariants:** Zero changes to Application Services, Domain entities, or existing Channel Adapters. All platform-specific network quirks, rate limits, authentication handshakes, and UI presentation details stay strictly encapsulated inside each adapter.

---

### 9.2 Matrix Adapter Specification (Active / Current Implementation)

* **Status:** **Active / Currently Implemented** (refactoring to `ChannelAdapter` in Phase 4).
* **Engine:** `matrix-sdk` with its crypto state in the SDK-managed SQLite store under `data/` (not the application database). Never kept across a fresh login; never pinned to a device ID.
* **Core Capabilities:**
  * **End-to-End Encryption (E2EE):** Megolm / Olm session management, device verification, cross-signing auto-verification via recovery key.
  * **Event Handling:** Timeline sync listener (`SyncSettings`), handling `m.room.message` (text, emote, image, audio, file), `m.reaction`, and room membership events.
  * **Quoted Context:** Parses `<mx-reply>` rich tags and plain text `> quote` fallbacks into `QuotedContext`.
  * **Reply Rendering:** Rich HTML formatting with `html_escape()` and clean plain-text fallback via `strip_html()` so push notifications render legibly.
  * **Voice Messages:** Dispatches Kokoro TTS synthesized Ogg Opus files as native Matrix voice notes with MSC3245 audio and waveform metadata.
* **Identity Mapping:** Resolves sender MXID (`@username:homeserver.tld`) against `platform_identities`.

---

### 9.3 Discord Adapter Specification ([TODO Feature] — Planned Extension)

> **[TODO Feature — Phase 4 Roadmap]**  
> Discord support is a **planned extension feature** to be built on top of the `ChannelAdapter` interface after SQLite removal, domain extraction, and the durable job queue are complete. The specification below defines its target architecture, slash command registry, and interaction model.

The Discord integration provides deep native Discord features, turning the bot into a first-class Discord Application using **`serenity`** and Discord Gateway / REST APIs.

#### 9.3.1 Discord Client & Gateway Architecture
* **Crate:** `serenity` with `tokio` async runtime.
* **Gateway Intents:** `GatewayIntents::GUILD_MESSAGES | GatewayIntents::DIRECT_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILD_MESSAGE_REACTIONS`.
* **Connection Lifecycle:** Runs in a background Tokio task supervised by the main daemon with automatic reconnection, session resumption, and exponential backoff on disconnect.

#### 9.3.2 Slash Commands (Application Commands)
The bot registers global Application Commands (or guild-scoped commands during development for immediate updates):

| Slash Command | Options / Arguments | Description |
|---|---|---|
| `/todo add` | `title: String`, `due: Option<String>`, `priority: Option<Integer>` | Create a new task |
| `/todo list` | `filter: Option<Choice['all', 'pending', 'done']>` | View task list |
| `/todo done` | `task_id: String` (Autocomplete) | Complete a task |
| `/expense add` | `amount: Number`, `category: String` (Autocomplete), `description: String` | Log an expense |
| `/expense summary` | `period: Option<Choice['today', 'month', 'year']>` | View spending summary |
| `/portfolio list` | *None* (Enforces ephemeral or DM response) | View investment holdings & P&L |
| `/portfolio buy` | `symbol: String`, `quantity: Number`, `price: Number` | Execute a stock/crypto buy |
| `/remind` | `time: String`, `message: String` | Set a reminder |
| `/say` | `text: String`, `voice: Option<String>` | Synthesize voice audio |
| `/ask` | `prompt: String` | Ask conversational LLM |
| `/link` | *None* (DM Only) | Initiate WebAuthn account linking |

#### 9.3.3 Interactive Message Components & Modals
* **Buttons (ActionRow):**
  * Action confirmations: "Confirm Trade" (Green) / "Cancel" (Red).
  * Quick task toggles: "Mark Done", "Snooze 1h".
* **Select Menus:** Category selector for ambiguous receipt scans.
* **Modals:** Pop-up forms for multiline journal entries and budget configurations.

#### 9.3.4 Embeds & Formatting Design System
* Follows the project's minimalist Geist design system:
  * **Color Palette:** Neutral slate/gray for standard output; Emerald for positive P&L / completed tasks; Rose for errors / debit expenses; Amber for warnings / approval gates.
  * **Tabular Numbers:** Monetary amounts, quantities, and timestamps formatted in monospace inline codeblocks.
  * **Stat Strips:** Summary metrics rendered as inline embed fields (`grid-like`).

#### 9.3.5 Channel Privacy Gating
* **Public Guild Channels:** Sensitive financial operations (`/portfolio`, `/expense list`) are either automatically set to `ephemeral: true` (only visible to the command invoker) or refused with a prompt to use Direct Messages.
* **Direct Messages (DMs):** Unrestricted access to all personal workspace features, continuous conversational LLM interaction with direct tool calling, and receipt photo uploads.

---

### 9.4 Telegram Adapter Specification ([TODO Feature] — Planned Extension)

> **[TODO Feature — Phase 4 Roadmap]**  
> Telegram support is a **planned extension feature** to be built in Phase 4 alongside Discord. The specification below defines its webhook architecture, command bindings, inline keyboard interaction, and media ingestion pipeline.

The Telegram integration supports both modern cloud serverless webhooks and local development polling using **`teloxide`**.

#### 9.4.1 Client Architecture & Deployment Modes
* **Production Mode (Axum Webhook):**
  * Telegram sends updates via HTTPS `POST` to `/api/webhooks/telegram`.
  * Verified using the `X-Telegram-Bot-Api-Secret-Token` header.
  * Zero long-polling overhead, instant execution, and seamless integration with the existing Axum reverse-proxy (Caddy).
* **Development Mode (Long Polling):**
  * Standalone `teloxide::dispatching::Dispatcher` for local testing without public IP or ngrok tunnels.

#### 9.4.2 Bot Commands & Menu Integration
* Commands are registered with Telegram via `setMyCommands` API:
  * `/start` — Welcome message, account link status, and getting started guide.
  * `/todo` — Task management (add, list, check off).
  * `/expense` — Personal finance logging and summaries.
  * `/portfolio` — Asset holdings, cash balances, and trades.
  * `/remind` — Natural language reminders.
  * `/say` — Voice message synthesis.
  * `/link` — Secure web console account linking ceremony.
  * `/help` — Command cheatsheet and documentation.

#### 9.4.3 Inline Keyboards & Callback Queries
* **Callback Query Dispatcher:** Handles interactive button presses without sending new chat messages.
  * Interactive Todo Checklists: Users tap `[✓ Done]` directly under a task item.
  * Trade Confirmation Prompts: `[Confirm Buy]` / `[Cancel]` buttons for high-value actions.
  * Pagination Controls: `[◄ Prev]` / `[Next ►]` for browsing transaction history.

#### 9.4.4 Media Ingestion & Native Voice Notes
* **Voice Notes:** Text-to-speech replies are delivered via Telegram's `sendVoice` API, rendering as interactive audio bubbles with waveform visualization.
* **Photo Receipt Ingestion:**
  * When a user sends a photo or document receipt in Telegram DM:
  * Bot downloads the highest resolution `PhotoSize` via `getFile`.
  * Streams binary to Object Storage (`StorageService`).
  * Enqueues an OCR parsing job in PostgreSQL.
  * Sends an immediate acknowledgement: *"Receipt received. Processing with OCR..."*
  * Once parsed, edits the message or sends confirmation with parsed merchant and amount.

#### 9.4.5 Group vs Private Chat Context
* **Private Chat (1-on-1):** Full conversational workspace mode. Interactive requests execute directly via deterministic CLI or LLM + Tool Calling (bounded tool loop).
* **Group Chats:** Privacy mode enabled. The bot ignores background room traffic and only activates when explicitly mentioned (`@bot_username`) or when a reply targets one of its previous messages.

---

### 9.5 Omnichannel Account Linking Ceremony (`/link`)

A core strength of the architecture is that **one user account (`accounts.id`) links their Matrix, Discord, and Telegram identities** into a single personal workspace:

```text
User on Discord or Telegram sends: /link (in private DM)
  │
  ▼
1. Bot verifies interaction is in a private Direct Message:
   - If in public room: Reject immediately & burn token!
  │
  ▼
2. System mints single-use cryptographic token:
   Token: mb_link_7f8a9b2c3d4e (TTL: 10 minutes)
   Stored hashed in PostgreSQL (generalised from matrix_link_challenges)
  │
  ▼
3. Bot replies with secure link:
   "Click here to link this account to your Personal Life Workspace:
    https://mybot.domain/link?code=mb_link_7f8a9b2c3d4e"
  │
  ▼
4. User clicks link and opens Web Console in browser:
   - Browser prompts for WebAuthn passkey assertion.
   - User verifies biometrics (TouchID / FaceID / YubiKey).
  │
  ▼
5. Server binds identity in PostgreSQL:
   INSERT INTO platform_identities (
     account_id, platform, platform_user_id, display_name
   ) VALUES (
     $auth_account_id, 'telegram', '987654321', 'Alice'
   );
  │
  ▼
6. Bot sends confirmation message back on Telegram/Discord:
   "✓ Telegram account successfully linked! Your workspace is synchronized."
```

---

### 9.6 Cross-Platform Format Normalization & Media Bridge

To ensure domain services write output once and render beautifully everywhere, the system utilizes a **Format Normalization Engine**:

| Semantic Element | Matrix Presentation | Discord Presentation | Telegram Presentation | Web UI Presentation |
|---|---|---|---|---|
| **Bold Text** | `<b>text</b>` | `**text**` | `<b>text</b>` | `<strong>text</strong>` |
| **Monospace / Amount** | `<code>$42.50</code>` | `` `$42.50` `` | `<code>$42.50</code>` | `<span class="font-mono">$42.50</span>` |
| **Blockquotes** | `<blockquote>quote</blockquote>` | `> quote` | `<blockquote>quote</blockquote>` | `<blockquote class="border-l-2">` |
| **Voice Audio** | Native Ogg Opus + MSC3245 | Audio file attachment | `sendVoice` Voice Bubble | HTML5 `<audio>` player |
| **Interactive Card** | HTML table + Action links | Rich Embed + ActionRow Buttons | Message + Inline Keyboard | React Component Card |
| **User Mention** | `<a href="https://matrix.to/#/@id">Pill</a>` | `<@snowflake>` | `tg://user?id=123` | User Badge |

---

### 9.7 Web REST & SSE API (Axum)

* **Endpoints:** Full REST API under `/api/*` for todos, expenses, portfolio, bookmarks, habits, journal, footprints, LLM chat, and jobs.
* **Real-Time Streaming:** `/api/events` provides a per-account SSE stream for live UI synchronization.
* **OpenAPI Documentation:** Auto-generated via `utoipa`.

---

### 9.8 MCP Server (Model Context Protocol)

* **Transports:** `stdio` mode for Cursor / Claude Desktop and HTTP/SSE for remote agents.
* **Authentication:** Authenticated via Personal Access Tokens (`mb_mcp_` prefix) mapped to an `account_id`.
* **Access Control:** Exposes domain capabilities from the Unified Tool Registry.

---

## 10. External Media & Inference Services (STT, TTS, OCR)

To maintain lean backend execution, resource-intensive machine learning models are isolated from the main bot process.

```text
┌──────────────────┐               ┌─────────────────────────────────────┐
│   Matrix Bot     │   HTTP REST   │   matrix-bot-inference Server       │
│   (Rust / Axum)  ├──────────────►│   (Local Axum / ONNX Runtime)       │
│                  │               │                                     │
│  services/stt    │               │  POST /v1/audio/transcriptions (STT)│
│  services/tts    │               │  POST /v1/audio/speech (Kokoro TTS) │
│  services/ocr    │               │  POST /v1/vision/ocr (PaddleOCR)    │
└──────────────────┘               └─────────────────────────────────────┘
```

### 10.1 Service Abstractions

1. **Speech-to-Text (`SttProvider` trait):**
   * Default: Bundled inference server running `sherpa-onnx` (SenseVoice / Whisper).
   * Alternate: OpenAI Whisper API, Groq Cloud Whisper.
2. **Text-to-Speech (`TtsProvider` trait):**
   * Default: Bundled inference server running `Kokoro-82M` emitting native Ogg Opus.
   * Alternate: ElevenLabs API, OpenAI TTS.
3. **Optical Character Recognition (`OcrProvider` trait):**
   * Default: Bundled inference server running ONNX OCR.
   * Capability: Extracts raw text and bounding boxes from receipt and document images.
4. **LLM Gateway (`LlmGateway` trait):**
   * Supports any OpenAI-compatible completions API (OpenAI, Anthropic via bridge, Ollama, vLLM, DeepSeek).
   * Supports streaming token responses via Server-Sent Events.

### 10.2 Omnichannel Receipt Ingestion Pipeline

```text
User uploads receipt photo (over Matrix, Discord, or Telegram)
  │
  ▼
1. Platform Channel Adapter receives media event & downloads binary stream
  │
  ▼
2. Binary stream uploaded to Object Storage (S3 / R2 / Local)
   Storage Key: accounts/{id}/receipts/{uuid}.jpg
  │
  ▼
3. Enqueue Durable Job in PostgreSQL:
   job_type: "media.process_receipt"
   payload: { "storage_key": "...", "account_id": "...", "origin_platform": "telegram", "channel_id": "..." }
  │
  ▼
4. Job Worker claims task via SKIP LOCKED:
   a. Calls OCR Service to extract text lines
   b. Calls LLM with structured receipt prompt:
      Extract: Merchant, Total Amount, Currency, Category, Date
   c. Invokes FinanceService::create_expense(...)
   d. Links transaction to receipt_storage_key
  │
  ▼
5. Emits Outbox Event:
   ReceiptProcessed { transaction_id, amount, merchant, target_platform, channel_id }
  │
  ▼
6. Outbox Dispatcher delivers confirmation to the originating chat platform:
   "Receipt recorded: $42.50 at Trader Joe's (Groceries)"
```

---

## 11. Object Storage Architecture

Unstructured binary media (audio recordings, images, receipts, document attachments) are never stored as database blobs.

### 11.1 Storage Trait Contract

```rust
#[async_trait::async_trait]
pub trait StorageService: Send + Sync {
    async fn put(&self, key: &str, data: bytes::Bytes, content_type: &str) -> Result<(), StorageError>;
    async fn get(&self, key: &str) -> Result<bytes::Bytes, StorageError>;
    async fn delete(&self, key: &str) -> Result<(), StorageError>;
    async fn get_presigned_url(&self, key: &str, ttl: std::time::Duration) -> Result<String, StorageError>;
}
```

### 11.2 Storage Providers

* **Local Filesystem:** Default for single-node development (`data/storage/`).
* **S3 / Cloudflare R2:** Production standard configured via `S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY`, `S3_SECRET_KEY`.

---

## 12. Security, Authentication & Multi-Tenancy

### 12.1 WebAuthn / Passkeys

* Passkeys are the **exclusive** browser authentication mechanism. Passwords, magic links, and unencrypted bearer tokens in localStorage are banned.
* **Discoverable Credentials:** Sign-in requires no username input. The WebAuthn credential ID maps directly to `accounts.id`.
* **Session Storage:** Cookies are issued as `HttpOnly`, `SameSite=Strict`, `Secure`. Only the SHA-256 hash of the session token is persisted in `web_sessions`.

### 12.2 Step-Up Verification (`VerifiedUser`)

Sensitive actions (modifying passkeys, exporting financial history, generating MCP access tokens, high-value trades) require `VerifiedUser` extraction:
* Validates that the active session has performed a passkey ceremony within the preceding 5 minutes (`STEP_UP_WINDOW_MINUTES = 5`).
* If stale, the API returns `403 Forbidden` with a challenge header triggering the browser's biometric authenticator.

### 12.3 Omnichannel Chat Security Ceiling

Chat platforms carry inherent risks (stolen devices, public channels, unauthorized invites). Therefore:
* **DM Isolation:** All sensitive commands (`!expense`, `!portfolio`, `!link`, `/portfolio`) are strictly refused in public Matrix rooms or public Discord/Telegram channels. They must be executed in 1-on-1 private chats.
* **Account Linking:** Linking a Matrix, Discord, or Telegram account initiates via `/link` in private DM, generating a short-lived, single-use token that must be confirmed inside the authenticated Web UI with a passkey assertion.
* **Capability Ceiling:** High-risk actions (account deletion, passkey generation, raw exports) cannot be executed over chat; the bot replies with an authenticated web link.

---

## 13. Target Codebase Structure

The backend is refactored into a layer-first architecture. Sub-project 1 creates everything below except the entries marked *(Phase 4)* / *(Phase 5)*.

```text
backend/
├── Cargo.toml
├── .sqlx/                             # Committed offline query cache (cargo sqlx prepare)
├── migrations/                        # Forward-only PostgreSQL migrations
│   ├── 0001_baseline.sql              # Current schema, idempotent: adopts the live DB in place
│   ├── 0002_account_uuid.sql          # owner_id TEXT -> UUID + foreign keys
│   ├── 0003_jobs.sql
│   └── 0004_outbox.sql                # outbox_events + outbox_deliveries; drops webhook_outbox
├── tests/
│   └── architecture.rs                # Fails if domain/ imports sqlx/axum/matrix-sdk, or interface/ runs SQL
└── src/
    ├── main.rs                        # CLI: serve | mcp-stdio
    ├── app.rs                         # Composition root, task spawning, graceful shutdown
    ├── config.rs
    ├── error.rs                       # ApiError (HTTP), AppError (application)
    ├── domain/<feature>/              # Entities, value objects, invariants, DomainError
    ├── application/<feature>/         # Services: transaction + use case + outbox publish
    ├── interface/
    │   ├── http/                      # Axum REST, SSE, auth extractors, OpenAPI
    │   ├── chat/                      # Command parser, CommandRouter, per-feature handlers
    │   ├── channels/
    │   │   ├── adapter.rs             # ChannelAdapter trait
    │   │   ├── matrix/                # matrix-sdk adapter (E2EE, verification, !link)
    │   │   ├── discord/               # (Phase 4)
    │   │   └── telegram/              # (Phase 4)
    │   └── mcp/                       # stdio + HTTP
    ├── llm/                           # Tool registry, bounded tool loop, prompts
    │   └── agent/                     # (Phase 5) scheduled ReAct runtime
    ├── jobs/                          # queue, worker, reaper, scheduler, handlers/
    ├── events/                        # SystemEvent, outbox publish, dispatcher, SSE hub
    └── infrastructure/
        ├── postgres/                  # pool, migrate, repositories/<feature>.rs
        ├── inference/                 # LLM, STT, TTS, OCR HTTP clients
        └── storage/                   # (Phase 5) Local / S3
```

**Dependency rule:** `domain` depends on nothing but std/chrono/uuid/serde; `application` on `domain`, repositories, `events` and `jobs`; `interface` and `llm` on `application`. Enforced by `tests/architecture.rs`.

---

## 14. Phased Refactoring & Implementation Roadmap

To avoid disruptions to existing functionality, the refactoring will execute in six controlled, incremental phases, grouped into sub-projects that each get their own design and plan:

* **Sub-project 1 — PostgreSQL Foundation (phases 1–3):** [design](docs/superpowers/specs/2026-09-24-postgres-foundation-design.md). Phase 2 is realised as the per-feature move into the layers, done together with each feature's query rewrite so every file is touched once.
* **Sub-project 2 — Omnichannel (phase 4).**
* **Sub-project 3 — Scheduled agent & storage (phase 5).**
* **Sub-project 4 — Web polish (phase 6).**

```text
Phase 1: SQLite Deprecation ──► Phase 2: Domain Layer Extraction ──► Phase 3: Durable Job Queue
          │                                                                   │
          ▼                                                                   ▼
Phase 6: Multi-Interface Polish ◄── Phase 5: Scheduled Agent & Storage ◄─── Phase 4: Omnichannel Extensions
                                    (Scheduled Tasks Plugin Model)          (Discord & Telegram Adapters)
```

### Phase 1: Complete SQLite Removal & PostgreSQL Native Engine
* **Goal:** Eradicate all SQLite scaffolding and transition to pure PostgreSQL.
* **Tasks:**
  1. Delete `backend/src/db/executor.rs`, `dialect.rs`, `ddl.rs`, `dialect_probe.rs`.
  2. Refactor `DbPool` into a thin wrapper around `sqlx::PgPool`.
  3. Replace all dual-dialect macros (`db_fetch_all!`, etc.) across all existing modules with direct `sqlx::query_as!` / `sqlx::query!` calls, backed by the committed `.sqlx/` cache.
  4. Consolidate unversioned `schema*.rs` files into forward-only PostgreSQL migrations (`migrations/*.sql`), starting with an idempotent baseline the live database adopts in place, then the `owner_id` → `UUID` migration.
  5. Update CI workflows: merge the SQLite and PostgreSQL backend jobs into one job with a PostgreSQL 16 service container, plus `cargo sqlx prepare --check`.
* **Verification:** `cargo test -p matrix-bot` passes warning-free against PostgreSQL; the migrations applied to a production copy preserve every row and audit chain; no application code references SQLite.

### Phase 2: Domain Layer Extraction & Use-Case Services
* **Goal:** Separate business rules from chat handlers and Axum controllers.
* **Tasks:**
  1. Create pure domain packages under `src/domain/` for Todos, Finance, Portfolio, and Habits.
  2. Implement Application Services (e.g., `FinanceService`, `PortfolioService`) handling transactions and domain events.
  3. Refactor Matrix handlers and Axum endpoints to delegate to the new Application Services.
* **Verification:** Unit tests cover domain invariants in isolation without database or network mocks.

### Phase 3: PostgreSQL Durable Job Queue & Transactional Outbox
* **Goal:** Replace ad-hoc background polling loops (`due_reminder.rs`, briefing timers) with durable database jobs.
* **Tasks:**
  1. Implement `jobs` and `outbox_events` tables with SQL migrations.
  2. Build the `JobQueue` engine using `SELECT ... FOR UPDATE SKIP LOCKED`.
  3. Implement the `OutboxDispatcher` daemon using PostgreSQL `LISTEN` / `NOTIFY`.
  4. Migrate reminders, briefings, and TTS audio synthesis into durable jobs.
* **Verification:** Killing and restarting the bot process mid-job resumes cleanly without lost tasks or orphaned state.

### Phase 4: Omnichannel Chat Extensions [TODO Feature] & Extensible Channel Framework
* **Goal:** Introduce the extensible `ChannelAdapter` trait and `ChannelRegistry`, refactor existing Matrix support to it, and implement Discord and Telegram as planned extension plugins.
* **Tasks:**
  1. Create the `ChannelAdapter` trait, `ChannelRegistry`, and `IncomingMessage` / `OutgoingMessage` contract in `src/interface/channels/`.
  2. Refactor existing Matrix handler into `MatrixAdapter` implementing `ChannelAdapter`.
  3. **[TODO Feature]** Implement `DiscordAdapter` plugin using `serenity` with Gateway connection, Slash Commands (`/todo`, `/expense`, `/portfolio`), and Embeds.
  4. **[TODO Feature]** Implement `TelegramAdapter` plugin using `teloxide` supporting Axum Webhook, bot commands, Inline Keyboards, and photo/voice ingestion.
  5. Migrate identity resolution to `platform_identities` table, linking multiple platforms to one unified `accounts.id`.
  6. Connect Unified Tool Registry to all chat channels for direct LLM + Tool Calling on interactive user input.
* **Verification:** The same user can log an expense via Discord `/expense` or Telegram `/todo`, see it reflected in Matrix and the Web UI.

### Phase 5: Scheduled Autonomous Agent Runtime [TODO Feature] & Storage Decoupling
* **Goal:** Implement the extensible background Agent Runtime for scheduled workflows and decouple binary media storage.
* **Tasks:**
  1. Implement the extensible `ScheduledAgentTask` trait and `ScheduledTaskRegistry`.
  2. Create PostgreSQL tables: `scheduled_agent_tasks`, `agent_runs`, and `agent_steps`.
  3. Build the background agent worker loop claiming due tasks via `SELECT ... FOR UPDATE SKIP LOCKED`.
  4. Implement standard scheduled task plugins: `WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, and `PortfolioRebalanceAlertTask`.
  5. Implement the `StorageService` trait (Local filesystem and S3/R2).
  6. Refactor receipt OCR and voice notes across Matrix, Discord, and Telegram to store assets in Object Storage.
  7. Standardize HTTP contracts between `backend/` and `inference/` for STT, TTS, and OCR.
* **Verification:** Background worker automatically executes `WeeklyBookmarkDigestTask` on schedule, synthesizes audio with Kokoro TTS, and posts to linked Matrix/Discord/Telegram channels without any live user prompt.

### Phase 6: Web Console & Multi-Interface Polish
* **Goal:** Align React 19 web frontend with the omnichannel backend architecture.
* **Tasks:**
  1. Generate updated TypeScript API client and types from Axum OpenAPI spec.
  2. Implement live UI updates driven by the unified Server-Sent Events stream.
  3. Add UI settings for managing linked chat identities (Matrix, Discord, Telegram) and notification routing preferences.
* **Verification:** Actions taken in Discord, Telegram, or Matrix reflect instantly in the Web UI via SSE; Playwright E2E test suite passes.

---

## 15. Non-Functional Requirements & Operational Excellence

### 15.1 Reliability & High Availability
* **Process Model:** Single primary daemon managing Axum REST/SSE, Chat adapters (Matrix, Discord, Telegram), Job worker pool, and Outbox dispatcher.
* **Channel Fault Isolation:** A network error or reconnection backoff in Discord or Telegram never impairs the Matrix loop, Axum server, or background jobs.
* **Crash Recovery:** All pending workflows and jobs survive unexpected host restarts due to PostgreSQL transactional persistence.
* **Graceful Shutdown:** `SIGINT` / `SIGTERM` initiates a 15-second shutdown window:
  1. Chat adapters disconnect gracefully from homeservers and gateways.
  2. Axum stops accepting new connections; in-flight requests complete.
  3. Job workers finish active steps and release claimed locks.
  4. PostgreSQL pool flushes and closes connections cleanly.

### 15.2 Observability & Telemetry
* **Structured Logging:** Standardized on `tracing` with JSON output in production and colored human-readable logs in development.
* **Distributed Tracing:** Spans carry `account_id`, `job_id`, `platform`, and `channel_id` across chat interactions, application use cases, and database transactions.
* **Health Probes:**
  * `GET /health/live`: Basic liveness check returning `200 OK`.
  * `GET /health/ready`: Deep readiness probe checking PostgreSQL connectivity, chat adapter connections, and storage reachability.

### 15.3 Omnichannel Environment Configuration Reference

The platform dynamically activates chat channel adapters depending on which environment variables are supplied. If a platform's token is omitted, that specific channel adapter is disabled gracefully without impacting the others.

| Variable Name | Required | Default | Description |
|---|---|---|---|
| **Database & System** | | | |
| `DATABASE_URL` | **Yes** | - | PostgreSQL connection string (`postgres://user:pass@host:5432/dbname`) |
| `API_PORT` | No | `8080` | Port for Axum Web REST, SSE, and Webhook listener |
| `ENVIRONMENT` | No | `development` | Environment mode (`development` / `production`) |
| **Matrix Integration** | | | |
| `MATRIX_HOMESERVER` | Optional | - | Homeserver URL (e.g. `https://matrix.org`) |
| `MATRIX_USER` | Optional | - | Bot MXID (e.g. `@mybot:matrix.org`) |
| `MATRIX_PASSWORD` | Optional | - | Bot Matrix account password |
| `MATRIX_RECOVERY_KEY` | Optional | - | E2EE recovery key for automatic cross-signing verification |
| **Discord Integration** | | | |
| `DISCORD_BOT_TOKEN` | Optional | - | Discord Bot Token from Discord Developer Portal |
| `DISCORD_APPLICATION_ID` | Optional | - | Discord Application ID (for Slash Command registration) |
| `DISCORD_PUBLIC_KEY` | Optional | - | Discord Public Key (for HTTP interaction verification) |
| `DISCORD_GUILD_ID` | Optional | - | Optional Guild ID for instant development command syncing |
| **Telegram Integration** | | | |
| `TELEGRAM_BOT_TOKEN` | Optional | - | Telegram Bot API Token from @BotFather |
| `TELEGRAM_WEBHOOK_URL` | Optional | - | Public URL for Axum webhook endpoint (`https://host/api/webhooks/telegram`) |
| `TELEGRAM_SECRET_TOKEN` | Optional | - | Secret token verified against `X-Telegram-Bot-Api-Secret-Token` |
| **LLM & Inference Services** | | | |
| `AI_ENDPOINT` | No | `https://api.openai.com/v1` | OpenAI-compatible LLM endpoint base URL |
| `AI_API_KEY` | Optional | - | API key for LLM completions |
| `AI_MODEL` | No | `gpt-4o` | Default model identifier |
| `TTS_BASE_URL` | Optional | - | Endpoint for Kokoro TTS / speech synthesis |
| `STT_BASE_URL` | Optional | - | Endpoint for Sherpa STT / Whisper transcriptions |
| `OCR_BASE_URL` | Optional | - | Endpoint for OCR receipt recognition |
| `TAVILY_API_KEY` | Optional | - | Tavily Search API key for web search |
| **WebAuthn & Security** | | | |
| `WEBAUTHN_RP_ID` | **Yes** | `localhost` | WebAuthn Relying Party ID (bare hostname, e.g. `mybot.domain`) |
| `WEBAUTHN_ORIGIN` | **Yes** | `http://localhost:8080` | WebAuthn Origin (including protocol and port) |
| `REGISTRATION_OPEN` | No | `false` | Allow open web registration (`true` / `false`) |
| `REGISTRATION_SECRET` | Optional | - | Shared secret required for invite-only web signups |
| **Object Storage** | | | |
| `STORAGE_DRIVER` | No | `local` | Storage driver (`local` or `s3`) |
| `S3_ENDPOINT` | Optional | - | S3/R2 Endpoint URL |
| `S3_BUCKET` | Optional | - | S3/R2 Bucket name |
| `S3_ACCESS_KEY` | Optional | - | S3/R2 Access Key |
| `S3_SECRET_KEY` | Optional | - | S3/R2 Secret Key |

---

## 16. Architectural Acceptance Criteria

A pull request completing any milestone of this specification must satisfy:

1. **PostgreSQL Exclusivity:** The application database is PostgreSQL only: no `sqlx` `sqlite` feature, no `SqlitePool`, `Dialect` or `db_*!` macro. The only SQLite in the process is matrix-sdk's own crypto store.
2. **Query & Migration Hygiene:** `SQLX_OFFLINE=true cargo build` succeeds, `cargo sqlx prepare --check` is clean, and every data-rewriting migration has been rehearsed on a production copy.
3. **Compile-Time Hygiene:** Warning-free compilation with `cargo check` and clean formatting via `cargo fmt --all -- --check`.
4. **Omnichannel Feature Parity:** Every domain capability exposed in Matrix must be equally accessible via the Web REST API and Unified Tool Registry (MCP/Agent), and via Discord and Telegram once their respective Phase 4 TODO milestones are completed.
5. **Data Isolation:** All database tables and outbox notifications strictly isolate records by `account_id`, regardless of which chat platform originated the request.
6. **Channel Fault Tolerance:** Failure or disconnection of any single chat adapter (e.g. Discord rate limits or Matrix homeserver downtime) must not crash the daemon or affect other platforms.
7. **No Phantom State:** Every mutation must be committed to PostgreSQL within an atomic transaction alongside an Outbox event.
8. **Crash Safety:** Killing the process mid-job re-runs that job after restart; a reminder is never lost or sent twice; a webhook receiver that was down receives every event once per `X-Delivery-Id` after it recovers.
9. **Layering:** `tests/architecture.rs` passes.
