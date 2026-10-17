# Spec: Todo tags, recurrence and checklist steps

**Status:** approved design, 2026-10-18. Next: implementation plan in `docs/superpowers/plans/`.

## 1. Problem

A todo today is text, priority, an optional due date and an optional reminder lead (`domain/todos/mod.rs::TodoItem`). That covers one-off tasks but not three everyday needs:

1. **Grouping.** There is no way to say "show me my work tasks". Notes have tags (`notes.tags TEXT[]`); todos do not.
2. **Repeating tasks.** "Pay rent" every month has to be re-entered by hand. Reminders recur (`RecurrencePattern`), todos do not.
3. **Multi-step tasks.** "Prepare tax return" has steps, and today they either get crammed into the text or split into unrelated todos.

## 2. Goals and non-goals

**Goals**

- Free-form **tags** on todos, filterable from chat, REST, AI tools/MCP and the web UI.
- **Recurrence** scheduled from the due date: completing a recurring todo spawns its successor.
- A simple ordered **checklist** of steps per todo.
- One shared tag value object for notes and todos, accepting Unicode (e.g. `#工作`).

**Non-goals (YAGNI)**

Projects/lists; tag rename, colours or metadata; custom intervals ("every 2 weeks", "weekdays"); recurrence measured from completion; steps with their own due date, priority, reminder or nesting; drag-and-drop reordering.

## 3. Decisions

| Decision | Choice | Why |
|---|---|---|
| Tag storage | `todos.tags TEXT[]` + GIN index | Same as `notes`; one indexed query per filter; no N:M merge logic |
| Tag rule | Shared `domain/tags::Tag`, Unicode letters/digits, `_`, `-`, ≤ 50 chars, lowercased | Second real use justifies extraction; widening is backward compatible for every existing note tag |
| Recurrence anchor | From the due date, via `next_occurrence_in_tz`, strictly after now | Fixed schedule; late completion never drifts it; identical to reminder recurrence |
| On completion | Spawn a new todo; the completed one stays as history | `!todo clear` and history keep working; ids are not reused |
| Spawn idempotency | `next_todo_id` on the completed row; spawn only while it is `NULL`, under `FOR UPDATE` | Double completion and done→undo→done never produce a second successor |
| Steps | Child table `todo_steps`, text + done, ≤ 50 per todo | Compile-checked SQL per step; cheap single-step toggles |
| Events | No new variants: step mutations publish `TodoUpdated` | SSE and views already refresh on it |

## 4. Data model

### 4.1 Migration `backend/migrations/0012_todo_tags_recurrence_steps.sql`

Plain DDL with defaults; no data rewrite. Existing todos become untagged, non-recurring and stepless.

```sql
ALTER TABLE todos
    ADD COLUMN tags         TEXT[]  NOT NULL DEFAULT '{}',
    ADD COLUMN recurrence   TEXT    NOT NULL DEFAULT 'none'
        CHECK (recurrence IN ('none', 'daily', 'weekly', 'monthly', 'yearly')),
    ADD COLUMN recurrence_anchor TIMESTAMPTZ,
    ADD COLUMN next_todo_id BIGINT  REFERENCES todos(id) ON DELETE SET NULL,
    ADD CONSTRAINT todos_recurrence_needs_due
        CHECK (recurrence = 'none' OR due_at IS NOT NULL);

CREATE INDEX idx_todos_tags ON todos USING GIN (tags);

CREATE TABLE todo_steps (
    id       BIGSERIAL PRIMARY KEY,
    todo_id  BIGINT  NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    owner_id UUID    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    text     TEXT    NOT NULL,
    done     BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (todo_id, position)
);
CREATE INDEX idx_todo_steps_owner ON todo_steps(owner_id);
```

- `todo_steps` is added to `OWNED_TABLES` (`domain/accounts/mod.rs`) so an account merge moves it.
- `.sqlx/` is regenerated and committed.

### 4.2 Domain

**`domain/tags/mod.rs` (new, shared)**

- `Tag` newtype with a private field; `Tag::parse(&str) -> Result<Tag, TagError>` trims, strips one leading `#`, lowercases (`str::to_lowercase`), and accepts characters where `char::is_alphanumeric()` or `_`/`-`; 1–50 characters.
- `parse_tags(&[String]) -> Result<Vec<Tag>, TagError>`: strict — parse each, dedupe, sort; the first invalid tag is an error. Todos use this, so a bad tag is reported rather than silently lost.
- `normalize_tags_lenient(&[String]) -> Vec<Tag>`: today's notes behaviour — invalid tags are dropped, the rest deduped and sorted.
- `domain/knowledge/note.rs` replaces its `normalize_tag`/`normalize_tags` with these (`Tag::parse` mapped into `KnowledgeError::InvalidTag`; the lenient helper where it drops today). Behaviour for notes changes only in also accepting non-ASCII letters and digits.

**`domain/todos`**

