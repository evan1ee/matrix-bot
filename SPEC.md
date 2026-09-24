# Matrix Bot — Complete Architecture Design & Implementation Specification

## 1. Architecture Goal

The Matrix Bot should evolve into a **Personal AI / Life Workspace** built around:

* Rust
* Matrix
* Axum
* React 19
* PostgreSQL
* LLM
* MCP
* Optional Agent Runtime
* Persistent background jobs
* External STT / TTS / OCR services
* Object storage

The core architectural principle is:

> **Matrix is the primary interaction interface, Web is the management/visualization interface, LLM is the AI Interface Layer, Agent is the future orchestration layer, Domain Services contain the actual business logic, and PostgreSQL is the central persistent state and durable job system.**

The application should be designed so that Matrix, Web, MCP, LLM, and Agent all reuse the same application/domain capabilities rather than implementing separate business logic.

---

# 2. High-Level Architecture

```text
                                      ┌─────────────────────┐
                                      │   Claude / Cursor   │
                                      │   External LLM      │
                                      └──────────┬──────────┘
                                                 │
                                            MCP / HTTP
                                                 │
                                                 ▼
┌──────────────────┐                  ┌──────────────────────┐
│   Matrix Client  │                  │      Web Browser     │
│ Element X / etc. │                  │ React Web Console    │
└────────┬─────────┘                  └──────────┬───────────┘
         │                                       │
         │ Matrix                                │ REST / SSE
         ▼                                       ▼
┌──────────────────────────────────────────────────────────────┐
│                         MATRIX BOT                            │
│                       Rust / Axum                            │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                    Interface Layer                     │  │
│  │                                                        │  │
│  │  Matrix Handler    Web API    MCP Server    LLM API    │  │
│  └───────────┬────────────┬────────────┬─────────────────┘  │
│              │            │            │                    │
│              └────────────┴────────────┘                    │
│                           │                                  │
│                           ▼                                  │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                Application / Orchestration             │  │
│  │                                                        │  │
│  │ Command Router   AI Router   Agent Runtime   Workflows │  │
│  └──────────────────────────┬─────────────────────────────┘  │
│                             │                                │
│              ┌──────────────┴──────────────┐                 │
│              │                             │                 │
│              ▼                             ▼                 │
│  ┌─────────────────────────┐    ┌────────────────────────┐  │
│  │      Domain Layer       │    │     Job System         │  │
│  │                         │    │                        │  │
│  │ Todo                    │    │ Durable Jobs           │  │
│  │ Finance                 │    │ Retry / Schedule       │  │
│  │ Portfolio               │    │ Workers                │  │
│  │ Bills                   │    │                        │  │
│  │ Bookmarks               │    │ STT / TTS / OCR         │  │
│  │ Habits                  │    │ News / Agent Jobs       │  │
│  │ Journal                 │    │ Media Processing        │  │
│  │ Footprints              │    │                        │  │
│  └────────────┬────────────┘    └────────────┬────────────┘  │
│               │                              │               │
│               └──────────────┬───────────────┘               │
│                              ▼                               │
│                    ┌──────────────────┐                      │
│                    │ Repository / DB   │                      │
│                    │      Layer       │                      │
│                    └────────┬─────────┘                      │
└─────────────────────────────┼────────────────────────────────┘
                              │
                              ▼
                    ┌──────────────────┐
                    │   PostgreSQL     │
                    │                  │
                    │ Domain Data      │
                    │ Jobs             │
                    │ Agent State      │
                    │ Outbox           │
                    │ Audit            │
                    └──────────────────┘

         ┌────────────────────────────────────────────┐
         │             External AI Services           │
         │                                            │
         │ STT │ TTS │ OCR │ LLM │ Search │ Embedding│
         └────────────────────────────────────────────┘
```

---

# 3. Database Decision: PostgreSQL Only

The project should now standardize on **PostgreSQL as the only supported database backend**.

Do not maintain separate SQLite and PostgreSQL implementations.

### Reasons

PostgreSQL provides:

* Strong concurrency
* Multi-user support
* Reliable transactions
* Row-level locking
* `SELECT ... FOR UPDATE SKIP LOCKED`
* Durable background job queues
* Agent state persistence
* Outbox pattern support
* Future `pgvector` support
* Better support for multiple workers
* Easier future horizontal scaling

