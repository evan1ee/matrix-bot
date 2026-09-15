# AGENTS.md — Development & Architecture Guidelines for AI Agents & Developers

This document defines the architectural conventions, coding rules, frontend design standards, and development practices for working on `matrix-bot`.

---

## 🏛️ Project Architecture & Design Philosophy

1. **Modular `BotModule` Plugin System**:
   - All feature capabilities (e.g., Core, To-Do, Reminder, Portfolio, Finance, Bookmarks, Habits, Life Grid, Webhooks, AI) must be implemented as independent modules implementing the `BotModule` trait defined in [`backend/src/modules/mod.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/modules/mod.rs).
   - Never add feature-specific command handlers directly to `main.rs` or `bot/handler.rs`. Register new modules in [`backend/src/bot/mod.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/bot/mod.rs).

2. **Embedded Axum REST API Layer**:
   - Web API endpoints live in `backend/src/api/` (`auth.rs`, `todos.rs`, `reminders.rs`, `bookmarks.rs`, `finance.rs`, `portfolio.rs`, `habits.rs`, `life.rs`, `sse.rs`, `health.rs`).
   - Use `ApiError` in [`backend/src/error.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/error.rs) for all REST API error responses to guarantee standardized JSON `{ "error": "...", "code": 400 }` output.
   - Every state-modifying action (create, update, delete, trigger) must emit a `SystemEvent` via `EventBroadcaster` in [`backend/src/events.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/events.rs) for real-time SSE streaming.
   - Every `SystemEvent` variant must expose its owner through `SystemEvent::owner_id()`. The broadcast channel is process-wide and the SSE handler delivers an event only to that owner — a variant with no owner would either leak across users or never be delivered.
   - Enforce input length validation (max 500 chars) on all API payload strings to prevent DoS/database bloat.

3. **Strict File Line Limit (< 500 Lines)**:
   - **No source file in `backend/src/` should exceed 500 lines of _production_
     code** (target < 300). The count is lines above the `#[cfg(test)]` module —
     tests live beside the code they cover and are not counted against it.
   - Split along a real seam, never by line count alone. Examples:
     `db/schema.rs` → DDL vs `db/backfill.rs` vs `db/schema_auth.rs`;
     `api/account.rs` → one file per concern;
     `portfolio/handler.rs` → `view.rs` (renders, never writes) and `cash.rs`.
   - Decompose feature modules into standard sub-files:
     - `mod.rs`: Module struct & `BotModule` trait implementation
     - `models.rs`: Structs & Enums (e.g. `ReminderStatus`, `TodoPriority`, `Bookmark`)
     - `repository.rs`: SQLx database CRUD operations
     - `handler.rs`: Decomposed command parsing sub-functions and HTML formatting
     - `scheduler.rs`: (Optional) Background worker tasks

