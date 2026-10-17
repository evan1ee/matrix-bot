# Todo Tags, Recurrence and Checklist Steps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give todos free-form tags, due-date-anchored recurrence that spawns a successor on completion, and an ordered checklist of steps, across chat, REST, AI/MCP and the web UI.

**Architecture:** Tags are a `TEXT[]` column with a GIN index; recurrence reuses `domain::reminders::RecurrencePattern` plus a `next_todo_id` back-link that makes spawning idempotent; steps live in a new owner-keyed `todo_steps` table. All rules live in `domain/todos` and `TodoService`; every interface calls the service. A shared `domain::tags::Tag` replaces the notes-only tag normalizer.

**Tech Stack:** Rust (Axum, sqlx compile-checked macros, PostgreSQL 18, utoipa), React 19 + Vite + Tailwind v4, Playwright.

**Spec:** [`docs/superpowers/specs/2026-10-18-todo-tags-recurrence-steps.md`](../specs/2026-10-18-todo-tags-recurrence-steps.md)

## Global Constraints

- Work on branch `feat/todo-tags-recurrence-steps` in a separate worktree (AGENTS.md §9); never touch the main checkout's `.env` or `data/`; never run the bot binary.
- Tag: trimmed, one leading `#` stripped, `str::to_lowercase`, chars `char::is_alphanumeric()` or `_`/`-`, 1–50 characters. Max **10** tags per todo.
- Steps: max **50** per todo; step text ≤ **200** characters; todo text ≤ 500 (unchanged).
- Recurrence values: `none | daily | weekly | monthly | yearly`; anything but `none` requires a due date.
- Successor due date: `next_occurrence_in_tz(recurrence_anchor, recurrence, max(now, due_at), owner_tz)` — strictly after both now and the current due date; the anchor keeps the series' day-of-month.
- Step mutations publish `TodoUpdated`; no new `SystemEvent` variants.
- Every new owner-keyed table goes into `OWNED_TABLES`; `.sqlx/` regenerated after each query change; migration files LF.
- No production file in `backend/src/` over 500 lines (`#[cfg(test)]` excluded).
- Chat inputs ≤ 500 chars; every user value in HTML through `html_escape()`; replies via `text_html`.
- REST error body `{ "error", "code" }` via `ApiError`; another owner's todo/step → 404.
- Commits: Conventional Commits ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not push, merge or open a PR.

## Rulings made while planning

Recorded in `docs/superpowers/HANDOFF.md` in Task 10.

- Ruling: completion through `PATCH /todos/{id}` (`completed: true`) and through `set_completed` share one locked code path — the HTTP UI completes via PATCH today, so spawning only in `set_completed` would silently skip the web — cost if wrong: web-completed recurring todos never recur.
- Ruling: chat `add` accepts `due <duration>` using the existing relative parser (`parse_reminder_args`, e.g. `due 3d`), not calendar phrases like "1st 9am" — the spec said existing due parsing stays — cost if wrong: users set the first due date with `!todo due` instead.
- Ruling: REST completion reports the successor via `next_todo_id` on the returned `TodoItem` (the successor arrives through the list/SSE refresh) rather than a new wrapper body — keeps the PATCH response shape additive — cost if wrong: one extra GET in clients that want the successor immediately.
- Ruling: add a `recurrence_anchor TIMESTAMPTZ` column (spec amended) — stepping from each successor's own clamped due date would drift Jan 31 → Feb 28 → Mar 28, contradicting the spec's Jan 31 → Feb 28 → Mar 31 test — cost if wrong: one nullable column carried through every todo query.
- Ruling: new `TodoItem` fields carry `#[serde(default)]` — outbox payloads written before the migration must still deserialize — cost if wrong: replaying an old `TodoCreated` fails.

## Review Focus

1. **Recurring todo completed through the web PATCH** — must spawn exactly like `!todo done`. Test: Task 3 `patch_completing_a_recurring_todo_spawns_a_successor`.
2. **Clearing the due date of a recurring todo** (`!todo due 5 none`, PATCH `due_at: null`) — must be refused with a validation message, not hit the DB `CHECK` as a 500. Test: Task 3 `clearing_due_on_recurring_todo_is_a_validation_error`.
3. **Setting recurrence on a todo with no due date** (`!todo repeat 5 monthly`, AI `update_todo`) — a clear validation error naming the due date. Test: Task 3 `recurrence_without_due_is_refused_on_patch`.
4. **A todo whose text contains `#` that is not a tag** (`Fix bug #123`, `C# course`) — chat extraction takes only `#word` tokens that parse as tags and are separated by whitespace; `#123` is a valid tag, so it is extracted — the text keeps its other words. Test: Task 7 `extracts_tags_and_keeps_remaining_text`.
5. **Old outbox payloads without the new fields** — must deserialize. Test: Task 2 `todo_item_deserializes_without_new_fields`.

---

## File Structure

