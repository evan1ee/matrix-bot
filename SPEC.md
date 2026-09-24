# Matrix Bot — Complete Architecture Design & Implementation Specification

> **Version:** 2.0.0-draft  
> **Status:** Target Architectural Specification  
> **Target Database:** PostgreSQL 16+ Exclusively (SQLite Deprecated & Removed)  
> **Target Platforms:** Matrix (E2EE), Web (React 19 SPA), MCP (Model Context Protocol), External AI Agents  

---

## 1. Executive Summary & Architectural Vision

The `matrix-bot` platform is evolving from a single-user hybrid chat-bot into a robust, multi-tenant **Personal AI & Life Workspace**. The system unifies daily productivity, personal finance, investment tracking, knowledge management, location logging, and AI assistance into a cohesive, self-hosted platform.

### Core Architectural Principle

> **Matrix is the conversational interaction interface; Web is the visual management and analytics interface; MCP is the external AI developer interface; LLMs act as the Natural Language understanding layer; Agents orchestrate multi-step autonomous workflows; Domain Services encapsulate all business invariants; and PostgreSQL is the sole, authoritative source of truth for persistent state, durable jobs, and transactional outbox events.**

```text
┌────────────────────────────────────────────────────────────────────────┐
│                           INTERACTION SURFACES                         │
│                                                                        │
│   ┌──────────────────┐   ┌────────────────────┐   ┌─────────────────┐  │
│   │  Matrix Clients  │   │   Web Browser UI   │   │  MCP Clients    │  │
│   │  (Element X/E2EE)│   │  (React 19 / SSE)  │   │  (Cursor/Claude)│  │
│   └────────┬─────────┘   └─────────┬──────────┘   └────────┬────────┘  │
└────────────┼───────────────────────┼───────────────────────┼───────────┘
             │                       │                       │
             ▼                       ▼                       ▼
┌────────────────────────────────────────────────────────────────────────┐
│                        INTERFACE ADAPTER LAYER                         │
│                                                                        │
│     Matrix Handler         Axum REST & SSE           MCP Server        │
└────────────┬───────────────────────┬───────────────────────┬───────────┘
             │                       │                       │
             └───────────────────────┼───────────────────────┘
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                     AI & ORCHESTRATION SUBSYSTEM                       │
│                                                                        │
│   ┌────────────────────────────────────────────────────────────────┐   │
│   │                     Unified Tool Registry                      │   │
│   │    (OpenAI Function Schemas, Permission Gates, Dispatch)       │   │
│   └──────────────┬──────────────────────────────────┬──────────────┘   │
│                  │                                  │                  │
│                  ▼                                  ▼                  │
│       ┌──────────────────────┐           ┌──────────────────────┐      │
│       │   LLM Tool Gateway   │           │    Agent Runtime     │      │
│       │   (Single-turn ReAct)│           │ (Multi-step Planner) │      │
│       └──────────┬───────────┘           └──────────┬───────────┘      │
└──────────────────┼──────────────────────────────────┼──────────────────┘
                   │                                  │
                   └─────────────────┬────────────────┘
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                       APPLICATION SERVICE LAYER                        │
│                                                                        │
│      Use Case Handlers  •  Command / Query Bus  •  Tx Orchestration     │
└────────────┬───────────────────────────────────────────────┬───────────┘
             │                                               │
             ▼                                               ▼
┌─────────────────────────────┐               ┌──────────────────────────┐
│        DOMAIN LAYER         │               │     DURABLE JOB SYSTEM   │
│                             │               │                          │
│  Todos & Reminders          │               │  STT & Voice Processing  │
│  Finance & Expenses         │               │  TTS Audio Synthesis     │
│  Portfolio & Trading        │               │  OCR Receipt Extraction  │
│  Recurring Bills            │               │  Daily Briefings & News  │
│  Bookmarks & Reading List   │               │  Scheduled Reminders     │
│  Habits & Routines          │               │  Long-Running Agent Runs │
│  Journal & Footprints       │               │  Transactional Outbox    │
└────────────┬────────────────┘               └──────────────┬───────────┘
             │                                               │
             └───────────────────────┬───────────────────────┘
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                     DATA & INFRASTRUCTURE LAYER                        │
│                                                                        │
│   ┌────────────────────────────────────────────────────────────────┐   │
│   │                      PostgreSQL 16+ (Only)                     │   │
│   │   Domain Tables  •  Jobs Queue (SKIP LOCKED)  •  Outbox Events │   │
│   │   Agent State    •  Audit Logs  •  Sessions  •  Full-Text FTS  │   │
│   └────────────────────────────────┬───────────────────────────────┘   │
│                                    │                                   │
│              ┌─────────────────────┴────────────────────┐              │
│              ▼                                          ▼              │
│   ┌──────────────────────┐                   ┌──────────────────────┐  │
│   │    Object Storage    │                   │   External / Local   │  │
│   │  (S3 / R2 / Local)   │                   │   Inference Engine   │  │
│   │ Audio, Images, OCR   │                   │ (Kokoro, Sherpa STT) │  │
│   └──────────────────────┘                   └──────────────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Standardization on PostgreSQL: Complete Removal of SQLite

### 2.1 The Architectural Decision

**SQLite support is completely removed from the project.** PostgreSQL 16+ becomes the sole supported database backend across development, testing, and production.

### 2.2 Rationale and Technical Debt Elimination

Maintaining a dual-dialect database abstraction layer created massive incidental complexity and hindered system capabilities:

1. **Elimination of the Dual-Dialect Macro Layer:**
   The project previously maintained over 700 lines of complex macros in `backend/src/db/executor.rs` (`db_fetch_all!`, `db_fetch_one!`, `db_fetch_optional!`, `db_scalar_*!`, `db_execute!`, `db_fetch_all_dyn!`, `tx_*`) and the `DbPool` enum (`Sqlite(SqlitePool)` vs `Postgres(PgPool)`). Removing SQLite eliminates this layer entirely in favor of idiomatic, type-safe, direct `sqlx::PgPool` and `sqlx::query!` / `sqlx::query_as!` usage.
2. **Elimination of Runtime Query Rewriting:**
   `dialect::render_placeholders` dynamically parsed and rewritten every query from SQLite `?` syntax to PostgreSQL `$1..$n` at runtime. Eliminating this provides zero runtime translation overhead, unlocks native prepared statement caching, and allows direct SQL compilation.
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

The refactor involves removing the following files and constructs:

| Component | Target Action | Consequence |
|---|---|---|
| `backend/src/db/executor.rs` | **Delete** dual-dialect macros and `DbPool` enum | Replace with direct `sqlx::PgPool` and repository methods |
| `backend/src/db/dialect.rs` | **Delete** entirely | No runtime placeholder rewriting (`?` -> `$1..$n`) |
| `backend/src/db/dialect_probe.rs` | **Delete** entirely | No dual `FromRow` probe required |
| `backend/src/db/ddl.rs` | **Delete** entirely | Native PostgreSQL DDL replaces token substitution |
| `backend/src/db/backfill.rs` | **Deprecate / Simplify** | Remove legacy SQLite table migration and SQLite fixtures |
| `backend/src/db/pool.rs` | **Refactor** | Retain only `PgPoolOptions` configuration and health check |
| In-memory SQLite tests | **Replace** | Tests run against a transient PostgreSQL database / testcontainers |
| Unversioned `schema_*.rs` | **Transition** | Migrate to forward-only versioned migrations (`migrations/*.sql`) |

---

## 3. System Architecture & Six-Layer Model

The system is structured as a strict **Layered & Hexagonal Architecture** to enforce separation of concerns, testability, and multi-interface reuse.

```text
┌────────────────────────────────────────────────────────┐
│ 1. INTERFACE LAYER                                     │
│    Matrix Handlers  •  Axum REST & SSE  •  MCP Server  │
└───────────────────────────┬────────────────────────────┘
                            │ Calls use cases
                            ▼
┌────────────────────────────────────────────────────────┐
│ 2. APPLICATION LAYER                                   │
│    Commands  •  Queries  •  Workflows  •  Permissions  │
└──────────────┬──────────────────────────┬──────────────┘
               │                          │
               ▼                          ▼
┌──────────────────────────┐   ┌─────────────────────────┐
│ 3. AI & AGENT LAYER      │   │ 4. DOMAIN LAYER         │
│    Tool Registry         │   │    Entities & Invariants│
│    LLM Gateway           │   │    Domain Services      │
│    Agent Runtime (ReAct) │   │    Value Objects        │
│    Context Assemblers    │   │    Domain Events        │
└──────────────┬───────────┘   └──────────┬──────────────┘
               │                          │
               └─────────────┬────────────┘
                             │ Persists & Schedules
                             ▼
┌────────────────────────────────────────────────────────┐
│ 5. JOB & EVENT SUBSYSTEM                               │
│    Durable Jobs (SKIP LOCKED)  •  Transactional Outbox │
└───────────────────────────┬────────────────────────────┘
                            │ Implements via adapters
                            ▼
┌────────────────────────────────────────────────────────┐
│ 6. DATA & INFRASTRUCTURE LAYER                         │
│    PostgreSQL Repositories  •  S3 Storage  •  AI Clients│
└────────────────────────────────────────────────────────┘
```

### 3.1 Layer Responsibilities & Invariants

1. **Interface Layer (`src/interface/`):**
   * Translates incoming protocol requests (Matrix room messages, HTTP REST payloads, MCP JSON-RPC calls) into strongly typed Application Commands and Queries.
   * Formats domain responses into protocol-specific outputs (Matrix HTML with fallbacks, Axum JSON responses, SSE event payloads, MCP tool results).
   * **Rule:** Zero business logic; zero direct SQL execution.
2. **Application Layer (`src/application/`):**
   * Implements specific use cases (e.g., `CreateTodoCommand`, `RecordExpenseCommand`, `ExecuteTradeCommand`).
   * Manages transaction boundaries (`sqlx::Transaction<'_, Postgres>`).
   * Dispatches domain events to the Transactional Outbox.
   * Enforces cross-domain orchestration (e.g., receipt image uploaded -> enqueue OCR job -> parse expense -> notify user).
3. **AI & Agent Layer (`src/agent/` and `src/ai/`):**
   * Maintains the **Unified Tool Registry**, exposing domain capabilities to LLMs and Agents as standard schemas.
   * Manages the **Agent Runtime**, providing a deterministic, step-bounded ReAct execution loop with persistent state tracking in PostgreSQL.
   * Assembles dynamic system prompts combining user preferences, room context, and permission filters.
4. **Domain Layer (`src/domain/`):**
   * Encapsulates enterprise business rules, entities, and value objects (e.g., `Money`, `Holding`, `RecurrenceSchedule`).
   * Expresses operations as pure domain methods enforcing business invariants (e.g., "cannot buy securities without sufficient cash balance").
   * **Rule:** Completely decoupled from web frameworks, Matrix SDK, and database libraries.
5. **Job & Event Subsystem (`src/jobs/` and `src/events/`):**
   * Houses the durable PostgreSQL job queue, worker pool orchestration, retry policies, and dead-letter handling.
   * Implements the Transactional Outbox pattern with `LISTEN` / `NOTIFY` and poll fallback for guaranteed event delivery.
6. **Data & Infrastructure Layer (`src/infrastructure/`):**
   * Implements repository traits using native PostgreSQL SQLx queries.
   * Manages external clients: S3/R2 storage, Matrix homeserver HTTP client, local/remote STT/TTS/OCR engines.

---

## 4. PostgreSQL Database Architecture & Schema Specification

All relational, unstructured (JSONB), and queuing state resides in PostgreSQL 16+.

### 4.1 Primary Database Standards

* **Identifiers:** Standardized on `UUIDv7` or `UUIDv4` for global unique identifiers across all primary keys.
* **Timestamps:** Standardized on `TIMESTAMPTZ` (UTC). Application code uses `chrono::DateTime<Utc>`.
* **Money & Finance:** All currency amounts are represented as `BIGINT` representing minor units (e.g., cents, satoshis) alongside a 3-character ISO 4217 currency code (`VARCHAR(3)`), or high-precision `NUMERIC(18, 4)` where fractional units are required.
* **Quantities:** Investment quantities use `BIGINT` with an 8-decimal fixed exponent or `NUMERIC(24, 8)`.
* **Booleans:** Native PostgreSQL `BOOLEAN` (`TRUE` / `FALSE`).
* **Unstructured Payloads:** Native PostgreSQL `JSONB` with `GIN` indexing where queried.

### 4.2 Core Schemas

#### 4.2.1 Multi-Tenant Accounts & Authentication

```sql
-- Core User Account (Owner of all data)
CREATE TABLE accounts (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name        TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'unclaimed', 'suspended')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Matrix Identities linked to an Account (1-to-Many)
CREATE TABLE matrix_links (
    mxid                TEXT PRIMARY KEY,
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    linked_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_matrix_links_account ON matrix_links(account_id);

-- WebAuthn / Passkey Credentials
CREATE TABLE webauthn_credentials (
    id                  BYTEA PRIMARY KEY,
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    public_key          BYTEA NOT NULL,
    counter             BIGINT NOT NULL DEFAULT 0,
    aaguid              UUID NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at        TIMESTAMPTZ
);

CREATE INDEX idx_webauthn_account ON webauthn_credentials(account_id);

-- Web Sessions (Indexed by SHA-256 hash of the session token)
CREATE TABLE web_sessions (
    token_hash          BYTEA PRIMARY KEY,
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    last_asserted_at    TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sessions_account ON web_sessions(account_id);
CREATE INDEX idx_sessions_expires ON web_sessions(expires_at);

-- User Settings
CREATE TABLE user_settings (
    account_id          UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    timezone            TEXT NOT NULL DEFAULT 'UTC',
    default_currency    VARCHAR(3) NOT NULL DEFAULT 'USD',
    tts_voice           TEXT,
    ai_model            TEXT,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### 4.2.2 Productivity: Todos & Reminders

```sql
CREATE TABLE todos (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    title               TEXT NOT NULL,
    description         TEXT,
    completed           BOOLEAN NOT NULL DEFAULT FALSE,
    completed_at        TIMESTAMPTZ,
    due_at              TIMESTAMPTZ,
    priority            SMALLINT NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 3),
    tags                TEXT[] NOT NULL DEFAULT '{}',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_todos_account_pending ON todos(account_id) WHERE completed = FALSE;
CREATE INDEX idx_todos_due_at ON todos(due_at) WHERE completed = FALSE;

CREATE TABLE reminders (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    room_id             TEXT,
    message             TEXT NOT NULL,
    remind_at           TIMESTAMPTZ NOT NULL,
    recurrence_rule     TEXT, -- Optional iCalendar RRULE
    dispatched          BOOLEAN NOT NULL DEFAULT FALSE,
    dispatched_at       TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_reminders_pending ON reminders(remind_at) WHERE dispatched = FALSE;
```

#### 4.2.3 Personal Finance & Investment Portfolio

```sql
-- Finance Transactions
CREATE TABLE transactions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    amount_minor        BIGINT NOT NULL,
    currency            VARCHAR(3) NOT NULL,
    category            TEXT NOT NULL,
    description         TEXT NOT NULL,
    occurred_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    receipt_storage_key TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_transactions_account_date ON transactions(account_id, occurred_at DESC);
CREATE INDEX idx_transactions_category ON transactions(account_id, category);

-- Portfolio Cash Balances (per currency)
CREATE TABLE portfolio_cash_balances (
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    currency            VARCHAR(3) NOT NULL,
    balance_minor       BIGINT NOT NULL DEFAULT 0,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_id, currency)
);

-- Portfolio Holdings (Investment Assets)
CREATE TABLE portfolio_holdings (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    symbol              TEXT NOT NULL,
    currency            VARCHAR(3) NOT NULL,
    quantity_scaled     BIGINT NOT NULL, -- Scaled by 10^8 for fractional shares/crypto
    avg_cost_minor      BIGINT NOT NULL, -- Weighted average cost per unit
    current_price_minor BIGINT,          -- Last known price per unit
    last_priced_at      TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_account_symbol UNIQUE (account_id, symbol)
);

-- Trade Audit Log
CREATE TABLE portfolio_trades (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    symbol              TEXT NOT NULL,
    side                TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    quantity_scaled     BIGINT NOT NULL,
    price_minor         BIGINT NOT NULL,
    currency            VARCHAR(3) NOT NULL,
    gross_proceeds_minor BIGINT NOT NULL,
    realized_pl_minor   BIGINT,
    executed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_trades_account ON portfolio_trades(account_id, executed_at DESC);
```

#### 4.2.4 Knowledge & Life Workspace

```sql
-- Bookmarks & Reading List
CREATE TABLE bookmarks (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    url                 TEXT NOT NULL,
    title               TEXT NOT NULL,
    summary             TEXT,
    content_text        TEXT,
    tags                TEXT[] NOT NULL DEFAULT '{}',
    is_archived         BOOLEAN NOT NULL DEFAULT FALSE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_bookmarks_account ON bookmarks(account_id);
CREATE INDEX idx_bookmarks_fts ON bookmarks USING GIN(to_tsvector('english', title || ' ' || COALESCE(summary, '') || ' ' || COALESCE(content_text, '')));

-- Habits Tracker
CREATE TABLE habits (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    frequency_days      SMALLINT NOT NULL DEFAULT 1,
    target_count        INTEGER NOT NULL DEFAULT 1,
    archived            BOOLEAN NOT NULL DEFAULT FALSE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE habit_logs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    habit_id            UUID NOT NULL REFERENCES habits(id) ON DELETE CASCADE,
    logged_date         DATE NOT NULL,
    count               INTEGER NOT NULL DEFAULT 1,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_habit_date UNIQUE (habit_id, logged_date)
);

-- Journal & Notes
CREATE TABLE journal_entries (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    content             TEXT NOT NULL,
    mood                SMALLINT CHECK (mood BETWEEN 1 AND 5),
    tags                TEXT[] NOT NULL DEFAULT '{}',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_journal_account_date ON journal_entries(account_id, created_at DESC);
CREATE INDEX idx_journal_fts ON journal_entries USING GIN(to_tsvector('english', content));

-- Geolocation Footprints
CREATE TABLE footprints (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    latitude            DOUBLE PRECISION NOT NULL,
    longitude           DOUBLE PRECISION NOT NULL,
    accuracy_meters     DOUBLE PRECISION,
    location_name       TEXT,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_footprints_account_time ON footprints(account_id, recorded_at DESC);
```

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
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for instant worker dequeue without scanning running or completed jobs
CREATE INDEX idx_jobs_dequeue ON jobs (priority DESC, available_at ASC)
WHERE status = 'pending';

-- Index for monitoring, stuck worker detection, and cleanup
CREATE INDEX idx_jobs_running ON jobs (locked_at)
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
WHERE jobs.id = next_job.id
RETURNING jobs.*;
```

### 5.3 Error Handling, Backoff, and Dead-Letter Protocol

When a job execution fails:
* If `attempts < max_attempts`:
  * Calculate exponential backoff: `available_at = NOW() + (backoff_seconds * 2^(attempts - 1))`.
  * Set `status = 'pending'`, `locked_at = NULL`, `locked_by = NULL`, `error = $error`.
* If `attempts >= max_attempts`:
  * Transition to dead-letter state: `status = 'failed'`, `completed_at = NOW()`, `error = $error`.
  * Emit an alert event via the Transactional Outbox.

### 5.4 Stuck Worker Reaper

A background task runs periodically to reclaim jobs where `status = 'running'` and `locked_at < NOW() - INTERVAL '15 minutes'`:
* Reset such jobs to `status = 'pending'`, increment `attempts`, or mark as `failed` if timeout exceeded.

---

## 6. Transactional Outbox Pattern & Notification Subsystem

To guarantee that state changes reliably produce Matrix messages, Web SSE events, and audit logs without distributed transaction inconsistency, the system implements the **Transactional Outbox Pattern**.

### 6.1 Outbox Schema

```sql
CREATE TABLE outbox_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    event_type      TEXT NOT NULL,
    payload         JSONB NOT NULL,
    idempotency_key TEXT UNIQUE,
    published       BOOLEAN NOT NULL DEFAULT FALSE,
    published_at    TIMESTAMPTZ,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_outbox_pending ON outbox_events(created_at ASC)
WHERE published = FALSE;
```

### 6.2 Outbox Processing Lifecycle

```text
┌────────────────────────────────────────────────────────┐
│               Application Service Mutation             │
│                                                        │
│  BEGIN TRANSACTION;                                    │
│    1. INSERT INTO transactions (...) VALUES (...);     │
│    2. UPDATE portfolio_cash_balances SET ...;         │
│    3. INSERT INTO outbox_events (                      │
│         account_id, event_type, payload                │
│       ) VALUES (...);                                  │
│    4. PERFORM pg_notify('outbox_channel', id::text);   │
│  COMMIT;                                               │
└───────────────────────────┬────────────────────────────┘
                            │ (Zero chance of missing event)
                            ▼
┌────────────────────────────────────────────────────────┐
│                 Outbox Dispatcher Daemon               │
│                                                        │
│  1. Woken by LISTEN outbox_channel (or 1s timer)       │
│  2. SELECT ... FROM outbox_events                      │
│     WHERE published = FALSE                            │
│     FOR UPDATE SKIP LOCKED LIMIT 50;                   │
│  3. Fan-out concurrently:                              │
│     ├── Broadcast to Axum Web SSE channel              │
│     ├── Route to Matrix DM / Room Client               │
│     └── Trigger Webhook integrations                   │
│  4. UPDATE outbox_events                               │
│     SET published = TRUE, published_at = NOW()         │
│     WHERE id IN (...);                                 │
└────────────────────────────────────────────────────────┘
```

---

## 7. Unified Tool Registry & AI Architecture

The system decouples tool execution from specific LLM providers and user interfaces. A single **Unified Tool Registry** serves Matrix conversational AI, the Web AI console, MCP servers, and autonomous Agent workflows.

### 7.1 Architectural Position of LLMs and Agents

* **LLM is an Interface:** The model acts as a natural language parser, intent classifier, and parameter extractor. It does **not** make authoritative business decisions.
* **Agent is an Orchestrator:** The Agent executes multi-step plans requiring iterative feedback. Single-turn commands bypass the Agent and execute tools directly.

```text
User Input: "Add 15 dollars for lunch"
  └─► LLM identifies tool: finance.create_expense(amount=15, category="food")
        └─► Direct invocation: FinanceService::create_expense(...)
              └─► DB write + Outbox event

User Input: "Read my unread bookmarks, summarize them, and send me an audio brief"
  └─► LLM identifies complex workflow
        └─► Dispatches to Agent Runtime
              ├─► Step 1: bookmarks.list_unread()
              ├─► Step 2: llm.summarize_batch()
              ├─► Step 3: jobs.enqueue_tts()
              └─► Step 4: matrix.send_voice_message()
```

### 7.2 Tool Definition Specification

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
    pub allowed_interfaces: Vec<InterfaceKind>, // Matrix, Web, MCP, Agent
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

### 7.3 Standardized Tool Modules

| Namespace | Tool Name | Description | Sensitivity |
|---|---|---|---|
| `todo` | `todo.create` | Create a new task with due date and priority | Low |
| `todo` | `todo.list` | List tasks with filters (pending, completed, tags) | Low |
| `todo` | `todo.complete` | Mark a task as completed | Low |
| `finance` | `finance.record_expense` | Log an expense with amount, currency, and category | Low |
| `finance` | `finance.get_summary` | Aggregate spending by category for a time period | Low |
| `portfolio` | `portfolio.get_holdings` | Retrieve current asset positions and values | Low |
| `portfolio` | `portfolio.execute_trade` | Buy or sell securities/crypto | **High (Requires Confirmation)** |
| `reminders` | `reminders.schedule` | Create a time-based or recurring reminder | Low |
| `bookmarks` | `bookmarks.save` | Save and summarize a URL | Low |
| `bookmarks` | `bookmarks.search` | Search saved bookmarks with semantic or full-text | Low |
| `habits` | `habits.log` | Check in a habit completion | Low |
| `journal` | `journal.write` | Record a journal entry or thought | Low |
| `media` | `media.synthesize_speech` | Convert text into speech audio | Low |
| `search` | `search.web` | Live web search via Tavily / DuckDuckGo | Low |

---

## 8. Agent Runtime Specification

The Agent Runtime provides an isolated, stateful, ReAct (Reasoning + Acting) execution environment for complex workflows.

### 8.1 Agent State Persistence Schema

```sql
CREATE TABLE agent_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    initiator       TEXT NOT NULL CHECK (initiator IN ('matrix', 'web', 'mcp', 'cron')),
    goal            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'planning'
                    CHECK (status IN ('planning', 'executing', 'awaiting_approval', 'completed', 'failed', 'cancelled')),
    max_steps       INTEGER NOT NULL DEFAULT 10,
    current_step    INTEGER NOT NULL DEFAULT 0,
    total_tokens    INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_runs_account ON agent_runs(account_id, started_at DESC);

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

### 8.2 Execution Loop & Safety Boundaries

```text
┌────────────────────────────────────────────────────────┐
│                   Agent Execution Loop                 │
│                                                        │
│  1. Check Boundaries:                                  │
│     ├── step_count <= max_steps (default 10)           │
│     ├── elapsed_time <= max_timeout (default 180s)     │
│     └── total_tokens <= token_budget (default 32k)     │
│  2. Assemble Context:                                  │
│     ├── Goal + System Persona                          │
│     ├── Past Agent Steps (Thoughts, Tools, Results)    │
│     └── Allowed Tool Definitions                       │
│  3. Model Inference (LLM Generates Thought + Tool)     │
│  4. Evaluate Tool Call:                                │
│     ├── If requires confirmation:                      │
│     │     Set status = 'awaiting_approval';            │
│     │     Send approval prompt to Matrix / Web;        │
│     │     Halt loop until user responds;               │
│     └── Else:                                          │
│           Execute tool via ToolHandler;                │
│  5. Persist Step into agent_steps (PostgreSQL);        │
│  6. Check Goal Completion:                             │
│     ├── If Final Answer: Set status = 'completed';     │
│     └── Else: Increment step, loop back to 1.          │
└────────────────────────────────────────────────────────┘
```

---

## 9. Interface Layer Specifications

### 9.1 Matrix Bot Interface

* **Library:** `matrix-sdk` with full End-to-End Encryption (E2EE) support via PostgreSQL/state store.
* **Responsibilities:**
  * Process timeline room events, direct messages (DMs), reactions, and encrypted media attachments.
  * Command Router: Parses prefix commands (`!todo`, `!expense`, `!portfolio`, `!remind`, `!say`, `!link`, `!recover`).
  * Conversational AI Router: Dispatches freeform messages to the AI Context Builder when the bot is mentioned or addressed in a DM.
  * Reply Formatting: Renders structured HTML with `<mx-reply>` support and clean plain-text fallbacks via `html_escape()` and `strip_html()`.
  * Audio Notes: Dispatches `!say` voice synthesis results as native Matrix voice messages (`audio/ogg; codecs=opus`) with waveform metadata.
* **Security Guard:** Never execute privileged administrative actions (account export, passkey enrollment, unlinking) directly over Matrix. Matrix commands provide web links for sensitive step-up verification.

### 9.2 Web REST & SSE API (Axum)

* **Endpoints:**
  * `/api/auth/*`: WebAuthn registration, passkey authentication, session status (`/api/auth/me`), step-up assertions.
  * `/api/todos/*`: Full CRUD for tasks, completion toggles, tag filters.
  * `/api/finance/*`: Expense logging, categories, aggregation summaries, receipt upload.
  * `/api/portfolio/*`: Holdings view, trade execution, cash balance adjustments.
  * `/api/bookmarks/*`: Bookmark ingestion, tagging, full-text search.
  * `/api/habits/*`: Habit definitions, streak tracking, daily check-ins.
  * `/api/journal/*`: Daily notes, mood ratings, search.
  * `/api/footprints/*`: Geolocation ingestion, location history feeds.
  * `/api/ai/*`: Direct conversational AI chat with SSE streaming tokens.
  * `/api/jobs/*`: Background job inspection, queue metrics.
  * `/api/events`: Server-Sent Events (SSE) stream for real-time live UI updates, filtered strictly by authenticated `account_id`.
* **Standardized Error Envelope:**
  ```json
  {
    "error": "Human readable error description",
    "code": 400,
    "details": null
  }
  ```

### 9.3 MCP Server (Model Context Protocol)

* **Transports:** Standard I/O (`stdio`) for local editor integrations (Cursor, Claude Desktop) and HTTP/SSE for remote agents.
* **Authentication:** Authenticated via Personal Access Tokens (`mb_mcp_` prefix) tied to an `account_id`.
* **Access Control:** Exposes domain capabilities from the Unified Tool Registry. Destructive or sensitive financial operations require explicit permission allowlisting in the user's settings.

---

## 10. External AI Services & Media Processing Pipeline

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

### 10.2 Media Processing Example: Automated Receipt Ingestion

```text
User uploads receipt image to Matrix DM
  │
  ▼
1. Matrix Handler receives m.image event
  │
  ▼
2. Binary stream uploaded to Object Storage (S3 / R2 / Local)
   Storage Key: accounts/{id}/receipts/{uuid}.jpg
  │
  ▼
3. Enqueue Durable Job in PostgreSQL:
   job_type: "media.process_receipt"
   payload: { "storage_key": "...", "account_id": "..." }
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
   ReceiptProcessed { transaction_id, amount, merchant }
  │
  ▼
6. Outbox Dispatcher sends Matrix reply & Web SSE notification:
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

### 12.3 Matrix Capability Ceiling

Matrix chat carries inherent risks (stolen phone, unlocked Element desktop, unauthorized room invites). Therefore:
* Privileged administrative actions cannot be completed over Matrix.
* `!link` begins in Matrix but requires passkey confirmation in the web browser.
* `!recover` initiates a mandatory 24-hour cooling-off period during which all active linked sessions are alerted and can cancel the request.
* `!passkey add` generates a single-use token valid only for enrolling an account's very first passkey.

---

## 13. Target Codebase Structure

The backend will be refactored into a clear layered architecture:

```text
backend/
├── Cargo.toml
├── migrations/                        # Versioned PostgreSQL migrations (*.sql)
│   ├── 0001_initial_schema.sql
│   ├── 0002_jobs_and_outbox.sql
│   └── 0003_agent_state.sql
│
└── src/
    ├── main.rs                        # Process entrypoint & graceful shutdown
    ├── config.rs                      # Typed environment configuration
    ├── error.rs                       # Standardized Error types & ApiError
    │
    ├── interface/                     # 1. Interface Adapters
    │   ├── mod.rs
    │   ├── matrix/                    # Matrix SDK event handlers & formatting
    │   │   ├── handler.rs
    │   │   ├── router.rs
    │   │   ├── quote.rs
    │   │   └── format.rs
    │   ├── api/                       # Axum REST & SSE HTTP routes
    │   │   ├── routes.rs
    │   │   ├── auth/
    │   │   ├── todos.rs
    │   │   ├── finance.rs
    │   │   ├── portfolio.rs
    │   │   └── sse.rs
    │   └── mcp/                       # Model Context Protocol adapter
    │       ├── stdio.rs
    │       └── protocol.rs
    │
    ├── application/                   # 2. Application Layer (Use Cases)
    │   ├── mod.rs
    │   ├── todos/
    │   ├── finance/
    │   ├── portfolio/
    │   └── common/
    │
    ├── agent/                         # 3. AI & Agent Subsystem
    │   ├── mod.rs
    │   ├── runtime.rs                 # ReAct execution loop
    │   ├── context.rs                 # Context & prompt assembler
    │   ├── state.rs                   # Agent persistence
    │   └── tools/                     # Unified Tool Registry
    │       ├── registry.rs
    │       ├── definitions.rs
    │       └── handlers/
    │
    ├── domain/                        # 4. Pure Domain Layer
    │   ├── mod.rs
    │   ├── todos/
    │   ├── finance/                   # Money, Categories, Invariants
    │   ├── portfolio/                 # Holdings, Weighted Cost, Trades
    │   └── habits/
    │
    ├── jobs/                          # 5. Durable Background Job Queue
    │   ├── mod.rs
    │   ├── queue.rs                   # SKIP LOCKED enqueue/dequeue
    │   ├── worker.rs                  # Worker pool loop
    │   ├── reaper.rs                  # Stuck job recovery
    │   └── handlers/                  # TTS, STT, OCR, Briefing handlers
    │
    ├── events/                        # 6. Transactional Outbox & Events
    │   ├── mod.rs
    │   ├── outbox.rs                  # Outbox poller & LISTEN/NOTIFY
    │   ├── bus.rs                     # Process-wide broadcast channel
    │   └── definitions.rs             # SystemEvent variants
    │
    └── infrastructure/                # 7. Infrastructure Adapters
        ├── mod.rs
        ├── postgres/                  # PgPool setup & repositories
        │   ├── pool.rs
        │   └── repositories/
        ├── storage/                   # S3 / Local storage adapter
        ├── inference/                 # STT, TTS, OCR HTTP clients
        └── matrix/                    # Homeserver crypto & sync client
```

---

## 14. Phased Refactoring & Implementation Roadmap

To avoid disruptions to existing functionality, the refactoring will execute in six controlled, incremental phases.

```text
Phase 1: SQLite Deprecation ──► Phase 2: Domain Layer Extraction ──► Phase 3: Durable Job Queue
          │                                                                   │
          ▼                                                                   ▼
Phase 6: Multi-Interface Polish ◄── Phase 5: Storage & Media ◄─── Phase 4: Unified Tools & Agent
```

### Phase 1: Complete SQLite Removal & PostgreSQL Native Engine
* **Goal:** Eradicate all SQLite scaffolding and transition to pure PostgreSQL.
* **Tasks:**
  1. Delete `backend/src/db/executor.rs`, `dialect.rs`, `ddl.rs`, `dialect_probe.rs`.
  2. Refactor `DbPool` into a thin wrapper around `sqlx::PgPool`.
  3. Replace all dual-dialect macros (`db_fetch_all!`, etc.) across all existing modules with direct `sqlx::query_as!` / `sqlx::query!` calls.
  4. Consolidate unversioned `schema_*.rs` files into clean, forward-only PostgreSQL migrations (`migrations/*.sql`).
  5. Update CI workflows: eliminate SQLite matrix jobs; configure PostgreSQL 16 service containers for all test runs.
* **Verification:** `cargo test -p matrix-bot` passes warning-free against PostgreSQL with zero references to `SqlitePool` or `sqlite3`.

### Phase 2: Domain Layer Extraction & Use-Case Services
* **Goal:** Separate business rules from Matrix handlers and Axum controllers.
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

### Phase 4: Unified Tool Registry & Agent Runtime
* **Goal:** Unify tool definitions across Matrix, Web, and MCP, and introduce the Agent execution engine.
* **Tasks:**
  1. Build `ToolRegistry` exposing all domain application use cases as OpenAI-compatible schemas.
  2. Implement the stateful `AgentRuntime` with PostgreSQL tables `agent_runs` and `agent_steps`.
  3. Expose the Tool Registry over the MCP protocol (`stdio` and HTTP/SSE).
  4. Connect Matrix conversational AI router to the Unified Tool Registry.
* **Verification:** Claude Desktop / Cursor can invoke domain tools over MCP; Matrix can orchestrate multi-step tasks.

### Phase 5: Storage Abstraction & External AI Decoupling
* **Goal:** Decouple binary media and AI inference services from the core bot.
* **Tasks:**
  1. Implement the `StorageService` trait (Local filesystem and S3/R2).
  2. Refactor receipt OCR and voice notes to store assets in Object Storage.
  3. Standardize HTTP contracts between `backend/` and `inference/` for STT, TTS, and OCR.
* **Verification:** Image receipts uploaded over Matrix are saved to S3/local storage, parsed via OCR jobs, and correctly recorded in Finance.

### Phase 6: Web Console & Multi-Interface Polish
* **Goal:** Align React 19 web frontend with the unified backend architecture.
* **Tasks:**
  1. Generate updated TypeScript API client and types from Axum OpenAPI spec.
  2. Implement live UI updates driven by the unified Server-Sent Events stream.
  3. Implement UI consoles for Job Queue monitoring and Agent Run inspection.
* **Verification:** Actions taken in Matrix reflect instantly in the Web UI via SSE; Playwright E2E test suite passes.

---

## 15. Non-Functional Requirements & Operational Excellence

### 15.1 Reliability & High Availability
* **Process Model:** Single primary daemon managing Axum REST/SSE, Matrix sync loop, Job worker pool, and Outbox dispatcher.
* **Crash Recovery:** All pending workflows and jobs survive unexpected host restarts due to PostgreSQL transactional persistence.
* **Graceful Shutdown:** `SIGINT` / `SIGTERM` initiates a 15-second shutdown window:
  1. Axum stops accepting new connections; in-flight requests complete.
  2. Matrix sync loop stops polling.
  3. Job workers finish active steps and release claimed locks.
  4. PostgreSQL pool flushes and closes connections cleanly.

### 15.2 Observability & Telemetry
* **Structured Logging:** Standardized on `tracing` with JSON output in production and colored human-readable logs in development.
* **Distributed Tracing:** Spans carry `account_id`, `job_id`, `trace_id`, and `room_id` across Matrix events, application use cases, and database transactions.
* **Health Probes:**
  * `GET /health/live`: Basic liveness check returning `200 OK`.
  * `GET /health/ready`: Deep readiness probe checking PostgreSQL connectivity, Matrix sync status, and storage reachability.

---

## 16. Architectural Acceptance Criteria

A pull request completing any milestone of this specification must satisfy:

1. **PostgreSQL Exclusivity:** Zero lines of code referencing SQLite, `rusqlite`, or `sqlx::SqlitePool`.
2. **Compile-Time Hygiene:** Warning-free compilation with `cargo check` and clean formatting via `cargo fmt --all -- --check`.
3. **Dual Interface Parity:** Every domain capability exposed in Matrix must be equally accessible via Web REST API and the Unified Tool Registry (MCP/Agent).
4. **Data Isolation:** All database tables and outbox notifications strictly isolate records by `account_id`.
5. **No Long-Running Blocks:** Any operation taking more than 500ms (STT, TTS, OCR, LLM scraping) must run as an asynchronous PostgreSQL Job.
6. **No Phantom State:** Every mutation must be committed to PostgreSQL within an atomic transaction alongside an Outbox event.