This also significantly reduces code complexity.

Avoid code such as:

```rust
if sqlite {
    ...
} else if postgres {
    ...
}
```

The application should assume PostgreSQL.

---

# 4. Six Major Layers

The architecture should be organized into six conceptual layers:

```text
1. Interface Layer
2. Application Layer
3. Agent Layer
4. Domain Layer
5. Infrastructure Layer
6. Data Layer
```

Responsibilities must remain separated.

---

# 5. Interface Layer

The Interface Layer exposes the application to external users and systems.

```text
Interface
├── Matrix
├── Web API
├── MCP
└── LLM Interface
```

## Matrix Interface

The Matrix layer handles:

* Matrix events
* Messages
* Commands
* Reactions
* Media
* Voice messages
* Locations
* Rooms
* Users

The Matrix layer must NOT directly access SQL or repositories.

Use:

```text
Matrix Event
    ↓
Command / AI Router
    ↓
Application Service
```

---

# 6. Web API

The React 19 frontend should communicate with the Rust backend through Axum REST APIs and SSE.

```text
React 19
    ↓
Axum REST API
    ↓
Application Layer
    ↓
Domain Services
    ↓
PostgreSQL
```

The Web API must reuse the same application/domain services as Matrix and MCP.

For example:

```text
Matrix:
"show my expenses"

Web:
GET /api/expenses

MCP:
finance.list_expenses()

LLM:
"what did I spend yesterday?"
```

All should eventually use:

```text
FinanceService
```

There must not be three independent implementations of Finance logic.

---

# 7. MCP Server

MCP should be treated as another Interface Layer.

Architecture:

```text
Claude Desktop
Cursor
External LLM
       │
       ▼
      MCP
       │
       ▼
┌─────────────────────┐
│   MCP Tool Adapter  │
└──────────┬──────────┘
           ▼
    Application Layer
           ▼
      Domain Services
           ▼
       PostgreSQL
```

MCP tools must NOT directly access repositories.

Use:

```text
MCP Tool
    ↓
Application Service
    ↓
Domain Service
    ↓
Repository
    ↓
PostgreSQL
```

---

# 8. LLM Interface Layer

The LLM is an **Interface Layer**, not the business logic layer.

Example:

```text
User:
"Yesterday I spent $15 on lunch. Record it."

       ↓

LLM
       ↓
Tool Call
       ↓
finance.create_expense
       ↓
FinanceService
       ↓
PostgreSQL
```

Example tool call:

```json
{
  "tool": "finance.create_expense",
  "arguments": {
    "amount": 15,
    "category": "food",
    "date": "2026-09-23"
  }
}
```

Important:

> The LLM should never own business rules.

Business rules must remain in Domain/Application services.

---

# 9. Unified Tool Registry

The existing domain tools should become a unified Tool Registry.

Example:

```text
Tool Registry
├── todo.create
├── todo.list
├── todo.complete
├── finance.create
├── finance.list
├── portfolio.get
├── reminder.create
├── bookmark.search
├── habit.create
└── ...
```

The same Tool Registry should be usable by:

```text
Matrix AI
Web AI
MCP
Agent
```

Architecture:

```text
                Tool Registry
                     ▲
       ┌─────────────┼──────────────┐
       │             │              │
     Matrix          MCP           Agent
       │             │              │
       └─────────────┴──────────────┘
```

This is a critical architectural component.

---

# 10. Agent Layer

Agent should be introduced as an orchestration layer, not as a replacement for the existing LLM/tool system.

The architecture should support:

```text
LLM + Tool Calling
       │
       ├── Simple task → Direct Tool
       │
       └── Complex task → Agent Runtime
```

Example simple task:

```text
"Add a $15 lunch expense."
```

Use:

```text
LLM → Tool → Database
```

No Agent is required.

Example complex task:

```text
"Collect today's important news,
summarize it,
generate audio,
and send it to Matrix."
```

Use:

```text
Agent
 ↓
Search
 ↓
Collect
 ↓
Summarize
 ↓
Create TTS Job
 ↓
Worker
 ↓
Matrix
```

---

# 11. Agent Runtime

The Agent Runtime should be independently designed.

