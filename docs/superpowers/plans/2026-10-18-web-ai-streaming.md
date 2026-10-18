# Web AI streaming, stop and live tool steps — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The web AI Assistant streams replies, shows each tool call as a live step, and can be stopped, while every channel keeps sharing one `run_turn`.

**Architecture:** `ProgressSink` gains defaulted methods (conversation, tool start/finish, retract). `run_turn` gains an optional cancellation token that it checks only at safe points. A new `POST /api/v1/ai/chat/stream` spawns the turn on a tracked task, feeds a bounded channel from a web sink, and answers `text/event-stream`. Closing that stream cancels the turn. On the frontend, a module-level live-turn store reads the stream, so a turn survives navigation, and the AI view renders from that store.

**Tech Stack:** Rust (axum 0.7 SSE, tokio, tokio-util `CancellationToken`/`TaskTracker`, sqlx/PostgreSQL 18, utoipa), React 19 + TanStack Query, Playwright.

**Spec:** [`docs/superpowers/specs/2026-10-18-web-ai-streaming.md`](../specs/2026-10-18-web-ai-streaming.md). Read it before any task.

## Global Constraints

- Work in a separate worktree. Never run the bot, never contact a real LLM, never touch `.env` or `data/` (AGENTS §9).
- `POST /api/v1/ai/chat` keeps its exact contract; `backend/openapi.json` changes only by additions.
- Matrix, Discord and Telegram progress output stays identical: their existing sink tests pass unmodified.
- Prompt cap 4000 (`MAX_PROMPT_LENGTH`), turn limit 180 s, keep-alive every 15 s, event channel capacity 64, max 2 concurrent streaming turns per account (429 above that).
- Cancellation is never checked inside `tool_round::run_calls`.
- Stopped marker text: `_(stopped)_`, appended after a blank line to any partial text; alone if there is no partial text.
- No production file in `backend/src/` over 500 lines (tests excluded). `llm/turn.rs` is near that, so put new turn logic in `llm/stream.rs` or a new `llm/cancel.rs`, not inline.
- Frontend: all HTTP goes through `core/apiClient.ts`; follow `Design.md`; every API-shape change is followed by `npm --prefix frontend run generate-api`.
- Each task ends with `cargo fmt --all -- --check`, `cargo clippy -p matrix-bot --all-targets -- -D warnings`, and its own tests green. Commits use Conventional Commits and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **The provider ignores the dropped request and keeps streaming.** Cancel must still return promptly: the stream helper stops reading on cancel, it doesn't wait for EOF. Covered by Task 4 `cancel_during_stream_returns_promptly`, which uses a fake LLM that never ends its stream.
2. **Two tabs, three turns.** The third concurrent stream gets 429, and permits come back when a turn ends any way it can (done, error, cancel, panic). Covered by Task 6 `turn_permit_is_released_when_the_turn_errors`.
3. **A Chinese or emoji delta splits a multi-byte character across SSE chunks.** The frontend parser must decode with a streaming `TextDecoder` (`{ stream: true }`), not per chunk. Covered by Task 7's e2e `multibyte text arriving across chunks renders intact`.
4. **The stream drops right after `started` (network blip, not Stop).** The UI must not hang in "Stopping…" forever: it refetches when the conversation-updated event arrives, or after the 180 s fallback. Covered by Task 8 e2e `a dropped stream falls back to stored history`.
5. **Shutdown while a turn is running.** The turn's token is a child of `ApiState.shutdown`, so it cancels, stores the stopped marker, and the tracker drains within the shutdown bound. Covered by Task 6 `shutdown_cancels_a_running_turn`.

---

### Task 1: Report conversation and every tool step through `ProgressSink`

**Files:**
- Modify: `backend/src/domain/llm/mod.rs` (add `ToolStepOutcome`)
- Modify: `backend/src/llm/progress.rs`
- Modify: `backend/src/llm/turn.rs` (calls only; tests in its `mod tests`)

