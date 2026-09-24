# Feature Proposals

Implementation notes for four candidate features, written against the tree at
`bdb5f68`. Each section is meant to be enough to start from cold.

These proposals follow the **Working Principles** in `AGENTS.md`:
no compatibility shims, simplest implementation that meets the requirement, grow
in layers on top of a product that already works, and lean on dependencies that
are already here (`matrix-sdk`, `sqlx`, `chrono`) before adding new ones.

## Context: the gap these address

The Matrix side is currently a text-command CRUD front-end to the same rows the
web UI shows. Nothing uses Matrix *as a platform*:

- `bot/handler.rs::register_handlers` registers exactly two handlers — auto-join
  on invite, and `OriginalSyncRoomMessageEvent`. No reactions, no state events.
- Every row is private to one `owner_id`; there is no shared or room-scoped state.
- Nothing is ever pushed into a room except a reminder firing.

Ranked by value; recommended build order is **2 → 3 → 1**, with 4 deliberately
deferred.

---

## 1. Room-shared lists

**Status:** proposed, not scheduled. Large, and rewrites a security invariant.

A todo created with `--shared` is owned by the *room* rather than one user, and
is visible to every joined member. This is what turns the bot from a private
notepad that happens to live in a chat into an actual group tool.

### Why it is the hard one

`AGENTS.md` is emphatic that `AuthUser::owner_id()` is *the* single ownership
key, matching `sender_mxid` and nothing else. Shared rows break that: ownership
becomes "the room", and visibility becomes a membership question that only the
Matrix client can answer.

The SSE layer is where this actually bites. Today `api/sse.rs:23`:

```rust
fn owned_events(broadcaster: &EventBroadcaster, owner_id: String) -> impl Stream<Item = SystemEvent> {
    // ... (msg.owner_id() == owner_id).then_some(msg)
}
```

A string equality against one owner. For shared rows this has to become "is this
subscriber a joined member of the row's room", which needs the bot's
`Arc<RwLock<Option<Client>>>` inside the SSE handler and an async membership
check per event. That is a real design decision, not a patch.

### Sketch

1. `todos` gains `visibility TEXT NOT NULL DEFAULT 'private'` (`private` |
   `room`). Added in `db/schema.rs::run_migrations` as a best-effort
   `ALTER TABLE` alongside the existing ones.
2. Shared rows keep a real Matrix `room_id` (never the `web` sentinel) — the
   room *is* the ownership key.
3. `SystemEvent::owner_id()` becomes `audience()`, returning an enum:
   ```rust
   enum Audience { Owner(String), Room(String) }
   ```
   Every existing variant returns `Owner(..)`, so the change is mechanical; the
   deletion variants already carry both `room_id` and `owner_id`.
4. `owned_events` takes the subscriber's `AuthUser` plus the bot client and
   resolves `Audience::Room(id)` via `client.get_room(id)` +
   `is_joined_member(...)` — the same helper `api/rooms.rs` already uses.
   Memoize per stream; do not do a homeserver round trip per event.
5. `TodoRepository` gains `list_room_shared` and the existing `list_all_user`
   grows a union over rooms the caller is in.

### Risks to hold in mind

- A membership check per SSE event is a leak waiting to happen if it fails open.
  Fail closed: an unresolvable room means the event is dropped, not delivered.
- Local (`@web-<username>:api`) accounts are in no Matrix room at all. Decide
  explicitly whether they can see shared lists — the honest answer is no, and
  the UI should say so rather than showing an empty list.
- `sender_name` still must never appear in any predicate.

### Effort

Large. Touches `events.rs`, `api/sse.rs`, `api/todos.rs`, `todo/repository.rs`,
`todo/handler.rs`, `db/schema.rs`, and the frontend todo module. Do it as its own
piece of work with its own tests, not folded into anything else.

---

## 2. Reaction-driven completion — recommended first build