- `TodoItem` gains `tags: Vec<Tag>`, `recurrence: RecurrencePattern` (reused from `domain/reminders`), `recurrence_anchor`, `next_todo_id: Option<i64>`.
- `NewTodo` gains `tags`, `recurrence`; `TodoPatch` gains `tags: Option<Vec<Tag>>` (whole-set replace), `recurrence: Option<RecurrencePattern>`.
- Validation (added to the existing `validate` functions):
  - at most `MAX_TAGS = 10` tags;
  - a recurrence other than `None` requires a due date — on create, and on patch against the merged result (the service checks the patch applied to the locked row, so clearing the due date of a recurring todo is refused).
- `TodoStep { id, todo_id, position, text, done }`; `MAX_STEPS = 50`, `MAX_STEP_TEXT_LEN = 200`; `validate_step_text`.
- `TodoWithSteps { todo: TodoItem, steps: Vec<TodoStep> }` read model for single-todo reads; list rows carry `steps_done` / `steps_total`.
- `recurrence_anchor: Option<DateTime<Utc>>` on `TodoItem`: the due date the series started from. Set to `due_at` when recurrence is turned on or the due date of a recurring todo is changed; cleared when recurrence is set to `none`; inherited by successors. Stepping from the anchor rather than from each successor's due date keeps month-end series on their day (Jan 31 → Feb 28 → Mar 31 instead of drifting to the 28th).
- Pure `successor(todo: &TodoItem, now, tz) -> Option<NewTodo>`: `None` when `recurrence == None` or no due date; otherwise the same text, priority, tags, recurrence, anchor and `remind_before_minutes`, with `due_at = next_occurrence_in_tz(anchor, recurrence, max(now, due_at), tz)`. Missed periods are skipped, and an early completion still moves to the period after the current due date.
- A todo-domain error enum (`thiserror`) mapped to `AppError::Validation` in `application/todos`.

## 5. Service behaviour (`application/todos`)

### 5.1 Completing a todo

`set_completed(owner, id, completed)` becomes one transaction:

1. `SELECT … FOR UPDATE` the todo scoped to `owner_id` (replaces today's unlocked read-then-write). Missing → `NotFound`.
2. Unchanged state → return as is.
3. Write `completed`.
4. If `completed && recurrence != None && next_todo_id IS NULL`:
   - read the owner's timezone from `repositories::settings` (as `ReminderService::complete_delivery` does);
   - `successor(...)` → insert the new todo (same `room_id`, `sender_name`);
   - copy steps with `done = false`, same order;
   - set `next_todo_id` on the completed row.
5. Publish `TodoUpdated(original)` and, if spawned, `TodoCreated(successor)` via `outbox::publish` in the same transaction.
6. Commit. Then, as today, reminder side effects run after commit: cancel the original's reminder; sync the successor's (`sync_reminder_for_todo`) with the room of the original's most recent reminder of any status (new `ReminderService::last_room_for_source`) as the override, so the successor reminds where the original did even if that reminder already fired. With no such room the existing fallback applies (the todo's room, refused for `web`).

The return type gains the successor: `CompletionOutcome { todo: TodoItem, next: Option<TodoItem> }`.

**Guarantees**

| Scenario | Result |
|---|---|
| Two concurrent completions (two channels, retried AI call) | Row lock serialises; the second sees "already completed"; one successor |
| Done → undo → done | `next_todo_id` already set; no second successor; undo does not delete the successor |
| Outbox insert fails mid-transaction | Neither completion nor spawn persists |
| Reminder sync fails after commit | Error surfaced as today; the todo state is committed (unchanged existing semantics) |
| End a series | Set recurrence to `none`, then complete |

### 5.2 Tags

- Create/update accept a full tag set; the chat `+tag -tag` form is resolved to a full set in the interface layer, then calls `update`.
- `list` / `count` gain `tag: Option<&Tag>` (`tags @> ARRAY[$1]`), combined with the existing room/completed filters and owner scope.
- `list_tags(owner) -> Vec<(Tag, i64)>` over active todos (`unnest(tags)` grouped), owner-scoped.

### 5.3 Steps

`add_step`, `update_step` (text and/or done), `remove_step`, `reorder_steps(ids_in_order)`, `get_with_steps`.

Each mutation: one transaction that locks the parent todo `FOR UPDATE` scoped to the owner, checks the step belongs to that todo, writes, renumbers positions compactly (1..n) where needed, and publishes `TodoUpdated`. `add_step` refuses beyond `MAX_STEPS`. `reorder_steps` requires exactly the todo's current step ids. Completing a parent does not require its steps to be done.

Positions are renumbered by writing into a temporary negative range then the final range within the transaction, so `UNIQUE (todo_id, position)` holds at every statement.

### 5.4 Clearing and deleting

Unchanged. Steps cascade; a successor's back-reference is `SET NULL` when its predecessor is cleared.

## 6. Interfaces

All interfaces call the `TodoService` methods above; none re-derives a rule.

### 6.1 Chat (`interface/chat/todos/`)

Inputs capped at 500 characters; replies via `text_html` with every user value through `html_escape()`.