**Interfaces:**
- Produces:
  - `domain::llm::ToolStepOutcome { Ok, Failed, NeedsConfirmation }`: `Copy, Debug, PartialEq, Eq, Serialize, Deserialize`, `#[serde(rename_all = "snake_case")]`, plus `fn as_str(self) -> &'static str` (`"ok" | "failed" | "needs_confirmation"`) and `fn parse(&str) -> Option<Self>`.
  - `impl From<&tool_round::ToolOutcome> for ToolStepOutcome`: `refused` → `NeedsConfirmation`, else `succeeded` → `Ok`, else `Failed`. Place it in `llm/tool_round.rs`.
  - On `ProgressSink`, all with default no-op bodies: `async fn report_conversation(&self, conversation_id: &str)`, `async fn report_tool_started(&self, call_id: &str, name: &str)`, `async fn report_tool_finished(&self, call_id: &str, name: &str, outcome: ToolStepOutcome)`, `async fn retract_segment(&self)` (the last is called in Task 2).

- [ ] **Step 1: Write failing tests** in `turn.rs` tests. Extend `RecordingSink` to note `conv:{id}`, `start:{call_id}:{name}`, `finish:{call_id}:{outcome.as_str()}`.
  - `a_turn_reports_its_conversation_first`: the first entry of `seen()` starts with `conv:`, and its id equals `result.conversation_id`.
  - `every_call_in_a_round_is_reported_started_and_finished`: a scripted round with two tool calls (`list_finance_transactions` with `call_1` and `list_todos` with `call_2`) gives `start:call_1…`, `start:call_2…` before any `finish:`, then `finish:call_1:ok` and `finish:call_2:ok`.
  - `a_refused_call_finishes_as_needs_confirmation`: build the context `with_untrusted_input()` and script `delete_todo`. Expect `finish:<id>:needs_confirmation`.
  - `a_failing_tool_finishes_as_failed`: script a tool with invalid arguments. Expect `finish:<id>:failed`.
  - Unit test in `domain/llm`: `as_str`/`parse` round-trip all three variants, and `parse("x") == None`.
- [ ] **Step 2: Run** `cargo test -p matrix-bot llm::turn::tests` and confirm the new tests FAIL (missing methods).
- [ ] **Step 3: Implement.** Add the enum and trait methods. In `run_turn`: call `report_conversation` right after the conversation is resolved. Keep the existing `report_tool_call(first)`, add `report_tool_started` for each call before `run_calls`, and call `report_tool_finished` per outcome inside the existing outcome loop.
- [ ] **Step 4: Run** `cargo test -p matrix-bot llm::` and `cargo test -p matrix-bot interface::channels::`. All pass, with Matrix/Discord/Telegram tests unmodified.
- [ ] **Step 5: Commit** `feat(llm): report conversation and every tool step to progress sinks`.

### Task 2: Retract streamed text the user must not keep

**Files:**
- Modify: `backend/src/llm/stream.rs`, `backend/src/llm/turn.rs`
- Test: `turn.rs` tests, `stream.rs` tests

**Interfaces:**
- Consumes: `ProgressSink::retract_segment` (Task 1).

- [ ] **Step 1: Write failing tests.** `RecordingSink` notes `retract`.
  - `a_reply_the_guard_rejects_is_retracted`: a streamed reply claiming success ("✅ Added the todo") with no tool call. `seen()` contains `flush` followed later by `retract`, and `result.message` is the corrected reply or `guard::UNRECORDED_NOTICE`.
  - `a_stream_that_fails_midway_retracts_then_sends_the_fallback`: a fake LLM that sends one delta and then closes with a malformed chunk on streaming requests, and answers normal JSON `text_reply("full answer")` otherwise. `seen()` contains `delta:…`, `retract`, then `delta:full answer`, `flush`.
