# AGENTS.md — Development & Architecture Guidelines for AI Agents & Developers

This document defines the architectural conventions, coding rules, and development practices for working on `matrix-bot`.

---

## 🏛️ Project Architecture & Design Philosophy

1. **Modular `BotModule` Plugin System**:
   - All feature capabilities (e.g., Core, To-Do, Reminder, Portfolio, Finance) must be implemented as independent modules implementing the `BotModule` trait defined in [`src/modules/mod.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/modules/mod.rs).
   - Never add feature-specific command handlers directly to `main.rs` or `bot/handler.rs`. Register new modules in [`src/bot/mod.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/bot/mod.rs).

2. **Embedded Axum REST API Layer**:
   - Web API endpoints live in `src/api/` (`auth.rs`, `todos.rs`, `reminders.rs`, `sse.rs`, `health.rs`).
   - Use `ApiError` in [`src/error.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/error.rs) for all REST API error responses to guarantee standardized JSON `{ "error": "...", "code": 400 }` output.
   - Every state-modifying action (create, update, delete, trigger) must emit a `SystemEvent` via `EventBroadcaster` in [`src/events.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/events.rs) for real-time SSE streaming.
   - Enforce input length validation (max 500 chars) on all API payload strings to prevent DoS/database bloat.

3. **Strict File Line Limit (< 500 Lines)**:
   - **No single source file in `src/` should exceed 500 lines of code** (target < 300 lines).
   - Decompose feature modules into standard sub-files:
     - `mod.rs`: Module struct & `BotModule` trait implementation
     - `models.rs`: Structs & Enums (e.g. `ReminderStatus`, `TodoPriority`)
     - `repository.rs`: SQLx database CRUD operations
     - `handler.rs`: Decomposed command parsing sub-functions and HTML formatting
     - `scheduler.rs`: (Optional) Background worker tasks

4. **Database & SQLx Persistence Rules**:
   - Use runtime SQLx query binding (`sqlx::query(...)` and `sqlx::query_as::<_, Model>(...)`) with `.bind(...)` rather than compile-time checked macros (`query!`) to allow seamless DB connection switching without compile-time database URL requirements.
   - Always run table DDL migrations in [`src/db/schema.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/db/schema.rs) using `CREATE TABLE IF NOT EXISTS` and backwards-compatible `ALTER TABLE` statements.
   - SQLite pools must be initialized with `WAL` journal mode (`journal_mode=WAL`), `busy_timeout=5000`, and configurable max connections via `DATABASE_MAX_CONNECTIONS`.

5. **Matrix Response & Formatting Rules**:
   - Always construct Matrix responses using HTML formatting (`RoomMessageEventContent::text_html(&plain_text, &reply_html)`).
   - Use `html_escape()` in [`src/utils/html.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/utils/html.rs) for all user-supplied input strings embedded in HTML messages (escaping `<`, `>`, `&`, `"`, `'`).
   - Use `strip_html()` in [`src/utils/html.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/utils/html.rs) for the plain-text fallback argument to ensure push notifications don't render raw HTML tags.

---

## 🛠️ Module Implementation Checklist

When creating a new feature module (e.g. `Portfolio`, `Finance`, `Subscription`):

1. **Create directory structure**: `src/modules/<feature_name>/` containing `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`.
2. **Implement Database Migration**: Add required table schema in [`src/db/schema.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/db/schema.rs).
3. **Implement Repository**: Add async SQL CRUD methods in `repository.rs`.
4. **Implement Handler**: Parse user input, enforce quota & length limits (max 500 chars), and format HTML output in `handler.rs`.
5. **Implement REST API Endpoints**: Add corresponding REST CRUD handlers under `src/api/<feature_name>.rs` returning `Result<_, ApiError>`.
6. **Emit SystemEvents**: Add corresponding `SystemEvent` enum variants in `src/events.rs` and broadcast them when data changes.
7. **Implement `BotModule` Trait**: Define `name()`, `commands()`, and `handle_command()` in `mod.rs`.
8. **Register Module**: Add `registry.register(FeatureModule::new());` in [`src/bot/mod.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/src/bot/mod.rs) and router routes in `src/api/mod.rs`.
9. **Write Unit Tests**: Add `#[cfg(test)]` tests in `repository.rs`, `handler.rs`, and `api/<feature_name>.rs` using SQLite in-memory database (`sqlite::memory:`).

---

## 🧪 Testing & Verification Requirements

Before submitting any code change:

```bash
# 1. Verify zero compilation warnings & errors
cargo check

# 2. Execute all unit & integration tests (must pass 100%)
cargo test
```

Never commit code that fails `cargo check` or `cargo test`.