| Command | Behaviour |
|---|---|
| `!todo add [high] Pay rent #home every month due 1st 9am` | Inline `#tags` are extracted from the text; `every day\|week\|month\|year` sets recurrence; recurrence without a due date replies with a clear error |
| `!todo list #work` / `!todo room #work` | Tag filter on the existing views |
| `!todo tags` | Tags with active-todo counts |
| `!todo tag <id> +work -home` | Add/remove tags |
| `!todo repeat <id> <daily\|weekly\|monthly\|yearly\|none>` | Set or end recurrence |
| `!todo show <id>` | Todo with its checklist |
| `!todo step <id> add <text>` / `done <n>` / `undo <n>` / `rm <n>` | Steps addressed by 1-based position as displayed |

- List rows show tags, `↻ monthly` and `3/5` step progress.
- Completing a recurring todo replies with the successor: "Done. Next: #57, due 1 Nov".
- `mutate.rs` (485 lines) is split so no file exceeds 500 production lines: tag and repeat parsing → `tags.rs`, step commands → `steps.rs`.
- Help entries in `TodoModule::commands()` are updated.

### 6.2 REST (`interface/http/`)

- `POST /api/v1/todos`, `PATCH /api/v1/todos/{id}`: DTOs gain `tags: string[]`, `recurrence`.
- `GET /api/v1/todos?tag=…`: filter. List items gain `tags`, `recurrence`, `next_todo_id`, `steps_done`, `steps_total`.
- `GET /api/v1/todos/{id}`: includes `steps`.
- `GET /api/v1/todos/tags`: tags with counts.
- New `interface/http/todo_steps.rs` (keeps `todos.rs` under the cap):
  - `POST /api/v1/todos/{id}/steps`
  - `PATCH /api/v1/todos/{id}/steps/{step_id}`
  - `DELETE /api/v1/todos/{id}/steps/{step_id}`
  - `PUT /api/v1/todos/{id}/steps/order`
- Completion responses include the spawned successor (`next`).
- Another owner's todo or step → 404. Payload strings ≤ 500 chars (steps ≤ 200). No step-up routes.
- Additive contract change: regenerate `backend/openapi.json` and the frontend client (`npm --prefix frontend run generate-api`); update `contracts/` fixtures if any cover todos.

### 6.3 AI tools and MCP (`llm/tools/todo.rs`)

- `add_todo`, `update_todo`: add `tags` (array of strings) and `recurrence` (enum).
- `get_my_tasks`: add `tag` filter; results include tags, recurrence, step progress.
- `complete_todo`: result names the successor (id and due date) when one was spawned.
- New: `add_todo_step { todo_id, text }`, `set_todo_step_done { todo_id, position, done }`.
- Tool arguments are validated like every other input; the owner comes only from the authenticated context. MCP exposes the same registry.

### 6.4 Web (`frontend/src/modules/todo/`)

- `CreateTodoCard`: tag chip input; repeat selector enabled once a due date is set.
- `TodoRow`: tag chips (Geist Mono), `↻` badge, step progress; expands to an inline checklist with add, toggle, remove and keyboard reorder (up/down buttons; no drag-and-drop library).
- List header: tag filter fed by `GET /todos/tags`.
- Design.md rules apply (monochrome, visible labels and focus, `Enter` submits, `Escape` closes). Mock routes in `frontend/e2e/fake-api.ts`; Playwright coverage in `frontend/e2e/todo.spec.ts`.

## 7. Testing

| Level | Cases |
|---|---|
| Domain | `Tag::parse`: Unicode accepted, `#` stripped, case folded, empty/over-50/invalid chars rejected; every previously valid ASCII note tag still parses identically. `successor`: each pattern; monthly Jan 31 → Feb 28 → Mar 31 anchoring; DST in the owner's zone; missed periods skipped; `None` for non-recurring. Validation: recurrence without due date, > 10 tags, > 50 steps, step text > 200 |
| Repository | Tags round-trip; tag filter is owner-isolated; step positions compact after remove/reorder; steps cascade on todo delete |
| Service | Recurring completion spawns exactly one successor with unchecked steps and the next due date; concurrent completions → one successor; done → undo → done → no second successor; failed outbox publish rolls back completion and spawn; successor reminder synced, original cancelled; events `TodoUpdated` + `TodoCreated`; patch clearing due date on a recurring todo refused; step mutations publish `TodoUpdated` |
| Merge | `todo_steps` in `OWNED_TABLES`; schema catalog test passes |
| HTTP | New endpoints 404 across owners; payload caps; OpenAPI drift check |
| Chat / AI | Parsing of `#tags`, `every …`, `repeat`, `step`; malformed tool arguments rejected; a model-supplied owner is ignored |
| Frontend | Playwright: create with tags and filter; repeat badge and successor after completion; check off steps |

Verification gates: AGENTS.md §8 in full (fmt, check, test, clippy, sqlx check, OpenAPI drift, frontend build and e2e).

## 8. Rollout

- One forward-only, plain-DDL migration; no data rewrite, no rehearsal needed beyond the normal suite.
- No config or env changes.
- `docs/superpowers/HANDOFF.md` updated with status and rulings in the implementing commits; `docs/database.md` notes the shared tag rule if it documents tag storage.