- [ ] **Step 2: Run** them and confirm they FAIL.
- [ ] **Step 3: Implement.** In `run_turn`, call `retract_segment` right before pushing `CORRECTION_MESSAGE`, and again when the final guard replaces the content with `UNRECORDED_NOTICE` after text was streamed. In `execute_stream_or_complete`, if any delta reached the sink before the stream error: call `retract_segment`, run the fallback, then `push_delta(content)` and `flush` when the content is non-empty. If no delta was sent: only `push_delta` + `flush`.
- [ ] **Step 4: Run** `cargo test -p matrix-bot llm::` and `interface::channels::`. All pass.
- [ ] **Step 5: Commit** `feat(llm): retract streamed text that the guard or a fallback replaces`.

### Task 3: Store each tool row's outcome

**Files:**
- Create: `backend/migrations/0012_ai_message_tool_outcome.sql`: `ALTER TABLE ai_messages ADD COLUMN tool_outcome TEXT NULL CHECK (tool_outcome IN ('ok','failed','needs_confirmation'));` (LF line endings).
- Modify: `backend/src/infrastructure/postgres/repositories/llm.rs`, `backend/src/domain/llm/mod.rs` (`AiMessage.tool_outcome: Option<ToolStepOutcome>`), `backend/src/llm/models.rs` (DTO), `backend/src/llm/turn.rs` (the tool-row write), `.sqlx/`, `backend/openapi.json`, `frontend/src/types/api.ts`.

**Interfaces:**
- Produces: `repositories::llm::add_tool_message(pool: &PgPool, conversation_id: &str, call_id: &str, tool_name: &str, content: &str, outcome: ToolStepOutcome) -> Result<AiMessage, sqlx::Error>`. `add_message`/`insert_message` keep their signatures and write `NULL`. Every `SELECT` of `ai_messages` reads `tool_outcome`, and the row-to-domain conversion maps it with `ToolStepOutcome::parse`.
- DTO: `AiMessage.tool_outcome: Option<AiToolOutcome>`, where `AiToolOutcome` is a `ToSchema` enum, snake_case, with `From<ToolStepOutcome>`.

- [ ] **Step 1: Write failing tests.**
  - Repository: `tool_message_round_trips_its_outcome`. Store one row per variant, and `get_recent_messages` returns the same `tool_outcome`. A plain `add_message` row reads back `None`.
  - Turn: `tool_rows_are_stored_with_their_outcome`. After the two-call scripted turn from Task 1, the stored `tool` rows have `Some(ToolStepOutcome::Ok)`.
- [ ] **Step 2: Run** them and confirm they FAIL.
- [ ] **Step 3: Implement** the migration, repository function, domain field, DTO, and the `turn.rs` switch from `add_message(…"tool"…)` to `add_tool_message`.
- [ ] **Step 4: Regenerate and verify:** run `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` against a migrated scratch DB, then `cargo run --bin generate-openapi` and `npm --prefix frontend run generate-api`. `cargo test -p matrix-bot` passes, and `git diff backend/openapi.json` shows only the added field and enum.
- [ ] **Step 5: Commit** `feat(llm): store the outcome of each AI tool call`, including `.sqlx/`, `openapi.json` and `api.ts`.

### Task 4: Cancellable turns with a stored stopped marker