```text
AgentRuntime
├── AgentContext
├── Task State
├── Tool Registry
├── Tool Executor
├── Step Limit
├── Timeout
├── Retry
├── Cancellation
└── Job Integration
```

Typical loop:

```text
Agent
 │
 ├── Think
 │
 ├── Tool Call
 │
 ├── Observe
 │
 ├── Tool Call
 │
 ├── Observe
 │
 └── Final Response
```

The runtime must enforce limits:

```text
max_steps
max_duration
max_tool_calls
max_tokens
```

Never allow an unbounded agent loop.

---

# 12. Domain Layer

The Domain Layer contains the actual business capabilities.

Recommended modules:

```text
domain/
├── todos/
├── finance/
├── portfolio/
├── bills/
├── bookmarks/
├── habits/
├── journal/
├── footprints/
├── reminders/
└── users/
```

Each domain should contain concepts such as:

```text
Entity
Value Objects
Domain Services
Repository Traits
Business Rules
```

Example:

```text
TodoService
    ↓
TodoRepository
    ↓
PostgreSQL
```

The Domain Layer should not depend on Matrix, React, MCP, or a specific LLM provider.

---

# 13. Background Job System

The application should include a **persistent background job system**.

Do NOT introduce Redis, RabbitMQ, or Kafka at this stage.

Use PostgreSQL as the durable job queue.

```text
Application
    ↓
PostgreSQL jobs table
    ↓
Worker
    ↓
External Service / Domain Operation
```

Suitable job types include:

```text
stt
tts
ocr

news_fetch
news_summarize

media_process

reminder_execute

agent_task
```

Future jobs may include:

```text
embedding
backup
export
sync
notification
```

---

# 14. Job Lifecycle

Jobs should support:

```text
pending
running
completed
failed
cancelled
```

Typical lifecycle:

```text
              ┌────────────┐
              │  pending   │
              └─────┬──────┘
                    │
                    ▼
              ┌────────────┐
              │   running  │
              └─────┬──────┘
                    │
          ┌─────────┴─────────┐
          ▼                   ▼
    ┌───────────┐       ┌──────────┐
    │ completed │       │  failed  │
    └───────────┘       └────┬─────┘
                             │
                          retry?
                             │
                             ▼
                         pending
```

Support:

* Retry
* Exponential backoff
* Timeout
* Cancellation
* Priority
* Scheduled execution
* Maximum attempts

---

# 15. PostgreSQL Job Queue

Suggested schema:

```sql
CREATE TABLE jobs (
    id              UUID PRIMARY KEY,
    user_id         UUID,
    job_type        TEXT NOT NULL,
    payload         JSONB NOT NULL,

    status          TEXT NOT NULL,

    priority        INTEGER NOT NULL DEFAULT 0,

    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3,

    available_at    TIMESTAMPTZ NOT NULL,

    locked_at       TIMESTAMPTZ,
    locked_by       TEXT,

    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,

    error           TEXT,

    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL
);
```

For multiple workers, use PostgreSQL row locking and `SKIP LOCKED` for job claiming.

Example conceptual flow:

```text
                 PostgreSQL
                     │
       ┌─────────────┼─────────────┐
       ▼             ▼             ▼
    Worker 1      Worker 2      Worker 3
       │             │             │
      TTS           OCR           STT
```

---

# 16. Durable Jobs vs Ephemeral Events

Not everything should become a database job.

### Durable jobs

Persist in PostgreSQL:

```text
TTS
STT
OCR
News fetching
News summarization
Scheduled reminders
Agent tasks
Media processing
```

### Ephemeral events

Use Tokio channels:

```text
typing indicators
temporary progress
streaming tokens
temporary UI state
```

Architecture:

```text
                    Matrix Bot
                       │
             ┌─────────┴─────────┐
             │                   │
       Durable Work        Ephemeral Work
             │                   │
             ▼                   ▼
       PostgreSQL          Tokio channel
          Jobs
```

---

# 17. Agent State

Agent execution state should be persisted in PostgreSQL.

Example:

```sql
CREATE TABLE agent_runs (
    id              UUID PRIMARY KEY,
    user_id         UUID NOT NULL,
    task            TEXT NOT NULL,
    status          TEXT NOT NULL,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    error           TEXT
);
```

And:

