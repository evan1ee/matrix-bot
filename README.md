# Matrix Bot (Rust) 🦀

A production-ready, highly modular, multi-feature Matrix bot & REST API engine written in Rust using the official [`matrix-sdk`](https://github.org/matrix-org/matrix-rust-sdk), [`axum`](https://github.org/tokio-rs/axum), and [`sqlx`](https://github.org/launchbadge/sqlx).

---

## 🌟 Core Features

- 🤖 **Core Module**: Greetings (`!hello`, `!hi`), latency ping (`!ping`), and dynamic module help (`!help`).
- 📋 **To-Do Management Module**:
  - Task creation with inline priority tags (`!todo add [high] Buy groceries` or `-p high`).
  - Set task due dates (`!todo due <id> <time>`).
  - Task editing (`!todo edit <id> <new text>`) and status toggle (`!todo done <id>`).
  - User-specific (`!todo list`) and room-wide (`!todo room`) views with 25-item pagination.
  - Clearing completed tasks per user or room (`!todo clear [room]`).
- ⏰ **Persistent Reminder Module**:
  - One-time timers (`!remind 10s check oven`, `!remind 2h meeting`).
  - Recurring scheduled reminders (`!remind daily 8h standup`, `!remind weekly 1d report`).
  - Persistent background worker scheduler — reminders survive bot restarts!
  - Delivery retry logic (up to 5 retries on transient network errors) and downtime catch-up protection.
  - List (`!remind list`) and cancel (`!remind cancel <id>`) pending timers.
- ⚡ **Embedded Axum REST API Backend**:
  - Full CRUD REST API for To-Dos and Reminders, enabling Web App integration.
  - **Native Matrix Authentication**: Authenticate via Matrix Homeserver password (`POST /api/v1/auth/login` with `@user:domain`) or pass Matrix Access Tokens (`syt_...`) directly in Bearer headers. Automatic Matrix Well-Known server discovery included!
  - **Local JWT Authentication** (`/api/v1/auth/register`, `/api/v1/auth/login`) with `bcrypt` password hashing and optional `API_REGISTRATION_SECRET` gating.
  - **Server-Sent Events (SSE)** real-time stream (`/api/v1/events`), scoped to the authenticated user — a client receives only events for rows it owns. Messages are unnamed, carrying `{"type": "TodoCreated", "payload": {...}}` in the body, so browsers read them from `EventSource.onmessage` and discriminate on `type`.
  - **CORS** middleware & static Single-Page Application web server built-in.
  - Standardized JSON error response wrapper (`ApiError` with `{ "error": "...", "code": 400 }`).
- 💾 **SQLx Persistence Layer**:
  - SQLite default with automatic WAL mode (`journal_mode=WAL`), busy timeout, and configurable connection pool limit (`DATABASE_MAX_CONNECTIONS`).
  - Runtime storage cleanly isolated inside the `data/` directory.
  - Abstract repository interface ready for seamless PostgreSQL migration.
- 🧱 **Extensible `BotModule` Architecture**:
  - Trait-based plugin system where new features live in isolated submodules under `src/modules/`.
  - Strict quality guarantee: **Zero source files exceed 300 lines of code** and 100% test pass rate with 0 compilation warnings.

---

## 📋 Command Quick Reference

### Core Commands
| Command | Usage | Description |
|---|---|---|
| `!hello` / `!hi` | `!hello [name]` or `!hi` | Get a friendly greeting |
| `!ping` | `!ping` | Check bot status & latency |
| `!help` | `!help` | Display dynamic help for all registered modules |

### To-Do Commands
| Command | Usage | Description |
|---|---|---|
| `!todo add` | `!todo add [[high\|low]] <task>` | Add task with optional priority (`[high]`, `[med]`, `[low]`) |
| `!todo list` | `!todo [list\|my] [page]` | View your personal tasks (25 per page) |
| `!todo room` | `!todo [room\|all] [page]` | View all tasks in the current room |
| `!todo done` | `!todo done <id>` | Mark task `#id` as completed |
| `!todo priority` | `!todo priority <id> <low\|medium\|high>` | Update task priority |
| `!todo due` | `!todo due <id> <time>` | Set due date (e.g., `!todo due 1 2d`) |
| `!todo edit` | `!todo edit <id> <new text>` | Edit task description |
| `!todo clear` | `!todo clear [room]` | Remove completed tasks for user or room |

### Reminder Commands
| Command | Usage | Description |
|---|---|---|
| `!remind` | `!remind [daily\|weekly] <time> <msg>` | Set a one-time or recurring timer (e.g. `!remind 10s check oven`) |
| `!remind list` | `!remind list [page]` | View your pending active reminders |
| `!remind cancel` | `!remind cancel <id>` | Cancel a pending reminder `#id` |

---

## 🌐 REST API Endpoints

| Method | Endpoint | Auth Required | Description |
|---|---|---|---|
| `POST` | `/api/v1/auth/register` | No (Secret if set) | Register web account & get JWT token |
| `POST` | `/api/v1/auth/login` | No | Login via local account or Matrix Homeserver credentials (`@user:domain`) |
| `GET` | `/api/v1/health` | No | System health, version & DB connection status |
| `GET` | `/api/v1/todos` | Yes (Bearer/Matrix `syt_`) | List tasks (query params: `room_id`, `page`, `limit`) |
| `POST` | `/api/v1/todos` | Yes (Bearer/Matrix `syt_`) | Create new task (500 char max limit) |
| `PATCH` | `/api/v1/todos/:id` | Yes (Bearer/Matrix `syt_`) | Update text, priority, due date, or completion |
| `DELETE` | `/api/v1/todos/:id` | Yes (Bearer/Matrix `syt_`) | Delete task by ID |
| `GET` | `/api/v1/reminders` | Yes (Bearer/Matrix `syt_`) | List pending reminders (query params: `room_id`, `page`, `limit`) |
| `POST` | `/api/v1/reminders` | Yes (Bearer/Matrix `syt_`) | Create timer in seconds (500 char max limit) |
| `DELETE` | `/api/v1/reminders/:id` | Yes (Bearer/Matrix `syt_`) | Cancel reminder by ID |
| `POST` | `/api/v1/events/ticket` | Yes (Bearer/Matrix `syt_`) | Exchange the API token for a 30-second SSE stream ticket |
| `GET` | `/api/v1/events` | Yes (Bearer, or `?ticket=`) | SSE stream of the caller's own events (`type` field in the JSON body) |

---

## 🛠️ Setup & Running

### 1. Prerequisites
- [Rust & Cargo](https://rustup.rs/) (edition 2021+)
- A Matrix Account on any Matrix Homeserver (e.g. `matrix.org`, `matrix.mbm.one`, Synapse, Conduit).

### 2. Configure Environment

Copy `.env.example` to `.env`:

```bash
cp .env.example .env
```

Edit `.env` with your settings:

```env
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USERNAME=your_bot_username
MATRIX_PASSWORD=your_bot_password
MATRIX_RECOVERY_KEY=your_optional_e2ee_recovery_key
DATABASE_URL=sqlite://data/bot_data.db?mode=rwc
DATABASE_MAX_CONNECTIONS=10
API_HOST=0.0.0.0
API_PORT=8080
JWT_SECRET=matrix_bot_secret_jwt_token_key_change_in_production
API_REGISTRATION_SECRET=optional_secret_to_gate_registration
RUST_LOG=info,matrix_bot=debug,matrix_sdk=info
```

### 3. Run & Test

```bash
# Run unit & integration test suite (26 tests)
cargo test

# Launch the bot & REST API server
cargo run
```

---

## 🚀 Future Roadmap & Planned Features

We are actively expanding `matrix-bot` into a comprehensive personal assistant platform. Planned future modules include:

- 📈 **Portfolio Management Module (`src/modules/portfolio/`)**:
  - Real-time crypto & stock price tracking (`!portfolio add 1.5 BTC`, `!portfolio quote SOL`).
  - Total net worth aggregation and price target alerts via Matrix notifications.
- 💰 **Finance & Expense Manager (`src/modules/finance/`)**:
  - Expense logging with categories (`!expense add $15.50 lunch -c food`).
  - Monthly budget tracking, spending summaries, and export reports.
- 📅 **Subscription Manager (`src/modules/subscription/`)**:
  - Recurring subscription tracking (`!sub add Netflix $15.99/m`).
  - Renewal advance warnings (e.g., notify 3 days before renewal date) and monthly recurring burn rate analytics.
- 🐘 **PostgreSQL Migration Support**:
  - Production deployment capability switching from SQLite to PostgreSQL via `DATABASE_URL` configuration.
- 🎨 **Web Front-End SPA**:
  - Modern React/Next.js interface consuming `/api/v1/` REST endpoints and real-time SSE stream.

---

## 🏗️ Architecture Overview

```
data/                         # Runtime storage directory (ignored in git)
├── bot_data.db               # Application SQLite database (todos, reminders, web_users)
├── bot_data.db-wal           # SQLite Write-Ahead Log for high-concurrency WAL mode
├── bot_data.db-shm           # SQLite shared memory file
└── bot_store/                # Matrix SDK internal session & crypto store

src/
├── main.rs                   # Entry point bootstrapping Config validation, DB, API & Matrix loop (< 40 lines)
├── config.rs                 # Environment configuration & runtime validation rules
├── error.rs                  # Axum ApiError implementation with JSON error response format
├── events.rs                 # SystemEvent enum & EventBroadcaster for SSE streaming
├── api/                      # Axum REST API layer
│   ├── mod.rs                # Router assembly, CORS, static file fallback & launcher
│   ├── auth.rs               # Matrix Homeserver login, Matrix access token whoami, JWT claims & auth handlers
│   ├── health.rs             # System health & database connectivity check endpoint
│   ├── todos.rs              # To-Do REST CRUD handlers & input validation
│   ├── reminders.rs          # Reminder REST CRUD handlers & input validation
│   └── sse.rs                # Server-Sent Events stream with event type discrimination
├── bot/                      # Matrix bot client engine
│   ├── client.rs             # Matrix Client builder, well-known resolution & login
│   ├── dispatcher.rs         # Mention & command parser with HTML pill stripping
│   └── handler.rs            # Event routing for room invites and message events
├── db/                       # Database persistence layer (SQLx)
│   ├── pool.rs               # Connection pool manager with WAL mode & configurable max_connections
│   └── schema.rs             # Table DDL migrations (todos, reminders, web_users)
├── modules/                  # Extensible plugin system
│   ├── mod.rs                # BotModule trait definition, CommandContext & ModuleRegistry
│   ├── core/                 # Core module (!hello, !ping, !help)
│   ├── todo/                 # To-Do module (repository, decomposed sub-handlers, models)
│   └── reminder/             # Reminder module (repository, handler, scheduler, models, status enum)
└── utils/                    # Shared utilities
    ├── html.rs               # HTML escaping & strip_html notification fallback
    └── time.rs               # Duration & date parser utilities
```