React ✅ to the bot's todo confirmation message to complete it, 🗑️ to delete it.
Zero typing on mobile, and nothing else in the product has it.

### Where the hook already exists

`bot/handler.rs` currently throws away exactly the value this needs:

```rust
if let Err(err) = room.send(content).await {
    error!("Failed to send message to room {}: {:?}", room.room_id(), err);
}
```

`room.send(...)` resolves to a response carrying the sent `event_id`. Capture it
instead of discarding it, and a reaction's `relates_to.event_id` maps straight
back to the row.

### Sketch

1. **Schema.** `todos` gains `event_id TEXT` (nullable — rows created through
   the REST path have no Matrix message). Best-effort `ALTER TABLE` in
   `run_migrations`, plus:
   ```sql
   CREATE INDEX IF NOT EXISTS idx_todos_event_id ON todos(event_id);
   ```

2. **Capture the event ID.** `ModuleRegistry::dispatch` currently returns
   `Option<String>` (the HTML reply). The module that created a row needs to
   learn the event ID *after* the reply is sent, which the current signature
   cannot express. Simplest option that meets the requirement: have the todo
   module return the new row's `id` alongside the reply, and let `handler.rs`
   write the mapping after a successful send.

   Change `dispatch` to return `Option<DispatchOutcome>`:
   ```rust
   pub struct DispatchOutcome {
       pub reply_html: String,
       /// Set when the reply confirms creation of a todo, so the caller can
       /// link the sent Matrix event back to the row for reaction handling.
       pub links_todo: Option<i64>,
   }
   ```
   Per the no-compatibility-layers principle, change `BotModule::handle_command`
   to return this directly rather than adding a parallel method. Three modules
   implement it; two return `links_todo: None` unconditionally.

3. **Reaction handler.** A third `client.add_event_handler` in
   `register_handlers`, on `OriginalSyncReactionEvent`:
   - Ignore reactions from the bot itself (same `own_id` check as the message
     handler) and any room not in `RoomState::Joined`.
   - Read `ev.content.relates_to.key` — match `✅`/`☑️` and `🗑️`, ignore the rest.
   - Look up the todo by `relates_to.event_id`.
   - **Verify `ev.sender` equals the row's `sender_mxid` before mutating.**
     `TodoRepository::mark_done(pool, id)` takes no owner argument and will
     happily complete anyone's row — the ownership check belongs in the handler,
     or add an owner-scoped repository method. This is the security-relevant
     part of the feature; do not skip it.
   - Broadcast `SystemEvent::TodoUpdated` / `TodoDeleted` so the web UI updates
     live. No new event variants needed, so no `owner_id()` work.

4. **Redaction.** A user removing their reaction sends a redaction, not another
   reaction. Ignore it — un-completing on un-react is more state to track than
   the feature is worth. `!todo undone <id>` already exists.

### Effort

Roughly 150 lines of backend, one schema line, one index, zero frontend. The
`dispatch` signature change is the only part that touches files outside
`bot/` and `modules/todo/`.

### Tests

- Reaction from a non-owner leaves the row untouched.
- Unknown emoji is a no-op.
- Reaction on an event ID with no matching row is a no-op (not an error).
- `DispatchOutcome` round-trip: creating a todo yields `links_todo: Some(id)`.

---

## 3. Daily agenda digest

Each morning the bot posts today's due todos and pending reminders into a
chosen room. Pure layering — it introduces no new concepts.

### Why it is nearly free

`spawn_background_workers(&ModuleContext)` already hands a worker the Matrix
`Client`, the `Database`, and the broadcaster. `reminder/scheduler.rs` already
contains everything the digest needs:

- `deliver_to_matrix(ctx, reminder, content)` — send-with-join-retry, reusable
  as-is once generalized from `&Reminder` to `(room_id, content)`.
- `next_occurrence(current, recurrence, now)` — a daily digest is just
  `recurrence = "daily"`, including the catch-up loop for downtime.

### Sketch