```sql
CREATE TABLE agent_steps (
    id              UUID PRIMARY KEY,
    agent_run_id    UUID NOT NULL,
    step            INTEGER NOT NULL,
    tool            TEXT,
    input           JSONB,
    output          JSONB,
    status          TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL
);
```

This allows an Agent to recover after a process restart.

---

# 18. External AI Services

STT, TTS, and OCR should be logically independent services.

```text
                    Matrix Bot
                        │
              ┌─────────┴─────────┐
              │                   │
          Application          Job Worker
                                  │
                   ┌──────────────┼─────────────┐
                   ▼              ▼             ▼
                  STT            TTS           OCR
```

They can initially run on the same physical server or Docker Compose stack.

They should nevertheless have clear service interfaces.

---

# 19. AI Provider Abstraction

The Rust application should not be tightly coupled to specific AI models.

Use provider abstractions such as:

```rust
trait TtsProvider
trait SttProvider
trait OcrProvider
trait LlmProvider
trait SearchProvider
```

For example:

```text
TTS
 │
 ├── Kokoro
 ├── External TTS service
 └── Future provider
```

The application should depend on the interface rather than the implementation.

---

# 20. Media Processing Pipeline

Example: receipt image → Finance record.

```text
Matrix Image
 ↓
Media Handler
 ↓
Object Storage
 ↓
OCR Job
 ↓
OCR Worker
 ↓
OCR Service
 ↓
Extract Structured Data
 ↓
FinanceService
 ↓
PostgreSQL
 ↓
Matrix Confirmation
```

Example result:

```text
Receipt detected

Amount: $23.50
Category: Food
Merchant: Walmart

Recorded successfully.
```

---

# 21. Object Storage

Large files should not be stored directly in PostgreSQL.

Use object storage for:

```text
audio
images
receipts
attachments
generated media
```

Possible providers:

```text
Cloudflare R2
S3-compatible storage
WebDAV
Local filesystem
```

PostgreSQL stores metadata:

```text
object_key
mime_type
size
hash
owner
created_at
```

---

# 22. Internal Event System

Use a lightweight internal event bus for non-durable events.

Possible implementation:

```text
Tokio broadcast / mpsc
```

Examples:

```text
TodoCreated
ExpenseCreated
ReminderTriggered
JobCompleted
AgentCompleted
```

For example:

```text
ExpenseCreated
      │
      ├── Audit
      ├── Web SSE
      └── Matrix notification
```

---

# 23. Outbox Pattern

For reliable external side effects, implement an Outbox pattern.

Example:

```text
Create Expense
     │
     ├── INSERT expense
     │
     └── INSERT outbox event
```

Both operations happen in the same PostgreSQL transaction.

Then:

```text
Outbox Worker
     ↓
Matrix notification
Web SSE
External integration
```

This prevents situations where:

```text
Database write succeeds
but notification is lost
```

---

# 24. API Structure

Recommended Axum routes:

```text
/api
├── /auth
├── /users
├── /todos
├── /expenses
├── /portfolio
├── /bills
├── /bookmarks
├── /habits
├── /journal
├── /footprints
├── /reminders
├── /ai
├── /jobs
└── /events
```

SSE:

```text
GET /api/events
```

Use SSE for:

```text
Job progress
AI responses
Notifications
Data changes
```

---

# 25. Authentication

The Web UI should use WebAuthn / Passkeys.

Recommended flow:

```text
WebAuthn / Passkey
        ↓
Session
        ↓
API
```

Provide recovery codes or another recovery mechanism so users cannot permanently lose access if they lose their passkey.

If public registration is enabled, support an installation/admin registration secret or invitation-only registration.

---

# 26. Multi-User Architecture

Even if the initial deployment has only a few users, design the database for multi-user operation.

Core records should contain:

```text
user_id
```

Examples:

```text
todos
 ├── id
 ├── user_id
 └── ...

jobs
 ├── id
 ├── user_id
 └── ...

agent_runs
 ├── id
 ├── user_id
 └── ...
```

Do not rely on a single-user assumption in the data model.

---

# 27. Matrix Room Context

Matrix requires a distinction between users and rooms.

A single user can have:

```text
DM
Family room
Work room
Private room
```

Therefore, do not treat `room_id` as a user identity.