**Files:**
- Create: `backend/src/llm/cancel.rs` (stopped-marker composition, and the cancel-race helper if it doesn't fit in `stream.rs`)
- Modify: `backend/src/llm/context.rs`, `backend/src/llm/stream.rs`, `backend/src/llm/turn.rs`, `backend/src/llm/error.rs`, `backend/src/interface/http/ai.rs` (the `From<AiError>` arm), `backend/src/llm/test_support.rs`

**Interfaces:**
- Produces:
  - `LlmTurnContext.cancel: Option<CancellationToken>`, defaulting to `None` in `new`, plus `fn with_cancellation(self, token: CancellationToken) -> Self`.
  - `AiError::Cancelled`. Its `Display` is "The turn was stopped". In `From<AiError> for ApiError` it maps to status 499 with the message "Client closed request". This is only reachable on the streaming path, and it is never logged as an error.
  - `execute_stream_or_complete(…, cancel: Option<&CancellationToken>) -> Result<(String, Vec<ToolCall>), StreamError>`, where `enum StreamError { Failed(String), Cancelled { partial: String } }`. Update both call sites in `turn.rs`.
  - `llm::cancel::stopped_message(partial: &str) -> String`: `"_(stopped)_"` when `partial.trim()` is empty, otherwise `format!("{}\n\n_(stopped)_", partial.trim_end())`.
  - `test_support::hanging_stream_llm(first_delta: &str) -> String` (base URL): sends one streamed delta and then holds the connection open without ending.
- Safe points in `run_turn`: the LLM call itself (the token is raced inside the helper), the top of each loop iteration before building the request, and right before the final summary completion. On cancel: `append_assistant_message(owner, conv, &stopped_message(partial))`, then return `Err(AiError::Cancelled)`.

- [ ] **Step 1: Write failing tests.**
  - `stopped_message` unit tests: empty partial → `_(stopped)_`; `"Half"` → `"Half\n\n_(stopped)_"`.
  - `cancel_during_stream_returns_promptly`: use `hanging_stream_llm("Hel")` and cancel the token 200 ms after the turn starts. The turn returns `Err(AiError::Cancelled)` within 2 s, and the last stored message is assistant `"Hel\n\n_(stopped)_"`.
  - `cancel_during_a_tool_round_finishes_the_round_then_stops`: script a tool call whose execution the test can observe, and cancel while the round runs. The tool row is stored with its outcome, the fake LLM sees no further request, and the last stored message is `_(stopped)_`.
  - `a_turn_without_a_token_is_unchanged`: the existing turn tests pass untouched (no new assertion needed; this just runs them).
  - `ai.rs`: `ApiError::from(AiError::Cancelled).status.as_u16() == 499`.
- [ ] **Step 2: Run** them and confirm they FAIL.
- [ ] **Step 3: Implement** as described in Interfaces. Use `tokio::select!` on `token.cancelled()` against the next stream chunk inside the helper, keeping what has been accumulated so far.
- [ ] **Step 4: Run** `cargo test -p matrix-bot llm::` and `interface::`. All pass.
- [ ] **Step 5: Commit** `feat(llm): stop a turn at safe points and store a stopped marker`.

### Task 5: Stream event type and the web sink

**Files:**
- Create: `backend/src/interface/http/ai_stream/mod.rs` (handler goes here in Task 6), `ai_stream/events.rs`, `ai_stream/sink.rs`

**Interfaces:**
- Produces:
  - `AiStreamEvent`: `Serialize + ToSchema`, `#[serde(tag = "type", rename_all = "snake_case")]`, with variants exactly as in the spec's table: `Started { conversation_id }`, `Delta { text }`, `SegmentEnd`, `SegmentRetracted`, `ToolStarted { call_id, name }`, `ToolFinished { call_id, name, outcome: AiToolOutcome }`, `Done { conversation_id, message, model_used, tool_calls_executed }`, `Error { status: u16, message }`.
  - `WebStreamSink::new(tx: mpsc::Sender<AiStreamEvent>, cancel: CancellationToken)`, implementing `ProgressSink`:
    - `report_conversation` → `Started`; `push_delta` → `Delta`; `flush` → `SegmentEnd`; `retract_segment` → `SegmentRetracted`; tool methods → `ToolStarted`/`ToolFinished`.
    - `report_stage` and `report_tool_call` are ignored.
    - Sending awaits capacity. If the receiver is gone, it cancels the token. (Coalescing deltas is optional; never drop a non-delta event.)
  - `fn error_event(err: &AiError) -> AiStreamEvent`, using the same status and message as `ApiError::from`.

- [ ] **Step 1: Write failing tests.**
  - `events_serialize_with_snake_case_type`: `ToolFinished{…, outcome: NeedsConfirmation}` serializes to JSON with `"type":"tool_finished"` and `"outcome":"needs_confirmation"`; `SegmentEnd` serializes to `{"type":"segment_end"}`.
  - `sink_maps_each_progress_call_to_its_event`: drive every method and assert the events received, in order.
  - `a_dropped_receiver_cancels_the_token`: drop the receiver, call `push_delta`, and `cancel.is_cancelled()` is true.
  - `error_event_matches_api_error`: a `Provider` error gives status 503, `Timeout` gives 504.
- [ ] **Step 2: Run** them and confirm they FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p matrix-bot interface::http::ai_stream`. Pass.
- [ ] **Step 5: Commit** `feat(http): AI stream events and the web progress sink`.

### Task 6: `POST /api/v1/ai/chat/stream`, its limits, and shutdown

**Files:**
- Modify: `ai_stream/mod.rs` (handler), `backend/src/interface/http/routes.rs` (next to `/ai/chat`), `backend/src/interface/http/auth/types.rs` (`ApiState.ai_turns`), `backend/src/app.rs` (drain on shutdown), `backend/src/interface/http/openapi.rs` or wherever paths are registered, `backend/openapi.json`, `frontend/src/types/api.ts`
- Create: `ai_stream/turns.rs`

**Interfaces:**
- Consumes: Tasks 1, 4 and 5.
- Produces:
  - `AiTurns` (`Clone`), with `fn new(per_account: usize) -> Self` and `fn try_acquire(&self, owner: &AccountId) -> Option<TurnPermit>`. `TurnPermit` releases its slot on `Drop`. It holds a `tokio_util::task::TaskTracker` exposed as `fn tracker(&self) -> &TaskTracker`. `ApiState::new` builds `AiTurns::new(2)`.
  - The handler `chat_stream(State, AuthUser, Json<AiChatRequest>) -> Result<Response, ApiError>`. In order:
    1. Validate the prompt as `chat` does.
    2. If `conversation_id` is present, `services.llm.get_conversation(id, owner)` → 404 "Conversation not found".
    3. `try_acquire` → 429 "Too many AI replies running; wait for one to finish".
    4. Resolve the room.
    5. Make `token = state.shutdown.child_token()`.
    6. Create an mpsc channel of capacity 64.
    7. `tracker.spawn` the turn task. It runs `run_turn` with `WebStreamSink` and `with_cancellation(token)` inside `tokio::time::timeout(180 s)`. On timeout it cancels the token and sends `Error{504,…}`. It sends `Done` or `error_event`, and moves the permit in.
    8. Return `Sse::new(ReceiverStream)` with `KeepAlive` every 15 s, plus the headers `Cache-Control: no-cache` and `X-Accel-Buffering: no`. The body stream owns a `DropGuard` of the token.
  - `app::run` closes the tracker and awaits `wait()` within the existing shutdown bound, after the API server stops.
  - `#[utoipa::path(post, path = "/api/v1/ai/chat/stream", request_body = AiChatRequest, responses((status = 200, content_type = "text/event-stream", body = AiStreamEvent), (status = 404), (status = 429)))]`.

- [ ] **Step 1: Write failing tests** with `setup_state()` as in `ai.rs` tests and a scripted LLM, reading the SSE body into events.
  - `stream_sends_started_deltas_and_done_in_order`
  - `a_foreign_conversation_is_404_before_streaming`: another account's conversation id → JSON 404.
  - `an_empty_prompt_is_a_json_400`
  - `a_provider_failure_after_open_is_an_error_event_with_503`
  - `a_third_concurrent_turn_is_429`: two hanging turns open, the third gets 429.
  - `turn_permit_is_released_when_the_turn_errors`: after a failed turn, a new one is accepted (2 sequential failures + 1 more turn succeeds).
  - `dropping_the_response_body_cancels_the_turn`: hanging LLM; drop the body; the stored last message ends with `_(stopped)_` within 2 s.
  - `shutdown_cancels_a_running_turn`: hanging LLM; `state.shutdown.cancel()`; the stopped marker is stored and `tracker.wait()` finishes within 2 s.
- [ ] **Step 2: Run** them and confirm they FAIL. **Step 3: Implement.**
- [ ] **Step 4: Verify:** `cargo test -p matrix-bot --all-targets` (includes `--test architecture`), clippy, then `cargo run --bin generate-openapi` (the diff only adds the path and `AiStreamEvent`), then `npm --prefix frontend run generate-api` and `npm --prefix frontend run build`.
- [ ] **Step 5: Commit** `feat(http): stream AI turns over SSE with stop and limits`.

### Task 7: Frontend stream client and live-turn store

**Files:**
- Modify: `frontend/src/core/apiClient.ts`
- Create: `frontend/src/modules/ai/liveTurn.ts` (store), `frontend/src/modules/ai/useAiTurn.ts`
- Modify: `frontend/src/modules/ai/aiPromptBus.ts` (start through the store)
- Modify: `frontend/e2e/fake-api.ts`; create `frontend/e2e/ai-stream.spec.ts`

**Interfaces:**
- Produces:
  - `streamApi<E>(endpoint: string, body: unknown, opts: { signal: AbortSignal; onEvent: (e: E) => void }): Promise<void>`.
    - It shares headers, credentials, base URL, and the 401 → `AUTH_EXPIRED_EVENT` handling with `sendRequest` (extract a shared header builder; don't fork it), and has no timeout.
    - A non-2xx response rejects with `ApiHttpError`. A caller abort rejects with `DOMException` `AbortError` unchanged, never "Request timed out".
    - It parses `data:` lines with a streaming `TextDecoder` and skips `:` comment lines.
  - `liveTurn.ts`:
    - `type LivePart = { kind: 'text'; text: string; closed: boolean } | { kind: 'tool'; callId: string; name: string; outcome: 'running' | AiToolOutcome }`.
    - `type LiveTurn = { prompt: string; conversationId: string | null; parts: LivePart[]; phase: 'running' | 'stopping' | 'done' | 'error'; error?: string }`.
    - `startTurn(prompt, conversationId?, roomId?)`, `stopTurn()`, `subscribe(fn)`, `getSnapshot(): LiveTurn | null`, `clearTurn()`.
    - Only one turn at a time: `startTurn` returns `false` while one is `running` or `stopping`.
    - `segment_retracted` removes the last closed text part.
    - `done` → phase `done`.
    - `stopTurn` aborts and sets `stopping`. The store then waits for an `AiConversationUpdated` SSE event (hook it in `modules/ai/index.tsx`'s `handleSseEvent`) or 180 s, invalidates `['ai-messages', id]`, and clears.
  - `useAiTurn()` = `useSyncExternalStore(subscribe, getSnapshot)` plus the actions.
  - `fake-api.ts` handles `POST /ai/chat/stream` through a `window.fetch` stub installed with `addInitScript`. The stub returns a `ReadableStream` driven by a per-test script (`events`, per-event delay, optional "hang until aborted"). Expose `fake.scriptStream(…)`.

- [ ] **Step 1: Write failing Playwright tests** in `ai-stream.spec.ts`. They exercise only the store through a minimal view hook-up, so wire the view's send to `startTurn` in this task:
  - `text arrives in pieces`: two `delta` events 300 ms apart; the first piece is visible before the second.
  - `multibyte text arriving across chunks renders intact`: the stub splits "你好🎉" mid-character across two chunks; the bubble shows "你好🎉".
  - `the turn keeps running after navigating away`: a hanging script; navigate to Todos and back; the bubble is still streaming; then send `done` and the reply shows.
  - `a non-2xx start shows the server error and restores the prompt`: 429 with `{error}`.
- [ ] **Step 2: Run** `npm --prefix frontend run test:e2e -- ai-stream` and confirm they FAIL. **Step 3: Implement.** **Step 4: Run** again (pass) and `npm --prefix frontend run build` (pass).
- [ ] **Step 5: Commit** `feat(web): stream AI turns through a live-turn store`.

### Task 8: AI view split, stop, tool steps and history steps

**Files:**
- Modify: `frontend/src/modules/ai/AiAssistantView.tsx` (keeps toolbar and modals only; under 250 lines)
- Create: `frontend/src/modules/ai/MessageList.tsx`, `LiveTurnBubble.tsx`, `ToolStep.tsx`, `Composer.tsx`, `toolLabels.ts`
- Modify: `frontend/e2e/ai-chat.spec.ts`, `ai-enhancements.spec.ts`, `ai-brain-cross-module.spec.ts` (move from `/ai/chat` routes to `fake.scriptStream`), `ai-stream.spec.ts`

**Interfaces:**
- Consumes: `useAiTurn` and `LivePart` (Task 7), `AiMessage.tool_outcome` (Task 3).
- Produces:
  - `ToolStep({ name, outcome, result? })`: `outcome` is `'running' | 'ok' | 'failed' | 'needs_confirmation' | null` (`null` = an old row, no icon). With `result`, it is a collapsed disclosure (`<details>`, keyboard-reachable) showing the raw text in today's monospace block.
  - `toolLabel(name: string, outcome): string`: a map for the common tools. The fallback turns the name into words, e.g. `add_todo` → "Add todo". Running steps use "…" ("Adding todo…").
  - `Composer`: Send turns into Stop (a square icon with `aria-label="Stop"` and visible text "Stop") while the phase is `running`. `Escape` in the textarea calls `stopTurn`. The input stays editable, but sending is disabled while a turn runs.
  - `LiveTurnBubble` shows "Thinking…" before the first part, "Stopping…" (plus "finishing the current step" if any tool part is `running`), and the error notice on `error`.
  - Remove the old `chatMutation`, its `onSuccess` content de-duplication, and the duplicate `app:ai-prompt` handling (keep one path into `startTurn`).

- [ ] **Step 1: Write failing Playwright tests** (add to `ai-stream.spec.ts`):
  - `a tool step goes from running to done`: `tool_started add_todo`, then `tool_finished ok`. The step text goes "Adding todo…" → "Add todo" with an ok icon.
  - `a refused tool shows needs confirmation`
  - `stop shows stopping and then the stored stopped reply`: hanging script; click Stop. "Stopping…" is visible. The fake then emits `AiConversationUpdated` and the stored messages end with `_(stopped)_`, which is what renders.
  - `escape stops the turn`
  - `a dropped stream falls back to stored history`: the stub ends the stream after `started`, without `done` or `error`. The view refetches after the `AiConversationUpdated` event and shows the stored messages.
  - `stored tool rows render as collapsed steps`: messages with `tool_outcome: 'failed'` and with `null`. The steps are collapsed and expand to the raw result.
  - `the same prompt can be sent twice`: send "hi", then "hi" again. Two user bubbles.
- [ ] **Step 2: Run** them and confirm they FAIL. **Step 3: Implement**, then migrate the three existing AI specs.
- [ ] **Step 4: Run** `npm --prefix frontend run test:e2e` (the whole suite passes) and `npm --prefix frontend run build`.
- [ ] **Step 5: Commit** `feat(web): stop button and live tool steps in the AI assistant`.

### Task 9: Docs and hand-off

**Files:**
- Modify: `docs/llm.md` (the progress-sink methods, the retraction rule, the cancellation safe points, and stopped-marker storage), `docs/api.md` (the streaming endpoint, events table, limits and status codes, and the note that `/ai/chat` is unchanged), `docs/superpowers/HANDOFF.md` (status row, rulings for: keeping `/ai/chat`, abort-as-stop, per-account cap 2, `tool_outcome` column, turn outliving navigation)

- [ ] **Step 1:** Write the doc changes.
- [ ] **Step 2: Run the full gate list from AGENTS §8.** `fmt`, `check`, `test --all-targets`, `live_postgres -- --ignored --test-threads=1` (with `PG_TEST_URL` set), `clippy`, `generate-openapi` + `git diff --exit-code backend/openapi.json`, `sqlx prepare --check`, the frontend build, and `test:e2e`. Then `git diff --check`. All green.
- [ ] **Step 3: Commit** `docs: web AI streaming, stop and tool steps`.