1. New table:
   ```sql
   CREATE TABLE IF NOT EXISTS digest_prefs (
       sender_mxid TEXT PRIMARY KEY,
       room_id     TEXT NOT NULL,
       send_at     TEXT NOT NULL,   -- "HH:MM" local to the configured TZ
       next_at     TIMESTAMP NOT NULL,
       enabled     BOOLEAN NOT NULL DEFAULT 1
   );
   ```
   `next_at` is the scheduled instant, so the worker query matches the shape the
   reminder scheduler already uses (`WHERE enabled = 1 AND next_at <= ?`).

2. Second worker on the same 5s tick, registered from the digest module's
   `spawn_background_workers`. On fire: gather, render, send via the shared
   delivery helper, then `next_at = next_occurrence(next_at, "daily", now)`.

3. Commands: `!digest on 08:00`, `!digest off`, `!digest` (show current setting).

4. Rendering reuses `utils::html.rs` — `html_escape` for every user string,
   `strip_html` for the plain-text fallback body, per `AGENTS.md`.

### Deliberate non-goals

- No timezone selector in v1. Use one configured server timezone; a per-user TZ
  is speculative configuration until someone asks for it.
- No SSE event. Nothing in the web UI changes when a digest fires. If a settings
  page is added later, that is when a `DigestUpdated` variant earns its place —
  and it will need a meaningful `owner_id()`.

### Effort

~200 lines, all inside `modules/`. Extracting the delivery helper out of
`scheduler.rs` is the only edit to existing code.

---

## 4. Natural-language capture — deferred

`@bot remind me to call the bank tomorrow at 9` → a real reminder row.

Today `reminder/handler.rs:17` requires the message to literally start with
`remind`, and `utils::parse_reminder_args` handles the rest positionally.

### Shape if built

A new module that calls the Claude API with **strict tool use** so the model
returns a schema-validated `{intent, text, due_at, recurrence}` and the module
dispatches into the existing `TodoRepository` / `ReminderRepository`. No new
parser to maintain, no free-text JSON to clean up.

- `strict: true` on the tool definition, with `additionalProperties: false` and
  a complete `required` list.
- Model `claude-opus-5`. Thinking is on by default there, so `max_tokens` must
  leave room for it or responses truncate; `output_config: {effort: "low"}` for
  a latency-sensitive router.
- Register it **last** in `bot/mod.rs::new_full` so it only sees messages no
  deterministic module claimed. `ModuleRegistry::dispatch` returns the first
  `Some`, so ordering is the whole fallback mechanism — no extra plumbing.

### Why it is deferred

It is the only proposal that trades a self-contained binary for an external
network dependency and an API key in `Config`, and the existing `!` commands
already cover the same ground. Worth revisiting once 2 and 3 are in and the
command surface is broad enough that discovering it by typing is the actual
bottleneck.

---

## Cross-cutting checklist

Applies to any of the above (see `AGENTS.md` for the authoritative version):

- Every state mutation broadcasts a `SystemEvent` — from **both** the Matrix
  command path and the REST path. A mutation without an event silently breaks
  the live UI.
- Any new `SystemEvent` variant returns a meaningful `owner_id()`. A variant
  without one either leaks or is dropped.
- Ownership predicates use `owner_id()` / `sender_mxid` only. `sender_name` is
  display-only and collides across the two identity spaces.
- Runtime-bound queries (`sqlx::query_as::<_, Model>(...).bind(...)`), never the
  `query!` macros.
- Schema changes go in `db/schema.rs::run_migrations` — idempotent `CREATE TABLE
  IF NOT EXISTS`, best-effort `ALTER TABLE` with the error swallowed.
- After touching any `#[utoipa::path]` or `ToSchema`, regenerate both artifacts:
  ```bash
  cargo run --bin generate-openapi
  npm --prefix frontend run generate-api
  ```
- `cargo check` must be warning-free; `cargo test` currently passes 60 tests.