4. **Database & SQLx Persistence Rules**:
   - Use runtime SQLx query binding (`sqlx::query(...)` and `sqlx::query_as::<_, Model>(...)`) with `.bind(...)` rather than compile-time checked macros (`query!`) to allow seamless DB connection switching without compile-time database URL requirements.
   - Always run table DDL migrations in [`backend/src/db/schema.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/db/schema.rs) using `CREATE TABLE IF NOT EXISTS` and backwards-compatible `ALTER TABLE` statements.
   - SQLite pools must be initialized with `WAL` journal mode (`journal_mode=WAL`), `busy_timeout=5000`, and configurable max connections via `DATABASE_MAX_CONNECTIONS`.

5. **Matrix Response & Formatting Rules**:
   - Always construct Matrix responses using HTML formatting (`RoomMessageEventContent::text_html(&plain_text, &reply_html)`).
   - Use `html_escape()` in [`backend/src/utils/html.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/utils/html.rs) for all user-supplied input strings embedded in HTML messages (escaping `<`, `>`, `&`, `"`, `'`).
   - Use `strip_html()` in [`backend/src/utils/html.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/utils/html.rs) for the plain-text fallback argument to ensure push notifications don't render raw HTML tags.
   - **Quoted Message Context (`CommandContext.quoted`)**: Matrix replies (rich `<mx-reply>` HTML and plain `> quote` fallbacks) are parsed in [`backend/src/bot/quote.rs`](backend/src/bot/quote.rs) into `QuotedMessage`. Modules must support quoted context:
     - AI module: injects quoted content into context prompts via `QuotedMessage::to_context_prompt()`.
     - Todo / Reminder: if the user supplies no description or an empty default, uses the quoted text as the task / reminder body.

6. **AI Tools & Live Web Search Architecture**:
   - **Universal Function Calling**: Define tools using standard OpenAI `type: "function"` schema in [`backend/src/modules/ai/search.rs`](backend/src/modules/ai/search.rs). Never use provider-specific proprietary tool variants (e.g. xAI `web_search`) on chat completions to maintain cross-provider compatibility across Grok, OpenAI, Claude, and local models.
   - **Selective Intent Gating**: Web search must be enabled conditionally via `should_enable_web_search()` (fact-checking, verification, breaking news) and omitted for routine productivity commands (todos, reminders, expenses, general coding) to avoid unnecessary search latency and token bloat.
   - **Dual Search Backend**: Primary queries use Tavily Search API (`TAVILY_API_KEY`) returning clean relevance-scored snippets; gracefully falls back to keyless DuckDuckGo HTML scraping if no key is set or on API errors.
   - **Proxy Awareness**: Search HTTP clients must respect `AI_PROXY` / `HTTPS_PROXY` so requests route through the user's configured local proxy.
   - **Concurrent Tool Execution**: When an LLM returns multiple tool calls within a turn, execute them concurrently using `futures_util::future::join_all` rather than sequentially in a loop.
   - **Timeouts & Deduplication**: Long-running multi-turn research requests need generous router ceilings (Axum `TimeoutLayer` >= 180s). Prevent duplicate database records on immediate retries by checking the active conversation's tail message before inserting.
   - **Domain-Grouped Tool Architecture**: AI tools in [`backend/src/modules/ai/tools/`](backend/src/modules/ai/tools/) are organized strictly by domain module (`todo.rs`, `finance.rs`, `reminder.rs`, `portfolio.rs`, `bills.rs`, `habits.rs`, `life.rs`, `bookmarks.rs`, `webhooks.rs`, `settings.rs`). Tool executors directly invoke domain SQLx repositories (`TodoRepository`, `FinanceRepository`, etc.) rather than through anemic pass-through layers, preserving single-responsibility and ensuring files remain well below 500 lines.

7. **Frontend Design System & UI Craft (Following `Design.md`)**:
   All web interfaces must strictly adhere to the Vercel editorial visual standards documented in [`Design.md`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/Design.md):
   - **Outer Shell & Shared Gutters**:
     - The top-level [`AppShell`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/frontend/src/components/AppShell.tsx) provides the outer responsive gutters (`<main className="flex-1 overflow-y-auto px-6 md:px-8 py-6 flex flex-col">`).
     - Feature views must use `<div className="space-y-6">` (or `<div className="flex-1 flex flex-col min-h-0 space-y-4">` for full-height chat/map views) as their root element.
     - **Never** add redundant outer padding (`p-4`, `px-6`) or artificial centering containers (`max-w-5xl mx-auto`, `max-w-6xl mx-auto`) to module root views. Every module must align cleanly with the shared left baseline of the header.
   - **Geist Typography & Strict Roles**:
     - Use **Geist Sans** (`font-sans`) for headings, body prose, labels, controls, tables, KPIs, and descriptions.
     - Use **Geist Mono** (`font-mono` / `text-[10px] font-mono`) strictly for operational identifiers: code, commands, paths, timestamps, tags, badges, domains, and currency codes.
     - Use sentence-case headings. Avoid all-caps overlines, tracked eyebrows, or decorative numbered section prefixes.
   - **Monochrome-First Canvas & Restraint**:
     - Design in monochrome: canvas `bg-white dark:bg-neutral-950`, borders `border-neutral-200 dark:border-neutral-800`, text `text-neutral-900 dark:text-neutral-100` and secondary `text-neutral-500 dark:text-neutral-400`.
     - Reserve color strictly for meaningful state or sourced data (e.g. positive green / negative red cashflow, favorite amber star, high-priority badge), always paired with a textual or geometric non-color cue.
     - **Reject generated-design reflexes**: Never add decorative gradients, glowing blobs, stripes, textures, glassmorphism, or artificial card-in-card nesting.
   - **Default to Stillness**:
     - No pulsing status badges, auto-scrolling tickers, simulated typing animations, or decorative hover bounces.
     - Transitions must be subtle (`transition-colors duration-150`), explaining state changes without delaying the user.
   - **Data & Sourced Evidence**:
     - Use the **Stat Strip** pattern (`grid grid-cols-2 sm:grid-cols-4 gap-3`) for summary metrics with uppercase mono labels and tabular numbers.
     - Tables own the full content width; numeric values and headers must right-align (`text-right`), text values left-align.
   - **Native & Accessible Controls**:
     - Inputs, selects, and textareas must have visible labels, clear focus outlines, and full keyboard accessibility (`Escape` closes modals, `Enter` submits single-line forms).

---

## 🛠️ Module Implementation Checklist

When creating or enhancing a feature module (e.g. `Portfolio`, `Finance`, `Bookmarks`, `Habits`):

1. **Backend Directory Structure**: Create `backend/src/modules/<feature_name>/` containing `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`.
2. **Database Migration**: Add table schema in [`backend/src/db/schema.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/db/schema.rs) (or a modular `schema_<feature>.rs`).
3. **Repository**: Implement SQLx query binding CRUD methods in `repository.rs`.
4. **Command Handler**: Parse Matrix commands, enforce length limits (max 500 chars), and format HTML output in `handler.rs`.
5. **REST API Endpoints**: Add REST handlers in `backend/src/api/<feature_name>.rs` returning `Result<_, ApiError>`, and register in `backend/src/api/mod.rs`.
6. **Emit SystemEvents**: Add corresponding `SystemEvent` enum variants in `backend/src/events.rs` with `owner_id()` and broadcast them on every state change.
7. **BotModule Registration**: Implement `BotModule` in `mod.rs` and register `FeatureModule::new()` in [`backend/src/bot/mod.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/bot/mod.rs).
8. **OpenAPI & Types**: Update `ApiDoc` in [`backend/src/api/openapi.rs`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/backend/src/api/openapi.rs), then run:
   ```bash
   cargo run --bin generate-openapi
   npm --prefix frontend run generate-api
   ```
9. **Frontend Web UI**:
   - Create `frontend/src/modules/<feature_name>/` (`index.tsx`, `types.ts`, components).
   - Export `<feature>Module: FrontendModule` with `id`, `name`, `iconName`, `navTab`, and `handleSseEvent`.
   - Register module in [`frontend/src/App.tsx`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/frontend/src/App.tsx).
   - Add navigation item under appropriate group in [`frontend/src/components/Sidebar.tsx`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/frontend/src/components/Sidebar.tsx).
   - Add module label in [`frontend/src/components/AppShell.tsx`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/frontend/src/components/AppShell.tsx).
10. **E2E & Unit Tests**:
    - Add backend unit tests in `repository.rs`, `handler.rs`, and `api/<feature_name>.rs` using `sqlite::memory:`.
    - Add mock routes in [`frontend/e2e/fake-api.ts`](file:///Users/nosinx/Documents/GitHub/matrix-hello-bot/frontend/e2e/fake-api.ts).
    - Write Playwright E2E specs in `frontend/e2e/<feature_name>.spec.ts`.

---

## 🧪 Testing & Verification Requirements

Before submitting any code change:

```bash
# 1. Verify zero compilation warnings & errors in backend
cargo check

# 2. Execute all backend unit & integration tests (must pass 100%)
cargo test

# 3. Verify frontend TypeScript & build passes without errors
npm --prefix frontend run build

# 4. Execute Playwright E2E tests (must pass 100%)
npx --prefix frontend playwright test
```

Never commit code that fails compilation, lints, or tests.

---

## 💡 Design & Implementation Principles

- **Zero Backward Compatibility Baggage**: Do not preserve backward compatibility. Remove obsolete paths instead of adding compatibility layers, fallbacks, or migrations.
- **Simplest Working Implementation**: Choose the simplest implementation that fully meets the current requirements. Avoid speculative abstractions, premature configuration, and indirection.
- **Grow in Layers**: Start from the smallest version that works end to end, and add each new capability on top of a product that already works. Never trade a working product for unfinished complexity.
- **Modular Separation of Concerns**: Keep components modular and concerns clearly separated (API, DB, Matrix handler, UI, Events).
- **Lean on Existing Libraries**: Prefer established dependencies already in the workspace before adding new packages or writing custom implementations.
- **Long-Term Architectural Craft**: Make architectural decisions for the long term. Do not accept temporary stopgaps that only work for now and are meant to be replaced later.