| File | Responsibility |
|---|---|
| `backend/src/domain/tags/mod.rs` (new) | `Tag`, `TagError`, `parse_tags`, `normalize_tags_lenient` |
| `backend/src/domain/knowledge/note.rs` | switch to `domain::tags` |
| `backend/migrations/0012_todo_tags_recurrence_steps.sql` (new) | DDL from spec §4.1 |
| `backend/src/domain/todos/mod.rs` | new `TodoItem`/`NewTodo`/`TodoPatch` fields, validation, `TodoError` |
| `backend/src/domain/todos/recurrence.rs` (new) | pure `successor` |
| `backend/src/domain/todos/steps.rs` (new) | `TodoStep`, `TodoWithSteps`, step validation |
| `backend/src/domain/accounts/mod.rs` | `todo_steps` in `OWNED_TABLES` |
| `backend/src/infrastructure/postgres/repositories/todos.rs` | new columns, tag filter, `lock_owned`, `set_next_todo_id`, `list_tags`, step counts |
| `backend/src/infrastructure/postgres/repositories/todo_steps.rs` (new) | step CRUD, renumber, copy |
| `backend/src/application/todos/mod.rs` | create/update/list/count changes |
| `backend/src/application/todos/completion.rs` (new) | locked completion + successor spawn |
| `backend/src/application/todos/steps.rs` (new) | step use cases |
| `backend/src/application/reminders/…` | `last_room_for_source` |
| `backend/src/interface/http/todos.rs` | DTO fields, `?tag=`, `GET /todos/{id}`, `GET /todos/tags` |
| `backend/src/interface/http/todo_steps.rs` (new) | step endpoints |
| `backend/src/interface/chat/todos/{tags,steps}.rs` (new), `handler.rs`, `mutate.rs`, `view.rs`, `mod.rs` | chat syntax |
| `backend/src/llm/tools/todo.rs`, `todo_steps.rs` (new), `definitions.rs`, `executor.rs` | AI tools |
| `frontend/src/modules/todo/**`, `frontend/e2e/fake-api.ts`, `frontend/e2e/todos.spec.ts` | web UI |

---

### Task 1: Shared `Tag` value object

**Files:**
- Create: `backend/src/domain/tags/mod.rs`
- Modify: `backend/src/domain/mod.rs` (add `pub mod tags;`), `backend/src/domain/knowledge/note.rs:236-271`
- Test: unit tests in `domain/tags/mod.rs`; existing notes tests

**Interfaces:**
- Produces:
  - `pub struct Tag(String)` — `#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]`, `#[serde(transparent)]`; `Tag::parse(&str) -> Result<Tag, TagError>`, `Tag::as_str(&self) -> &str`, `impl Display`.
  - `pub enum TagError { Empty, TooLong { max: usize }, InvalidChars(String) }` (`thiserror`).
  - `pub const MAX_TAG_LEN: usize = 50;`
  - `pub fn parse_tags(raw: &[String]) -> Result<Vec<Tag>, TagError>` — strict, dedupe, sorted.
  - `pub fn normalize_tags_lenient(raw: &[String]) -> Vec<Tag>` — drops invalid, dedupe, sorted.
  - Deserialize goes through `Tag::parse` (implement `TryFrom<String>` and `#[serde(try_from = "String", into = "String")]` instead of `transparent`) so a stored or wire value can never bypass the rule.