The AI context should understand:

```text
Matrix Room
      ↓
Conversation Context
      ↓
User
      ↓
Agent / LLM
```

---

# 28. AI Context Builder

Create a dedicated context-building component:

```text
AI Context
├── User Context
├── Room Context
├── Recent Messages
├── Domain Context
├── Memories
├── Tool Definitions
└── Permissions
```

Flow:

```text
User Message
    ↓
Context Builder
    ↓
LLM
```

The LLM should not directly query arbitrary database tables to construct context.

---

# 29. Memory

Do not introduce a separate vector database initially.

Use PostgreSQL for:

```text
Conversation history
User preferences
Explicit memories
Conversation summaries
```

Later, if semantic search becomes necessary, consider PostgreSQL + `pgvector`.

This keeps the architecture simple.

---

# 30. Security Boundary

Recommended deployment boundary:

```text
                    Internet
                       │
                       ▼
                     Caddy
                       │
          ┌────────────┴────────────┐
          │                         │
     Matrix endpoint           Web/API endpoint
          │                         │
          └────────────┬────────────┘
                       ▼
                ┌─────────────┐
                │ Matrix Bot  │
                │ Rust/Axum   │
                └──────┬──────┘
                       │
              ┌────────▼────────┐
              │    Services     │
              └────────┬────────┘
                       │
                ┌──────▼──────┐
                │ PostgreSQL  │
                └─────────────┘
```

Security requirements:

* Authentication
* Authorization
* Tool permission checks
* Rate limiting
* Input validation
* Audit logging
* MCP authentication
* MCP tool allowlists
* User/room isolation
* Job ownership checks

Do not expose all domain tools to every MCP client by default.

---

# 31. Recommended Rust Project Structure

Target structure:

```text
matrix-bot/
│
├── backend/
│   ├── Cargo.toml
│   │
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs
│   │   ├── error.rs
│   │   ├── state.rs
│   │   │
│   │   ├── interface/
│   │   │   ├── matrix/
│   │   │   ├── api/
│   │   │   ├── mcp/
│   │   │   └── ai/
│   │   │
│   │   ├── application/
│   │   │   ├── commands/
│   │   │   ├── queries/
│   │   │   ├── tools/
│   │   │   └── workflows/
│   │   │
│   │   ├── agent/
│   │   │   ├── runtime.rs
│   │   │   ├── context.rs
│   │   │   ├── executor.rs
│   │   │   ├── state.rs
│   │   │   └── policies.rs
│   │   │
│   │   ├── domain/
│   │   │   ├── todos/
│   │   │   ├── finance/
│   │   │   ├── portfolio/
│   │   │   ├── bills/
│   │   │   ├── bookmarks/
│   │   │   ├── habits/
│   │   │   ├── journal/
│   │   │   ├── footprints/
│   │   │   └── reminders/
│   │   │
│   │   ├── jobs/
│   │   │   ├── queue.rs
│   │   │   ├── worker.rs
│   │   │   ├── scheduler.rs
│   │   │   └── handlers/
│   │   │
│   │   ├── infrastructure/
│   │   │   ├── postgres/
│   │   │   ├── matrix/
│   │   │   ├── llm/
│   │   │   ├── stt/
│   │   │   ├── tts/
│   │   │   ├── ocr/
│   │   │   ├── search/
│   │   │   └── storage/
│   │   │
│   │   └── events/
│   │       ├── bus.rs
│   │       ├── outbox.rs
│   │       └── events.rs
│   │
│   ├── migrations/
│   │
│   └── openapi.json
│
├── frontend/
│   └── React 19
│
├── ai-media/
│   ├── stt/
│   ├── tts/
│   └── ocr/
│
├── docs/
│   ├── architecture.md
│   ├── agent.md
│   ├── mcp.md
│   ├── jobs.md
│   └── api.md
│
└── docker/
    └── compose.yml
```

Do not perform a large restructuring blindly. Treat this as the target architecture and migrate incrementally.

---

# 32. Deployment Architecture

Recommended initial deployment:

