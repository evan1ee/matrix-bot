# Web AI assistant: streaming replies, stop, live tool steps (phase 1)

Status: draft, awaiting owner approval. Date: 2026-10-18.

## Why

The web AI Assistant page (`frontend/src/modules/ai/AiAssistantView.tsx`) sends one `POST /api/v1/ai/chat` and waits up to 180 s behind a spinner that says "Thinking and executing tools...". Nothing tells you what is happening, you can't stop a turn that has gone the wrong way, and the reply appears all at once.

The backend can already do better. `llm::run_turn` reports progress to a `ProgressSink` (`llm/progress.rs`): stages, tool calls and streamed text deltas. Matrix, Discord and Telegram each plug a sink in and stream replies. The web handler (`interface/http/ai.rs::chat`) passes `None`. So of all the front doors, the web, the richest one, is the only one that can't stream.

Phase 1 closes that gap. Later phases (conversation history sidebar, a "New chat" that doesn't delete, regenerate/copy, attachments) are out of scope here.

## Goals

1. **Streaming replies.** The assistant's text shows up as the model writes it.
2. **Stop.** While a turn runs, the Send button becomes Stop. Pressing it ends the turn quickly and leaves the conversation in a state that makes sense.
3. **Live tool steps.** Each tool the agent calls shows as a compact step in the reply ("Adding todo…" → "Added todo ✓" / "failed" / "needs your confirmation"), in the order things happened, mixed in with the streamed text.
4. **The same steps in history.** Reloading a conversation shows the stored tool rows as the same compact steps (expandable to the raw result), not the raw JSON blocks shown today.

## Non-goals

- Conversation list or switching, renaming, "New chat" without deleting: phase 2.
- Regenerate, edit-and-resend, copy-reply buttons: phase 3.
- Changing how Matrix, Discord or Telegram show progress. Their visible behaviour must not change.
- Resuming a turn's stream after a page reload or from a second tab. A turn's live stream belongs to the request that started it. Other tabs still see the result through the existing `AiConversationUpdated` SSE event.
- Interrupting a tool that is already running.

## Anti-goals (failing even if it "works")

- A stop that leaves a half-done record: a tool that did half its write, or a turn still running after the UI says it stopped.
- Streaming deltas through the outbox or the `/events` hub. The outbox is durable state-change history, not a token pipe.
- Duplicate turn logic: the web path calls the same `run_turn` as every other channel; no web-only copy of the loop.
- The UI claiming something was saved when the tool failed or was refused.

## Design

### Transport: one streaming request per turn

New endpoint **`POST /api/v1/ai/chat/stream`**. It takes the same body as `/ai/chat` (`AiChatRequest`: `prompt`, `conversation_id?`, `room_id?`) and answers `200 text/event-stream`. The browser reads it with `fetch` plus a `ReadableStream` reader (an `EventSource` can't POST). Validation failures that come before the turn starts (empty or too-long prompt, unknown conversation, unauthenticated) are normal JSON `ApiError` responses with their status codes, not stream events.

Why a per-request stream and not the `/events` hub: the hub is fed only by the outbox (durable, committed events per account); deltas are short-lived and belong to one caller. A per-request stream also ties the turn's lifetime to the one consumer that cares about it, which is exactly what Stop needs.

`POST /ai/chat` stays unchanged. It is a public, bearer-usable contract in `openapi.json`, and other callers can keep using it. Both endpoints go through `run_turn`. *(My call: keep it rather than delete it. Cost if wrong: one extra thin handler.)*

### Stream events

Each SSE message is an unnamed `data:` line holding one JSON object, `#[serde(tag = "type")]`, documented in OpenAPI as a `ToSchema` enum `AiStreamEvent` so `npm run generate-api` produces the frontend type:

| `type` | Fields | Meaning |
|---|---|---|
| `started` | `conversation_id` | The conversation this turn writes to. `run_turn` may create it or reuse the room's recent one (`SESSION_TTL_SECONDS`), so this is the only way the client learns which. First whenever it is sent; a turn that fails before resolving its conversation sends only `error`. |
| `delta` | `text` | Appended text for the current text segment. |
| `segment_end` | — | The current text segment is finished (maps to `ProgressSink::flush`). The next `delta` starts a new segment. |
| `segment_retracted` | — | Throw away the current text segment: the open one if text is still arriving, otherwise the one that just ended. Sent when `llm::guard` rejects a reply that claims an unrecorded success (`turn.rs` correction round / `UNRECORDED_NOTICE`), and when streaming fails mid-reply and `stream.rs` falls back to a non-streamed completion (the fallback text then arrives as fresh `delta`s plus `segment_end`). |
| `tool_started` | `call_id`, `name` | A tool call began. |
| `tool_finished` | `call_id`, `name`, `outcome`: `ok` \| `failed` \| `needs_confirmation` | A tool call ended. `needs_confirmation` is the `llm::guard` refusal. |
| `done` | `conversation_id`, `message`, `model_used`, `tool_calls_executed` | Final reply, the same fields as `AiChatResponse`. Always last on success. |
| `error` | `status`, `message` | The turn failed after the stream opened. `status` is the HTTP status `/ai/chat` would have returned for the same `AiError` (400/503/504/500), and `message` is the same client-safe text. Last event. |

Keep-alive comments every 15 s, like `sse.rs`. The response sets `Cache-Control: no-cache` and `X-Accel-Buffering: no` so a reverse proxy doesn't buffer it.

Stage text (`report_stage`, e.g. "🤔 思考中...") is **not** forwarded. The web UI draws its own "Thinking…" state from the event sequence (opened, no delta or tool yet). *(My call.)*

### `ProgressSink` changes

Today `run_turn` reports only the **first** tool call of a round (`sink.report_tool_call(&first_tc.function.name)`) and never reports outcomes. The web needs every call and its outcome. Add these methods **with default implementations**, so the Matrix, Discord and Telegram sinks compile and behave exactly as they do now:

- `report_conversation(&self, conversation_id: &str)`: default no-op. Called once, as soon as `run_turn` has resolved the conversation. Drives `started`.
- `report_tool_started(&self, call_id: &str, name: &str)`: default no-op.
- `report_tool_finished(&self, call_id: &str, name: &str, outcome: ToolStepOutcome)`: default no-op. `ToolStepOutcome` (`Ok | Failed | NeedsConfirmation`) is a small enum in `llm/progress.rs`. It's not called `ToolOutcome` because `llm::tool_round::ToolOutcome` already exists.
- `retract_segment(&self)`: default no-op (Matrix already replaces its stage message with the final reply). Called in the two cases listed under `segment_retracted`.

`run_turn` keeps its existing `report_tool_call(first)` call for the old sinks, and also calls `report_tool_started` for every call in the round before `tool_round::run_calls`, then `report_tool_finished` for every outcome. The outcome comes from the `refused` and `succeeded` flags `run_calls` already returns. The web sink implements only the new methods plus `push_delta` and `flush`.

The builder may restructure this (for example, folding `report_tool_call` into the new method for every sink) **only if** the Matrix, Discord and Telegram output stays visibly the same, with tests to prove it.

### Turn lifetime, Stop, and cancellation

The handler does not run the turn inside the response body future (dropping that future would cancel `run_turn` at any `.await`, including halfway through a tool's writes). Instead:

1. The handler validates, resolves the room as `chat` does, creates a **child `CancellationToken`** of the app shutdown token, and spawns the turn on a task that `app::run`'s shutdown can see (per AGENTS §4.3: no detached tasks).
2. The web sink sends events on a **bounded** `mpsc` channel to the response stream. The response stream holds a drop guard: when the client goes away (Stop, closed tab, network loss), hyper drops the body, the guard cancels the token.
3. `run_turn` gets an optional cancellation token (on `LlmTurnContext`, default none, so the other channels don't change). It checks the token at **safe points only**:
   - while waiting on an LLM request or stream (race the call against `cancelled()` and drop the request);
   - before starting a tool round;
   - before the final "summarise the tool results" completion.
   It never cancels inside `tool_round::run_calls`. A tool that started finishes and its result row is persisted, so nothing is half-written. Stop takes effect after the running tool round ends.
4. On cancellation, `run_turn` stores an assistant message that marks the turn as stopped: the partial text of the current segment (if any) followed by a short marker. `run_turn` never sees deltas today (they go straight from the LLM client to the sink), so the stream helper in `llm/stream.rs` takes the token and, when it is cancelled, returns the text it collected so far, e.g. as an `Err(Cancelled { partial })`. The stopped message is built, e.g. `_(stopped)_`. It does this through the existing `append_assistant_message`, so the `AiConversationUpdated` event fires as usual. It then returns a new `AiError::Cancelled` variant. The `/ai/chat` mapping for it is `499`-style "client closed": since no client is listening, log it at `info` and send nothing. Never map it to 500.
5. An overall 180 s limit, the same as `/ai/chat`, applies to the spawned turn. On timeout the stream sends `error` with 504 and the turn is cancelled the same way.

Stop in the UI = `AbortController.abort()` on the fetch. There is no separate stop endpoint: closing the stream is the stop signal. *(My call. Cost if wrong: a proxy that keeps the upstream connection open after the browser aborts would delay the stop until the next write fails. The bounded channel's send error is a second trigger, so the turn still stops on its next event.)*

**The UI says what really happened.** A tool round that's already running finishes after Stop, and the aborted client no longer hears about it. So the live bubble doesn't say "Stopped" right away. It says **"Stopping…"** (and "finishing the current step" when a tool step was running). The stored stopped message triggers the usual `AiConversationUpdated` SSE event. When it arrives, the client refetches the conversation, and the stored history (tool rows with their real outcomes, then the stopped marker) replaces the live bubble. If no event arrives within the turn's remaining time limit, the client refetches anyway. *(My call: this keeps the anti-goal without a stop endpoint or a server-side turn registry.)*

### Limits

- Per-account cap on concurrent streaming turns (2), refused with 429 before the stream opens. Nothing like this exists yet: `/ai/chat` has no limit, and `interface/http/rate_limit.rs` only counts requests per IP or token. So this is a new, small in-process counter per account (a permit released when the turn task ends, however it ends). No requests-per-window limit in this phase. *(My call. Cost if wrong: one more limiter later.)*
- Unknown or foreign `conversation_id`: the handler checks ownership through `LlmService` before the stream opens and answers 404. `/ai/chat` keeps its current behaviour (`run_turn` → `Validation` → 400). *(My call: the old endpoint's contract stays put.)*
- The bounded event channel (suggest 64). If it's full, the sink awaits (backpressure slows how fast the LLM stream is read), and it never drops `tool_*`, `done` or `error`. Coalescing `delta`s while the channel is full is allowed.
- Prompt cap 4000 chars, as now.

### Frontend

- `core/apiClient.ts` gets a streaming helper (e.g. `streamApi`) that does the POST with the same auth, CSRF/Origin behaviour and base URL as `fetchApi`, parses SSE frames into typed `AiStreamEvent`s, and takes an `AbortSignal`. All calls still go through `apiClient`. Today every abort in `apiClient` is reported as "Request timed out"; the helper must tell a caller's abort (Stop) apart from its own timeout, and it has no whole-request timeout (the server's 180 s limit and the 15 s keep-alive cover that).
- The AI view keeps a **live turn** in local state: an ordered list of parts, each a text segment (built from `delta`s and closed by `segment_end`) or a tool step (by `call_id`, updated by `tool_finished`). It renders as one assistant bubble below the user's message, using `MarkdownRenderer` for text.
- `started` sets the active conversation. `done` clears the live turn and invalidates `['ai-messages', id]` and `['ai-conversations']`, so the stored history takes over. No optimistic de-duplication by message text. The current `m.content === variables` check hides a repeated identical prompt and must go.
- **Stop button.** While a turn runs, Send turns into a Stop button (square icon, visible label for screen readers). `Escape` in the input also stops. After Stop, the live bubble keeps what arrived and shows "Stopped". Then the refetch replaces it with the stored stopped message.
- **Errors.** An `error` event or a failed request shows the existing red notice with the server's message and puts the prompt back in the input, as today.
- **Tool steps.** One line per step: an icon for running/ok/failed/needs-confirmation, plus a label. Labels come from a small frontend map of tool name → label, falling back to the name turned into words ("add_todo" → "Add todo"). A step in history (a stored `tool` row) renders the same way, collapsed, and expands to show the raw result in a monospace block (today's look, kept behind the expander).
- **Stored outcome.** Stored tool rows hold no outcome today, and guessing it from the result text (`"Error executing tool: "`, `{"error":"confirmation_required"}`) would copy backend string formats into the frontend. So add a forward-only migration with a nullable `tool_outcome` column on the AI messages table (`ok | failed | needs_confirmation`). `run_turn` writes it with each tool row, and `AiMessage` exposes it as an optional enum. Older rows have no value and render as a neutral step with no ok/failed icon. Regenerate `.sqlx/`. *(My call; this replaces the earlier "no migration".)*
- `registerAiPromptListener` and the `app:ai-prompt` event both start turns through the same streaming path.
- **The turn outlives the page.** *(Owner's decision.)* Moving to another module mid-turn must not stop it; only Stop or closing the tab does. So the live turn (the stream reader, its `AbortController`, and the ordered parts) lives in a module-level store under `modules/ai/`, not in component state. The view subscribes to it (e.g. `useSyncExternalStore`) and on remount shows the turn still running or, if it ended meanwhile, the refetched result. Unmounting the view never aborts. One live turn at a time per tab: Send stays disabled while one runs, wherever it was started (`triggerAiPrompt` included). Optional nice-to-have, builder's call: a small "AI is replying…" indicator on the AI nav item while a turn runs.
- `AiAssistantView.tsx` is already at the 500-line limit. Split it along real seams: the live-turn store plus a `useAiTurn` hook over it, the message list, the tool-step component, and the composer.
- Follow `Design.md`: monochrome, `transition-colors duration-150`, visible focus, keyboard access.
- `frontend/e2e/fake-api.ts` gets a fake streaming endpoint. `page.route` + `route.fulfill` sends the whole body at once, so it can't drip text or hold a stream open for Stop. The builder picks another way: a stubbed `window.fetch` for this one path via `addInitScript` that returns a `ReadableStream` the test controls, or a tiny local server. The Playwright spec covers: text arrives in pieces, leaving the AI page mid-turn and coming back shows the turn still running (and then its result), a tool step goes running → ok, Stop mid-turn, an `error` event, and a stored tool row rendering as a step.

## Tests the builder must add

Backend:
- A turn with a recording sink: every call in a round gets `report_tool_started`, and every outcome gets `report_tool_finished` with the right `ToolOutcome` (ok, failed, refused → `needs_confirmation`).
- Existing Matrix/Discord/Telegram sink tests still pass unchanged (behaviour is the same).
- A reply the guard rejects leads to `retract_segment`, and so does a stream that fails halfway and falls back; the final `done.message` matches the stored reply.
- Tool rows are stored with the right `tool_outcome`.
- Cancel during an LLM stream: the turn returns `Cancelled`, the stored conversation ends with the stopped marker, and no tool round started after the cancel.
- Cancel while a tool round runs: the round completes, its tool rows are stored, no further LLM call is made, and the stopped marker is stored.
- Client disconnect cancels the token (drop guard).
- Endpoint: pre-stream validation returns JSON errors with the right status; a provider failure after open sends an `error` event with 503; event order is `started` … `done`.
- The concurrency cap returns 429.
- Owner isolation: streaming into another account's `conversation_id` is a 404 before the stream opens.

All of this uses the fake LLM server in `llm/test_support` / `infrastructure/inference/test_support`. Nothing calls a real provider.

## Contracts touched

- `openapi.json`: new path plus the `AiStreamEvent` schema. `/ai/chat` unchanged.
- `ProgressSink`: additive methods with defaults.
- `AiError`: new `Cancelled` variant; the `From<AiError> for ApiError` match handles it.
- One additive migration (nullable `tool_outcome`). Stopped turns are ordinary assistant messages.
- `docs/llm.md` (progress sinks, cancellation safe points) and `docs/api.md` (the streaming endpoint and its events) are updated in the same change.

## Left to the builder

- Exact Rust names, module placement of the web sink (`interface/http/ai_stream.rs` or similar), and how the spawned task joins shutdown, within AGENTS §4.3.
- The tool-label map's wording.
- The exact stopped-marker text.
- Coalescing strategy for deltas under backpressure.

## Review findings that motivated this (for the record)

From reading the current page and handler, 2026-10-18:

- The web turn passes `progress: None`, so no streaming (fixed here).
- No way to stop a turn (fixed here).
- Tool rows render as raw JSON (fixed here); `tool_calls_executed` in the response is ignored.
- The optimistic update drops a repeated identical prompt (fixed here).
- "New chat" deletes the current conversation; only the newest conversation is reachable; history shows only the last 100 messages (phase 2).
- `get_ai_config` merges config in the handler and the handlers call `llm_repo` directly instead of going through `LlmService`, against AGENTS §2 rule 2 (separate cleanup; not in this phase).
- The model chip falls back to a hard-coded `gpt-4o-mini` label (small fix, phase 2).
- The 4000 prompt cap counts characters in the frontend and bytes in the backend (`.len()`), so a Chinese prompt is refused at about 1,333 characters (separate fix).