- [ ] **Step 1: Write failing tests** in `domain/tags/mod.rs`:
  - `parse_strips_hash_and_lowercases`: `Tag::parse("  #Work ")` → `"work"`.
  - `parse_accepts_unicode`: `"#工作"` → `"工作"`; `"Café"` → `"café"`.
  - `parse_rejects_empty`: `""`, `"#"`, `"   "` → `TagError::Empty`.
  - `parse_rejects_over_50_chars`: 51 × `"a"` → `TooLong { max: 50 }`; 50 × `"a"` ok.
  - `parse_rejects_spaces_and_punctuation`: `"a b"`, `"a.b"`, `"a#b"` → `InvalidChars`.
  - `parse_tags_dedupes_and_sorts`: `["b","#A","a"]` → `["a","b"]`; `["ok","bad tag"]` → `Err`.
  - `lenient_drops_invalid`: `["ok","bad tag","OK"]` → `["ok"]`.
  - `every_previously_valid_ascii_tag_parses_identically`: for `["rust","my-tag","a_1","ABC"]` assert `Tag::parse(x).as_str() == x.trim_start_matches('#').to_ascii_lowercase()`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot domain::tags::` — expect FAIL (module missing).
- [ ] **Step 3: Implement** the items listed under Produces.
- [ ] **Step 4: Switch notes.** In `note.rs` make `normalize_tag(tag) -> Result<String, KnowledgeError>` delegate to `Tag::parse` (map every `TagError` to `KnowledgeError::InvalidTag(err.to_string())`), and `normalize_tags` delegate to `normalize_tags_lenient` returning `Vec<String>`. Keep both signatures so callers do not change.
- [ ] **Step 5: Run** `cargo test -p matrix-bot domain::tags:: domain::knowledge::` — expect PASS.
- [ ] **Step 6: Commit** `refactor(domain): extract shared Unicode-aware Tag value object`.

---

### Task 2: Schema, todo domain fields and repository

**Files:**
- Create: `backend/migrations/0012_todo_tags_recurrence_steps.sql`, `backend/src/domain/todos/steps.rs`
- Modify: `backend/src/domain/todos/mod.rs`, `backend/src/domain/accounts/mod.rs:167-191`, `backend/src/infrastructure/postgres/repositories/todos.rs`, every `TodoItem { … }` / `NewTodo { … }` literal (find with `rg "NewTodo \{|TodoItem \{" backend/src`), `.sqlx/`
- Test: `repositories/todos.rs` tests, `domain/todos` tests

**Interfaces:**
- Consumes: `Tag` (Task 1), `RecurrencePattern` (`domain::reminders`).
- Produces:
  - `TodoItem` gains `#[serde(default)] pub tags: Vec<Tag>`, `#[serde(default)] pub recurrence: RecurrencePattern`, `#[serde(default)] pub recurrence_anchor: Option<DateTime<Utc>>`, `#[serde(default)] pub next_todo_id: Option<i64>` (add `Default` for `RecurrencePattern` = `None` if missing).
  - `NewTodo` gains `pub tags: Vec<Tag>`, `pub recurrence: RecurrencePattern`, `pub recurrence_anchor: Option<DateTime<Utc>>` (`None` from interfaces; the service sets it).
  - Anchor rule (service, Tasks 2–3): on create with recurrence, and on any patch that sets recurrence from `None` or changes `due_at` of a recurring todo, `recurrence_anchor = due_at`; setting recurrence to `None` clears it; a successor inherits it.
  - `TodoPatch` gains `pub tags: Option<Vec<Tag>>`, `pub recurrence: Option<RecurrencePattern>`.
  - `pub const MAX_TAGS: usize = 10;`
  - `pub enum TodoError` (`thiserror`): `TooManyTags`, `RecurrenceNeedsDue`, `TooManySteps`, `StepTextEmpty`, `StepTextTooLong`, `StepNotFound`, `StepOrderMismatch`, plus `Text(&'static str)` wrapping today's text/lead messages. `NewTodo::validate` / `TodoPatch::validate` return `Result<(), TodoError>`; `impl From<TodoError> for AppError` (→ `Validation(err.to_string())`) in `application/todos`.
  - `pub fn validate_merged(todo: &TodoItem) -> Result<(), TodoError>` — the post-patch invariant (recurrence needs due; ≤ 10 tags).
  - `domain/todos/steps.rs`: `TodoStep { id: i64, todo_id: i64, position: i32, text: String, done: bool }`, `TodoWithSteps { todo: TodoItem, steps: Vec<TodoStep> }`, `pub const MAX_STEPS: usize = 50; pub const MAX_STEP_TEXT_LEN: usize = 200;`, `pub fn validate_step_text(&str) -> Result<String, TodoError>` (trimmed, non-empty, ≤ 200 chars counted with `chars().count()`).
  - Repository (`repositories::todos`):
    - every `RETURNING`/`SELECT` adds `tags, recurrence, recurrence_anchor, next_todo_id`; `TodoRow` gains `tags: Vec<String>, recurrence: String, recurrence_anchor: Option<DateTime<Utc>>, next_todo_id: Option<i64>`; `From<TodoRow>` uses `normalize_tags_lenient` and `RecurrencePattern::from_str_or_default`.
    - `create` / `update` write `tags` (`&[String]`) and `recurrence`; `update` uses `CASE WHEN $n THEN $m ELSE tags END` for a present `tags`.
    - `list_all_user`, `count_all_user`, `list_room_user`, `count_room_user` gain `tag: Option<&str>` → `AND ($k::text IS NULL OR tags @> ARRAY[$k::text])`.
    - `pub async fn lock_owned(conn, owner: &AccountId, id: i64) -> Result<Option<TodoItem>, sqlx::Error>` — `SELECT … FOR UPDATE`.
    - `pub async fn set_next_todo_id(conn, id: i64, next: i64) -> Result<(), sqlx::Error>`.
    - `pub async fn list_tags(conn, owner: &AccountId) -> Result<Vec<(String, i64)>, sqlx::Error>` — `SELECT t, COUNT(*) FROM todos, unnest(tags) t WHERE owner_id = $1 AND NOT completed GROUP BY t ORDER BY t`.

- [ ] **Step 1: Write the migration** exactly as spec §4.1 (including `recurrence_anchor`), LF line endings.
- [ ] **Step 2: Add `todo_steps` to `OWNED_TABLES`** after `"todos"`.
- [ ] **Step 3: Write failing domain tests** in `domain/todos/mod.rs`:
  - `new_todo_rejects_recurrence_without_due`: `recurrence: Monthly, due_at: None` → `Err(TodoError::RecurrenceNeedsDue)`.
  - `new_todo_rejects_eleven_tags` → `TooManyTags`; ten tags ok.
  - `validate_merged_rejects_recurring_without_due`.
  - `todo_item_deserializes_without_new_fields`: `serde_json::from_value::<TodoItem>` of a JSON object holding only the pre-migration fields → `tags == []`, `recurrence == None`, `next_todo_id == None`.
  - in `steps.rs`: `step_text_is_trimmed_and_capped` (200 chars ok, 201 → `StepTextTooLong`, `"  "` → `StepTextEmpty`).