```text
                    Internet
                       │
                       ▼
                     Caddy
                       │
          ┌────────────┴────────────┐
          │                         │
     Matrix endpoint           Web/API endpoint
          │                         │
          └────────────┬────────────┘
                       ▼
                ┌─────────────┐
                │ Matrix Bot  │
                │ Rust/Axum   │
                └──────┬──────┘
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
      PostgreSQL    AI Media     Matrix
                    Services     Homeserver
          │
          ▼
      Backup / R2
```

Initially, all services can run on the same VPS using Docker Compose.

They should still have clear logical boundaries so they can be split later.

---

# 33. Main Data Flows

## Simple Todo

```text
Matrix
 ↓
Handler
 ↓
LLM
 ↓
Tool
 ↓
TodoService
 ↓
PostgreSQL
 ↓
Response
 ↓
Matrix
```

## TTS

```text
Matrix
 ↓
LLM
 ↓
TTS Job
 ↓
PostgreSQL
 ↓
Worker
 ↓
TTS Service
 ↓
Object Storage
 ↓
Matrix
```

## OCR Finance

```text
Matrix Image
 ↓
Media Handler
 ↓
Object Storage
 ↓
OCR Job
 ↓
OCR Worker
 ↓
OCR Service
 ↓
Finance Service
 ↓
PostgreSQL
 ↓
Matrix
```

## Agent

```text
Matrix
 ↓
LLM
 ↓
Agent Runtime
 ↓
Tool
 ↓
Tool
 ↓
Job
 ↓
Worker
 ↓
Tool
 ↓
Agent
 ↓
Final Answer
 ↓
Matrix
```

## MCP

```text
Claude / Cursor
 ↓
MCP
 ↓
Tool Registry
 ↓
Application Service
 ↓
Domain
 ↓
PostgreSQL
```

---

# 34. Recommended Technology Stack

| Layer           | Technology                           |
| --------------- | ------------------------------------ |
| Language        | Rust                                 |
| HTTP            | Axum                                 |
| Matrix          | Matrix Rust SDK                      |
| Database        | PostgreSQL only                      |
| Database Access | SQLx                                 |
| Frontend        | React 19                             |
| API             | REST + SSE                           |
| Async Runtime   | Tokio                                |
| AI              | LLM provider abstraction             |
| Tool Calling    | Unified Tool Registry                |
| MCP             | MCP Server                           |
| Agent           | Lightweight custom Agent Runtime     |
| Queue           | PostgreSQL Job Queue                 |
| STT             | sherpa-onnx / Whisper-class provider |
| TTS             | Kokoro / Candle / sherpa-onnx        |
| OCR             | Provider abstraction                 |
| Search          | Search provider abstraction          |
| Storage         | S3/R2-compatible                     |
| Authentication  | WebAuthn / Passkey                   |
| Reverse Proxy   | Caddy                                |
| Deployment      | Docker Compose                       |
| Backup          | PostgreSQL + R2                      |

---

# 35. Architectural Principles

The implementation should follow these principles.

### Principle 1 — Domain is the source of truth

Business rules belong in Domain/Application services.

Not in:

* Matrix handlers
* React
* MCP
* LLM prompts
* Agent code

---

### Principle 2 — Interfaces share the same capabilities

Matrix, Web, MCP, LLM, and Agent should reuse the same services/tools.

```text
Matrix ─────┐
Web ────────┤
MCP ────────┤
LLM ────────┼──→ Application → Domain → PostgreSQL
Agent ──────┘
```

---

### Principle 3 — LLM is not the business logic

LLM decides which tool to call.

The application decides whether that operation is valid.

---

### Principle 4 — Agent is not required for every task

Use direct tool calling for simple deterministic operations.

Use Agent only when the task requires:

* Multiple steps
* Planning
* Iteration
* Multiple tools
* Long-running operations
* Dynamic decisions

---

### Principle 5 — PostgreSQL is the system state center

PostgreSQL stores:

```text
Domain data
Jobs
Agent state
Outbox
Audit data
User state
Conversation metadata
```

---

### Principle 6 — Long-running work must be asynchronous

Do not block Matrix event processing with:

* STT
* TTS
* OCR
* Large media processing
* News aggregation
* Long-running Agent tasks

Use PostgreSQL-backed Jobs.

---

### Principle 7 — External AI providers must be replaceable

Use provider interfaces.

Do not tightly couple the core application to:

```text
Kokoro
Whisper
specific LLM
specific OCR engine
specific search provider
```

