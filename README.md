# Matrix Bot (Rust) 🦀

A production-ready, highly modular, multi-feature Matrix bot written in Rust using the official [`matrix-sdk`](https://github.org/matrix-org/matrix-rust-sdk) and [`sqlx`](https://github.org/launchbadge/sqlx).

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
- 💾 **SQLx Persistence Layer**:
  - SQLite default with automatic WAL mode (`journal_mode=WAL`) and busy timeout configuration.
  - Runtime storage cleanly isolated inside the `data/` directory.
  - Abstract repository interface ready for seamless PostgreSQL migration.
- 🧱 **Extensible `BotModule` Architecture**:
  - Trait-based plugin system where new features live in isolated submodules under `src/modules/`.
  - Clean code guarantee: **Zero source files exceed 300 lines of code**.

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

## 🛠️ Setup & Running

### 1. Prerequisites
- [Rust & Cargo](https://rustup.rs/) (edition 2021+)
- A Matrix Account on any Matrix Homeserver (e.g. `matrix.org`, `matrix.mbm.one`, Synapse, Conduit).

### 2. Configure Environment

Copy `.env.example` to `.env`:

```bash
cp .env.example .env
```

Edit `.env` with your Matrix credentials:

```env
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USERNAME=your_bot_username
MATRIX_PASSWORD=your_bot_password
MATRIX_RECOVERY_KEY=your_optional_e2ee_recovery_key
DATABASE_URL=sqlite://data/bot_data.db?mode=rwc
RUST_LOG=info,matrix_bot=debug,matrix_sdk=info
```

### 3. Run & Test

```bash
# Run unit & integration tests
cargo test

# Launch the bot
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
- 🌐 **Web Admin Dashboard & Metrics API**:
  - Optional web dashboard for system health, active module metrics, and room management.

---

## 🏗️ Architecture Overview

```
data/                         # Runtime storage directory (ignored in git)
├── bot_data.db               # Application SQLite database (todos & reminders)
├── bot_data.db-wal           # SQLite Write-Ahead Log for high-concurrency WAL mode
├── bot_data.db-shm           # SQLite shared memory file
└── bot_store/                # Matrix SDK internal session & crypto store

src/
├── main.rs                   # Clean entry point bootstrapping DB, modules & Matrix loop (< 40 lines)
├── config.rs                 # Environment & bot configuration management
├── error.rs                  # Custom error types and anyhow context
├── bot/                      # Matrix bot client engine
│   ├── client.rs             # Matrix Client builder, well-known resolution & login
│   ├── dispatcher.rs         # Mention & command parser with HTML pill stripping
│   └── handler.rs            # Event routing for room invites and message events
├── db/                       # Database persistence layer (SQLx)
│   ├── pool.rs               # Connection pool manager with SQLite WAL mode
│   └── schema.rs             # Table DDL migrations (todos & reminders)
├── modules/                  # Extensible plugin system
│   ├── mod.rs                # BotModule trait definition & ModuleRegistry dispatcher
│   ├── core/                 # Core module (!hello, !ping, !help)
│   ├── todo/                 # To-Do module (repository, handler, models)
│   └── reminder/             # Reminder module (repository, handler, scheduler, models)
└── utils/                    # Shared utilities
    ├── html.rs               # HTML escaping & strip_html notification fallback
    └── time.rs               # Duration & date parser utilities
```