- [ ] **Step 4: Write failing repository tests** in `repositories/todos.rs`:
  - `tags_and_recurrence_round_trip`: create with tags `["home","work"]`, `Monthly`, a due date → read back equal.
  - `tag_filter_is_owner_isolated`: owners A and B each have a `#work` todo; `list_all_user(A, tag=Some("work"))` returns only A's; `count_all_user` agrees.
  - `list_tags_counts_active_only`: two active `#work`, one completed `#work`, one active `#home` → `[("home",1),("work",2)]`.
  - `db_check_refuses_recurrence_without_due` (direct insert bypassing validation errors).
- [ ] **Step 5: Run** `cargo test -p matrix-bot domain::todos:: repositories::todos::` — expect FAIL.
- [ ] **Step 6: Implement** the domain and repository items in Interfaces; update every `NewTodo`/`TodoItem` literal (`tags: vec![], recurrence: RecurrencePattern::None`, `next_todo_id: None`); update `TodoService::list`/`count` to take and pass `tag: Option<&Tag>` and fix their callers with `None`.
- [ ] **Step 7: Regenerate the query cache** against a migrated scratch DB: `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets`.
- [ ] **Step 8: Run** `cargo test -p matrix-bot domain::todos:: repositories::todos:: db::` (includes schema catalog / `OWNED_TABLES` checks) and `cargo check -p matrix-bot --all-targets` — expect PASS, warning-free.
- [ ] **Step 9: Commit** `feat(todos): add tags, recurrence and steps schema with domain types`.

---

### Task 3: Locked completion and successor spawning

**Files:**
- Create: `backend/src/domain/todos/recurrence.rs`, `backend/src/application/todos/completion.rs`
- Modify: `backend/src/application/todos/mod.rs`, `backend/src/application/reminders/mod.rs` (or the file holding `find_pending_for_source`), `backend/src/infrastructure/postgres/repositories/reminders.rs`
- Test: unit tests in `recurrence.rs`; service tests in `completion.rs`

**Interfaces:**
- Consumes: Task 2 repository (`lock_owned`, `set_next_todo_id`, `create`), `repositories::todo_steps::copy_unchecked` (defined here, extended in Task 5), `repositories::settings::get(..).tz()`.
- Produces:
  - `pub fn successor(todo: &TodoItem, now: DateTime<Utc>, tz: Tz) -> Option<NewTodo>` in `domain/todos/recurrence.rs` (re-exported from `domain::todos`). Same `room_id`, `sender_name`, text, priority, tags, recurrence, `remind_before_minutes`, `recurrence_anchor`; `remind_room_id: None`; `due_at = next_occurrence_in_tz(anchor, recurrence, max(now, todo.due_at), tz)` where `anchor = recurrence_anchor.unwrap_or(due_at)` — stepping from the anchor keeps the series' day-of-month (Jan 31 → Feb 28 → Mar 31) and the `max` guarantees the successor is after the current due date even when completed early; `None` when recurrence is `None` or `due_at` is `None`.
  - `pub struct CompletionOutcome { pub todo: TodoItem, pub next: Option<TodoItem> }`.
  - `TodoService::set_completed(&self, owner, id, completed: bool) -> Result<CompletionOutcome, AppError>` (signature change; update callers in chat, AI tools, tests).
  - `TodoService::update` keeps `-> Result<(TodoItem, SyncOutcome), AppError>`, but locks the row, applies the patch, checks `validate_merged`, and when the patch transitions `completed` false → true runs the same spawn path.
  - `repositories::todo_steps::copy_unchecked(conn, from_todo: i64, to_todo: i64, owner: &AccountId) -> Result<(), sqlx::Error>` (create `todo_steps.rs` here with just this function).
  - `ReminderService::last_room_for_source(&self, source: ReminderSource, id: i64) -> Result<Option<String>, AppError>` — most recent reminder of any status for that source (`ORDER BY id DESC LIMIT 1`).

**Algorithm (in `completion.rs`, one transaction, private `async fn complete_locked(tx, owner, current: TodoItem) -> Result<CompletionOutcome, AppError>` shared by both entry points):**
1. caller has already `lock_owned` the row and found `!current.completed`;
2. `set_completed(tx, id, true)`;
3. if `current.next_todo_id.is_none()` and `successor(&current, Utc::now(), tz)` is `Some(new)`: `create(tx, owner, &new)`, `copy_unchecked(tx, current.id, created.id, owner)`, `set_next_todo_id(tx, current.id, created.id)`, re-read the completed row so `next_todo_id` is current;
4. publish `TodoUpdated(completed)` then `TodoCreated(successor)`.
After commit: `cancel_due_reminder(Todo, original.id)`; if a successor exists, `sync_reminder_for_todo(&successor, last_room_for_source(Todo, original.id).as_deref())` — look the room up **before** cancelling.