---

### Principle 8 — Design for multi-user from the beginning

Even if the initial deployment has only a few users, use:

```text
user_id
room_id
permissions
ownership
```

throughout the system.

---

# 36. Target Architecture Summary

The final architecture should be conceptually:

```text
                         ┌──────────────────┐
                         │ Matrix Clients   │
                         └────────┬─────────┘
                                  │
                         ┌────────▼─────────┐
                         │   Rust Matrix    │
                         │      Bot         │
                         └────────┬─────────┘
                                  │
              ┌───────────────────┼───────────────────┐
              │                   │                   │
              ▼                   ▼                   ▼
          Matrix API           Web API              MCP
              │                   │                   │
              └───────────────────┼───────────────────┘
                                  ▼
                       ┌────────────────────┐
                       │ Interface / Router │
                       └─────────┬──────────┘
                                 │
                    ┌────────────┴────────────┐
                    │                         │
                    ▼                         ▼
             ┌─────────────┐           ┌─────────────┐
             │ LLM / Tools │           │    Agent    │
             └──────┬──────┘           └──────┬──────┘
                    │                         │
                    └────────────┬────────────┘
                                 ▼
                       ┌───────────────────┐
                       │  Application      │
                       │  Services         │
                       └─────────┬─────────┘
                                 │
                  ┌──────────────┴──────────────┐
                  │                             │
                  ▼                             ▼
          ┌────────────────┐            ┌────────────────┐
          │ Domain Layer   │            │  Job System    │
          │                │            │                │
          │ Todo           │            │ STT            │
          │ Finance        │            │ TTS            │
          │ Portfolio      │            │ OCR            │
          │ Bills          │            │ News           │
          │ Journal        │            │ Agent Tasks    │
          │ Habits         │            │ Scheduled      │
          └───────┬────────┘            └───────┬────────┘
                  │                             │
                  └──────────────┬──────────────┘
                                 ▼
                       ┌───────────────────┐
                       │   PostgreSQL      │
                       │                   │
                       │ Data              │
                       │ Jobs              │
                       │ Agent State       │
                       │ Outbox            │
                       │ Audit             │
                       └───────────────────┘
                                 │
                       ┌─────────┴─────────┐
                       ▼                   ▼
                 Object Storage       AI Media
                    R2/S3             STT/TTS/OCR
```

# 37. Final Architectural Decisions

The following decisions should be treated as the current target architecture:

1. **PostgreSQL only** — do not maintain SQLite support.
2. **PostgreSQL also acts as the durable job queue.**
3. **Do not introduce Redis/RabbitMQ/Kafka unless there is a demonstrated scaling requirement.**
4. **Matrix is the primary interaction interface.**
5. **React Web UI is another interface over the same backend.**
6. **MCP is another interface, not a separate business layer.**
7. **LLM is the AI Interface Layer.**
8. **Agent is an orchestration layer for complex tasks, not a replacement for normal tool calling.**
9. **Domain Services are the source of truth for business logic.**
10. **All interfaces should reuse the same Application/Domain/Tool capabilities.**
11. **Long-running work uses persistent PostgreSQL Jobs.**
12. **STT/TTS/OCR should use provider abstractions and can initially run as separate services in the same Docker Compose deployment.**
13. **Large media files belong in object storage, not PostgreSQL.**
14. **Agent state should be persisted in PostgreSQL.**
15. **Use an Outbox pattern for reliable external side effects.**
16. **Design for multiple users even if the initial deployment is small.**
17. **Keep Agent capabilities optional and isolated from the core domain.**
18. **Do not perform a large rewrite immediately; implement and migrate incrementally.**

The key architectural relationship is:

```text
Matrix = Interface
Web = Interface
MCP = Interface
LLM = AI Interface
Agent = Orchestrator
Application = Use Cases
Domain = Business Capability
Job System = Durable Background Execution
PostgreSQL = Persistent System State
Object Storage = Large Media
External AI Services = Replaceable Providers
```

The target product is therefore:

> **A self-hosted Personal AI / Life Workspace built around Matrix, with PostgreSQL as the persistent system core, LLM as the natural-language interface, MCP as the external AI interface, and an optional Agent Runtime for complex multi-step workflows.**