- [ ] **Step 1: Write failing domain tests** in `recurrence.rs` (UTC unless stated):
  - `non_recurring_has_no_successor`.
  - `monthly_successor_keeps_fields_and_advances_due`: due 2026-01-01 09:00, now 2026-01-03 → due 2026-02-01 09:00; tags, priority, lead, text, room carried over.
  - `missed_periods_are_skipped`: daily, due 2026-01-01, now 2026-01-05 12:00 → 2026-01-06 (same time as original).
  - `month_end_keeps_anchor_day`: anchor and due 2026-01-31 09:00, now 2026-02-01 → successor due 2026-02-28 09:00 with `recurrence_anchor` still 2026-01-31; from that successor, now 2026-03-01 → due 2026-03-31 09:00.
  - `early_completion_moves_past_current_due`: daily, due 2026-01-05 09:00, now 2026-01-03 → 2026-01-06 09:00.
  - `dst_keeps_local_wall_time`: `Europe/Berlin`, weekly, due 2026-03-23 09:00 local, now just after → 2026-03-30 09:00 local (UTC offset changes).
- [ ] **Step 2: Write failing service tests** in `completion.rs` (use `setup` like `application/todos/mod.rs` tests; seed `user_settings` timezone where needed):
  - `completing_recurring_spawns_one_successor_with_unchecked_steps`: monthly todo with 2 steps (one done) → outcome `next` has due +1 month, both steps `done == false`, original `next_todo_id == Some(next.id)`.
  - `concurrent_completions_spawn_once`: `tokio::join!` two `set_completed(true)` → exactly 2 todos exist for the owner.
  - `done_undo_done_spawns_no_second_successor`.
  - `failed_publish_rolls_back_completion_and_spawn`: `outbox::fail_every_publish` → error; original still incomplete; owner has 1 todo.
  - `events_are_updated_then_created`: `event_types_for` == `["TodoCreated","TodoUpdated","TodoCreated"]` (initial create, completion, successor).
  - `successor_reminder_uses_original_reminder_room`: todo in `web` with lead + `remind_room_id "!r:x"`; complete → successor's pending reminder room is `"!r:x"`; original's reminder cancelled.
  - `patch_completing_a_recurring_todo_spawns_a_successor` (Review Focus 1): `update(.., TodoPatch { completed: Some(true), .. })`.
  - `clearing_due_on_recurring_todo_is_a_validation_error` (Review Focus 2): `update(.., due_at: Some(None))` → `AppError::Validation` mentioning "due date"; row unchanged.
  - `recurrence_without_due_is_refused_on_patch` (Review Focus 3).
- [ ] **Step 3: Run** `cargo test -p matrix-bot domain::todos::recurrence application::todos::` — expect FAIL.
- [ ] **Step 4: Implement** the Produces items and algorithm; move `update`'s write to: begin → `lock_owned` (missing → `NotFound("Todo item not found or access denied")`) → merge patch in memory → `validate_merged` → if completing, `complete_locked` after writing the other patch fields, else `repositories::todos::update` + publish → commit → reminder sync as today.
- [ ] **Step 5: Update callers** of `set_completed` (`interface/chat/todos/mutate.rs::handle_done`, reaction handlers found with `rg "set_completed" backend/src`, `llm/tools/todo.rs`) to use `outcome.todo`; `handle_done` appends `"<br/>↻ Next: <b>#{id}</b>, due <b>{date}</b>"` (date via `format_datetime(.., tz)`, escaped) when `outcome.next` is `Some`.
- [ ] **Step 6: Run** `cargo test -p matrix-bot todos` and `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` — expect PASS.
- [ ] **Step 7: Commit** `feat(todos): spawn the next occurrence when a recurring todo completes`.

---

### Task 4: Tag use cases in the service

**Files:**
- Modify: `backend/src/application/todos/mod.rs`
- Test: service tests there

**Interfaces:**
- Consumes: Task 2 repository `list_tags`, tag-filtered list/count.
- Produces:
  - `TodoService::list(&self, owner, room_id: Option<&str>, completed: Option<bool>, tag: Option<&Tag>, limit, offset)` and `count(.., tag: Option<&Tag>)` (already threaded in Task 2 — this task adds tests and the tag-set helper).
  - `TodoService::list_tags(&self, owner) -> Result<Vec<(Tag, i64)>, AppError>`.
  - `pub fn apply_tag_edits(current: &[Tag], add: &[Tag], remove: &[Tag]) -> Result<Vec<Tag>, TodoError>` in `domain/todos/mod.rs` — union then difference, sorted, ≤ 10.

- [ ] **Step 1: Write failing tests:** domain `apply_tag_edits_adds_and_removes` (`[home]` + `[work]` − `[home]` → `[work]`), `apply_tag_edits_caps_at_ten`; service `update_replaces_tag_set_and_publishes_updated`, `list_tags_is_owner_scoped`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot todos` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — expect PASS.
- [ ] **Step 5: Commit** `feat(todos): tag filtering and tag listing in TodoService`.

---

### Task 5: Checklist steps (repository + service)

**Files:**
- Create: `backend/src/application/todos/steps.rs`
- Modify: `backend/src/infrastructure/postgres/repositories/todo_steps.rs` (from Task 3), `repositories/mod.rs`, `repositories/todos.rs` (step counts on list rows)
- Test: repository and service tests in those files

**Interfaces:**
- Consumes: `lock_owned` (Task 2), `TodoStep`, `TodoWithSteps`, `validate_step_text`, `MAX_STEPS` (Task 2).
- Produces:
  - Repository: `list(conn, todo_id) -> Vec<TodoStep>` (by position); `insert_last(conn, todo_id, owner, text) -> TodoStep`; `count(conn, todo_id) -> i64`; `update(conn, todo_id, step_id, text: Option<&str>, done: Option<bool>) -> Option<TodoStep>`; `delete(conn, todo_id, step_id) -> bool`; `set_order(conn, todo_id, ids: &[i64])`; `renumber(conn, todo_id)`; `progress_for(conn, todo_ids: &[i64]) -> Vec<(i64, i64, i64)>` (todo_id, done, total).
  - Renumbering keeps `UNIQUE (todo_id, position)` valid at every statement: first `UPDATE todo_steps SET position = -position WHERE todo_id = $1`, then `UPDATE todo_steps s SET position = o.rn FROM (SELECT id, row_number() OVER (ORDER BY <key>) AS rn FROM todo_steps WHERE todo_id = $1) o WHERE s.id = o.id`, where `<key>` is `abs(position)` for `renumber` and `array_position($2::bigint[], id)` for `set_order`.
  - Service (`impl TodoService` in `application/todos/steps.rs`): `get_with_steps(owner, id) -> Result<TodoWithSteps, AppError>`; `add_step(owner, todo_id, text: &str) -> Result<TodoWithSteps, AppError>`; `update_step(owner, todo_id, step_id, text: Option<&str>, done: Option<bool>)`; `remove_step(owner, todo_id, step_id)`; `reorder_steps(owner, todo_id, ids: &[i64])` — all return `TodoWithSteps`; each locks the todo, writes, publishes `TodoUpdated(todo)`, commits. `step_progress(owner, ids: &[i64]) -> Result<HashMap<i64, (i64, i64)>, AppError>` for list views.
  - Step by display position (chat/AI): `TodoWithSteps::step_at(&self, position: usize) -> Option<&TodoStep>` (1-based).

- [ ] **Step 1: Write failing repository tests:** `positions_compact_after_delete` (1,2,3 delete 2 → 1,2), `set_order_reorders`, `steps_cascade_with_todo`, `copy_unchecked_resets_done_and_keeps_order`.
- [ ] **Step 2: Write failing service tests:** `add_step_refuses_the_51st` (`TodoError::TooManySteps`), `step_of_another_owners_todo_is_not_found`, `step_id_from_a_different_todo_is_not_found`, `reorder_requires_exact_id_set` (`StepOrderMismatch`), `each_step_mutation_publishes_todo_updated`.
- [ ] **Step 3: Run** `cargo test -p matrix-bot todo_steps application::todos::steps` — expect FAIL.
- [ ] **Step 4: Implement**, then `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets`.
- [ ] **Step 5: Run** — expect PASS.
- [ ] **Step 6: Commit** `feat(todos): checklist steps with compact ordering`.

---

### Task 6: REST endpoints

**Files:**
- Create: `backend/src/interface/http/todo_steps.rs`
- Modify: `backend/src/interface/http/todos.rs`, `interface/http/mod.rs` (module), `routes.rs:127-131`, `openapi.rs` (paths + schemas), `backend/openapi.json`, `frontend/src/types/api.ts` (generated)
- Test: HTTP tests in both files (follow the existing `#[cfg(test)]` style in `todos.rs:300+`)

**Interfaces:**
- Consumes: Tasks 2–5 service methods.
- Produces (wire):
  - `TodoItem` DTO gains `tags: Vec<String>`, `recurrence: String`, `next_todo_id: Option<i64>`, `steps_done: i64`, `steps_total: i64` (list handler fills counts from `step_progress`; single-item responses from their steps).
  - `TodoQuery` gains `tag: Option<String>` (parsed with `Tag::parse`; invalid → 400).
  - `CreateTodoRequest` / `UpdateTodoRequest` gain `tags: Option<Vec<String>>` (strict `parse_tags` → 400 on error) and `recurrence: Option<String>` (`none|daily|weekly|monthly|yearly`, anything else → 400).
  - `GET /api/v1/todos/{id}` → `TodoDetail { item: TodoItem, steps: Vec<TodoStepDto> }`; `TodoStepDto { id, position, text, done }`.
  - `GET /api/v1/todos/tags` → `TodoTagsResponse { tags: Vec<TodoTagCount { tag, count }> }`. Register before `/todos/{id}` so `tags` is not parsed as an id.
  - `todo_steps.rs`: `POST /api/v1/todos/{id}/steps` (`CreateStepRequest { text }`), `PATCH /api/v1/todos/{id}/steps/{step_id}` (`UpdateStepRequest { text: Option<String>, done: Option<bool> }`), `DELETE /api/v1/todos/{id}/steps/{step_id}`, `PUT /api/v1/todos/{id}/steps/order` (`ReorderStepsRequest { step_ids: Vec<i64> }`, max 50) — each returns `TodoDetail` except DELETE (also `TodoDetail`, so the client re-renders without a refetch).
  - Text caps: todo text 500, step text 200 (service enforces; handler returns its 400).

- [ ] **Step 1: Write failing HTTP tests:** `create_with_tags_and_recurrence_round_trips`, `invalid_tag_is_400`, `unknown_recurrence_is_400`, `list_filters_by_tag`, `get_todo_returns_steps`, `tags_endpoint_lists_counts`, `step_endpoints_404_for_other_owner`, `step_text_over_200_is_400`, `patch_completed_recurring_sets_next_todo_id`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot interface::http::todo` — expect FAIL.
- [ ] **Step 3: Implement** handlers, routes and `openapi.rs` registration.
- [ ] **Step 4: Regenerate contracts:** `cargo run --bin generate-openapi`, then `npm --prefix frontend run generate-api`.
- [ ] **Step 5: Run** `cargo test -p matrix-bot interface::http::` and `npm --prefix frontend run build` — expect PASS.
- [ ] **Step 6: Commit** `feat(api): todo tags, recurrence and step endpoints`.

---

### Task 7: Chat commands

**Files:**
- Create: `backend/src/interface/chat/todos/tags.rs`, `backend/src/interface/chat/todos/steps.rs`
- Modify: `handler.rs` (dispatch), `mutate.rs` (`handle_add`), `view.rs` (`format_todo_list`, `handle_list_*`), `mod.rs` (`commands()` help)
- Test: unit tests in `tags.rs`/`steps.rs`; existing chat tests

**Interfaces:**
- Consumes: Tasks 3–5 service methods.
- Produces:
  - `tags.rs`: `pub fn extract_tags(text: &str) -> (String, Vec<Tag>)` — whitespace-split tokens starting with `#` that `Tag::parse` accepts are removed and returned (deduped, sorted); remaining tokens re-joined with single spaces. `pub fn parse_recurrence_word(s: &str) -> Option<RecurrencePattern>` — `day|daily`, `week|weekly`, `month|monthly`, `year|yearly`, `none`. `pub fn extract_every(text: &str) -> (String, Option<RecurrencePattern>)` — removes a trailing-or-inline `every <word>`. `handle_tag`, `handle_tags`, `handle_repeat`.
  - `steps.rs`: `handle_show`, `handle_step` (`add <text>`, `done <n>`, `undo <n>`, `rm <n>`).
  - `handle_add`: after priority parsing, apply `extract_every`, then a `due <duration>` suffix (`parse_reminder_args`; split on the last `" due "`), then `extract_tags`. Recurrence without a due → reply `⚠️ A repeating task needs a due date, e.g. <code>!todo add Pay rent every month due 3d</code>`.
  - Dispatch order in `handler.rs`: `tags` before `tag `, and `show`, `step`, `repeat` added; `list`/`room` accept a trailing `#tag` (page number still parsed).
  - List line format: existing line + ` <code>#a #b</code>` + ` ↻ monthly` + ` · 3/5` (only when present); counts come from `TodoService::step_progress` (Task 5), fetched once per page.

- [ ] **Step 1: Write failing unit tests:**
  - `extracts_tags_and_keeps_remaining_text` (Review Focus 4): `"Fix bug #123 in C# code #Work"` → (`"Fix bug in C# code"`, `["123","work"]`) — `C#` is not a whole `#token`, so it stays.
  - `extract_every_parses_inline`: `"Pay rent every month"` → (`"Pay rent"`, `Monthly`); `"Every Day stretch"` → (`"stretch"`, `Daily`); `"everything ok"` → unchanged, `None`.
  - `tag_edit_args_parse`: `"+work -home"` → add `[work]`, remove `[home]`; bare `work` counts as add.
  - `step_args_parse`: `"done 2"` → `Done(2)`; `"rm x"` → usage error.
- [ ] **Step 2: Run** `cargo test -p matrix-bot interface::chat::todos::` — expect FAIL.
- [ ] **Step 3: Implement**; keep `mutate.rs` ≤ 500 production lines (`wc -l` minus its test module).
- [ ] **Step 4: Update help** in `mod.rs::commands()` with the spec §6.1 usages.
- [ ] **Step 5: Run** `cargo test -p matrix-bot interface::chat::` — expect PASS.
- [ ] **Step 6: Commit** `feat(chat): todo tags, repeat and checklist commands`.

---

### Task 8: AI tools and MCP

**Files:**
- Create: `backend/src/llm/tools/todo_steps.rs`
- Modify: `backend/src/llm/tools/todo.rs`, `mod.rs` (module + tests at lines 55-70), `definitions.rs:10`, `executor.rs:72`
- Test: `llm/tools/mod.rs` tests

**Interfaces:**
- Consumes: Tasks 3–5.
- Produces:
  - `add_todo` / `update_todo` schemas gain `tags: { type: array, items: string, maxItems: 10 }` and `recurrence: { type: string, enum: [none, daily, weekly, monthly, yearly] }`; parsed with strict `parse_tags`; a bad value returns `Err("…")` to the model.
  - `get_my_tasks` gains `tag: string`.
  - `complete_todo` returns `{"todo": …, "next": … | null}`.
  - `todo_steps.rs`: `tool_definitions()` with `add_todo_step { todo_id: integer, text: string }` and `set_todo_step_done { todo_id: integer, position: integer (1-based), done: boolean }`; `execute_tool(services, owner_id, name, args) -> Option<Result<String, String>>`; aggregated in `definitions.rs`, chained in `executor.rs` after `todo::execute_tool`.
  - No argument named `owner`/`owner_id` is read; owner always comes from the caller.

- [ ] **Step 1: Write failing tests:** `tool_registry_lists_step_tools`, `add_todo_with_tags_and_recurrence`, `add_todo_with_invalid_recurrence_errors`, `complete_recurring_todo_reports_next`, `set_step_done_by_position`, `owner_in_args_is_ignored` (args carry another account's id; todo is created for the caller).
- [ ] **Step 2: Run** `cargo test -p matrix-bot llm::tools::` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p matrix-bot llm::tools:: interface::mcp::` — expect PASS.
- [ ] **Step 5: Commit** `feat(llm): todo tag, recurrence and step tools`.

---

### Task 9: Web UI

**Files:**
- Create: `frontend/src/modules/todo/components/TagInput.tsx`, `frontend/src/modules/todo/components/StepList.tsx`
- Modify: `frontend/src/modules/todo/types.ts`, `index.tsx`, `components/CreateTodoCard.tsx`, `components/TodoRow.tsx`, `frontend/e2e/fake-api.ts`, `frontend/e2e/todos.spec.ts`

**Interfaces:**
- Consumes: generated `components['schemas']` types from Task 6 (`TodoItem`, `TodoDetail`, `TodoStepDto`, `TodoTagsResponse`); all calls through `core/apiClient.ts`.
- Produces:
  - `TodoPatch` gains `tags?: string[]`, `recurrence?: string`.
  - `TagInput({ value: string[], onChange })` — chip input; `Enter`/`,` commits a chip, `Backspace` on empty removes the last; labels visible; Geist Mono chips.
  - `StepList({ todoId, steps, onChange })` — add (single-line form, `Enter` submits), toggle checkbox, remove, move up/down buttons (calls `PUT …/steps/order`).
  - `CreateTodoCard`: `TagInput`; repeat `<select>` (`none/daily/weekly/monthly/yearly`) disabled until a due date is set.
  - `TodoRow`: tag chips, `↻ {recurrence}` badge, `{steps_done}/{steps_total}`; expand toggle loads `GET /todos/{id}` and renders `StepList`.
  - `index.tsx`: tag filter `<select>` fed by `GET /todos/tags`, passed as `?tag=`.
  - `fake-api.ts`: `FakeTodo` gains `tags`, `recurrence`, `next_todo_id`, `steps`; routes for `?tag=`, `/todos/tags`, `/todos/{id}`, the four step routes; PATCH `completed: true` on a recurring todo pushes a successor (due +1 period) and sets `next_todo_id`.

- [ ] **Step 1: Write failing Playwright tests** in `todos.spec.ts`: `creates a tagged todo and filters by tag`, `repeat selector requires a due date`, `completing a recurring todo shows its successor`, `checks off and reorders steps`.
- [ ] **Step 2: Run** `npm --prefix frontend run test:e2e -- todos.spec.ts` — expect FAIL.
- [ ] **Step 3: Implement** the components and fake routes, following Design.md (monochrome, `transition-colors duration-150`, visible focus, sentence-case headings, module root `<div className="space-y-6">`).
- [ ] **Step 4: Run** `npm --prefix frontend run build` and `npm --prefix frontend run test:e2e` — expect PASS (whole suite, to catch fake-API regressions such as `todo-due-reminder.spec.ts`).
- [ ] **Step 5: Commit** `feat(web): todo tags, repeat and checklist UI`.

---

### Task 10: Docs, handoff and full verification

**Files:**
- Modify: `docs/superpowers/HANDOFF.md`, `docs/database.md` (shared tag rule, `todo_steps`), `docs/llm.md` (new tools, if it lists tools)

- [ ] **Step 1: Update docs.** HANDOFF: status row for this feature, the four rulings from this plan in `Ruling: <decision> — <why> — <cost if wrong>` form, next actions (none / follow-ups from spec non-goals).
- [ ] **Step 2: Run every gate** (AGENTS.md §8):
  ```bash
  cargo fmt --all -- --check
  cargo check -p matrix-bot --all-targets
  cargo clippy -p matrix-bot --all-targets -- -D warnings
  cargo test -p matrix-bot --all-targets
  cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets
  cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
  npm --prefix frontend run build
  npm --prefix frontend run test:e2e
  ```
  Expected: all exit 0; clippy and check warning-free.
- [ ] **Step 3: Check file sizes:** no file in `backend/src/` over 500 production lines; `git diff --check` clean.
- [ ] **Step 4: Commit** `docs(superpowers): record todo tags, recurrence and steps in handoff`.
