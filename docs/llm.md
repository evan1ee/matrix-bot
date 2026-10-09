# LLM, agents and MCP

How the AI assistant, its tools and the MCP server are built. The governing rule: **the LLM is an untrusted orchestration component, not a source of truth.** Its output reaches application state only through a validated, independently authorized tool call into a deterministic application service, never through a direct mutation. Layers and dependency direction are in [`architecture.md`](architecture.md); the wire surface is in [`api.md`](api.md).

```text
Chat DM / mention ───┐
Web chat (/ai/chat) ─┼─► llm::run_turn ─► LLM (OpenAI-compatible) ─► tool call
                     │        ▲                                        │
                     │        └──────────── tool result ◄──────────────┤
MCP client ──────────┴───────────────────────────────────────────────► llm/tools (same registry)
                                                                        │
                                                                        ▼
                                                     application service → domain → repository → PostgreSQL
```

## Where things live

| Concern | Path (under `backend/src/`) |
|---|---|
| Turn loop, limits, streaming, prompts, context, retrospective, price alerts | `llm/{turn,stream,prompt,context,retrospective,alert}.rs` |
| Tool round, taint and confirmation, success-claim check | `llm/{tool_round,guard}.rs` |
| Tool definitions and executors, one file per domain | `llm/tools/<domain>.rs`; `definitions.rs` aggregates, `executor.rs` dispatches |
| Web search (Tavily, DuckDuckGo fallback) and the search-intent gate | `llm/search.rs` |
| Effective per-user configuration (env defaults merged with saved settings) | `llm/resolved_config.rs` |
| LLM HTTP transport and wire types (`ChatCompletionRequest`, `ToolDefinition`, …) | `infrastructure/inference/llm/{client,types}.rs` |
| Conversations, messages, user config (use cases, events) | `application/llm`, `domain/llm`, `infrastructure/postgres/repositories/llm.rs` |
| Chat front door (`!ai`, DMs, mentions; any channel) | `interface/chat/llm.rs` |
| Progress reporting seam (`ProgressSink`, `NoProgress`) | `llm/progress.rs`; Matrix's in-place message editor is `interface/channels/matrix/stage.rs` |
| Outage alert to the owner's dev conversation | `interface/chat/llm_alert.rs` (through the channel registry) |
| Web front door (`/ai/*`, `/account/ai-config`) | `interface/http/ai.rs` |
| MCP server | `interface/mcp/`, `interface/http/mcp.rs`, `interface/chat/mcp.rs`, `domain/mcp`, `infrastructure/postgres/repositories/mcp.rs` |

## Transport and configuration

`infrastructure/inference/llm` speaks any OpenAI-compatible chat-completions API (`/v1/chat/completions`, streaming or not) reached by **base URL alone**. There is deliberately no per-vendor provider enum and no vendor SDK: the base URL already reaches xAI, OpenAI, Groq, DeepSeek, OpenRouter, Ollama and vLLM. Everything that is the bot's own business (prompts, the tool loop, conversation storage, config merging) lives in `llm/` and `interface/chat/llm.rs`; the transport receives only `ResolvedAiConfig::endpoint()`. Text-to-speech is the one place a dialect exists (ElevenLabs); see [`architecture.md`](architecture.md#voice-replies-say).

- Settings read exactly one env name each: `AI_ENDPOINT` (base URL including `/v1`), `AI_API_KEY`, `AI_MODEL`; web search reads `TAVILY_API_KEY`.
- The AI transport's proxy chain is `AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy` → `HTTP_PROXY`/`http_proxy`. The web-search client has a separate, shorter chain (`AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy`, no `HTTP_PROXY`). Do not assume they share a list.
- A user may override endpoint, key, model, system prompt, temperature and token limit (`ai_user_configs`; `GET/PUT /account/ai-config`). A per-user base URL is an SSRF surface: one that resolves to a private host is refused unless `AI_ALLOW_PRIVATE_BASE_URL` is on (off by default). API keys are redacted in `GET` responses and never logged.
- Every external call has a timeout; a provider's types never enter the domain; a missing or unreachable provider degrades the AI features only, and the bot still boots.

## The turn

`llm::run_turn` runs one user turn for an `LlmTurnContext` (services, pool, env config, account, conversation key and an optional `&dyn ProgressSink`) and a prompt:

1. **Trigger.** In a DM, any message no command module claims goes to the AI (`LlmModule` registers after the TTS module for that reason). In a shared room it triggers on `!ai <prompt>` or a mention. A quoted reply carries its text into the prompt through `QuotedMessage::to_context_prompt()`, inside a `<quoted_message>` block (see *Untrusted content and confirmation*).
2. **Context.** The active conversation for `(owner, room)` is loaded; one idle longer than `SESSION_TTL_SECONDS` (1 h) is replaced by a fresh one, and `!ai reset` clears it. Immediate retries are deduped by checking the conversation's tail message before inserting.
3. **Tool loop.** The model may request tools; each round executes them, appends `tool` messages and calls the model again. Concurrent calls in a round run through `futures_util::future::join_all`. The loop stops after `MAX_TOOL_ROUNDS` (5); the Matrix front door rejects prompts over `MAX_PROMPT_LENGTH` (16384 bytes).
4. **Reply.** The turn returns text; the front door wraps it in a `Reply` and the adapter renders it (Matrix: `text_html(&plain, &html)`). When a `ProgressSink` is present the turn reports stages, tool calls and streamed deltas to it and falls back to a plain completion on any transport error; each adapter decides how to show them (Matrix edits one message in place, a channel that cannot edit ignores most of it). Every sink method is best-effort and infallible, so a sink can never fail a turn. The turn does not record which message a tool created: the Matrix callers do that after the turn from the sink's `message_ref()` (R77). The web endpoint is `POST /ai/chat`. Failures reach the user through `interface::chat::error_reply` and never show a provider or database error.

### Untrusted content and confirmation

Text the model reads can carry instructions: a quoted message, a web page, a webhook payload, a knowledge note. The chat turn treats it as data in three ways (`llm/guard.rs`, `llm/tool_round.rs`):

- **Delimiters.** Three wrappers, each around text an outsider wrote: `<quoted_message author="…">…</quoted_message>` (`QuotedMessage::to_context_prompt`, followed by `User request: …`), `<ocr_text>…</ocr_text>` (`build_ocr_prompt`) and `<tool_output name="…">…</tool_output>` (every tool result sent to the model). `escape_wrapped_content` rewrites **every** `<` (ASCII and full-width `＜`) inside the content as `&lt;`, so no tag, spaced, mixed-case or zero-width variant included, can be formed to end a wrapper; only the wrapper's own tags contain a `<`. The wrapping is applied to the model-facing copy only: the stored tool message and `AiTurnResult::tool_results` keep the raw result, because adapters parse it. The static prompt's *Untrusted content* section names exactly these three tags. The autonomous agent (`llm/agent/`) has its own prompt, which names `<tool_output>` only, and wraps its tool results the same way (see *Agent runs use native tool calling*).
- **Taint.** A turn is tainted when its prompt embeds outside text, or when an earlier round ran a tool that returns outside content. Prompt sources (`LlmTurnContext::with_untrusted_input()`): any quote, the bot's own included (`interface/chat/llm.rs`) and OCR text (`interface/chat/ocr`). Tools (`guard::returns_outside_content`, matched by name even if the call failed): `web_search`, `get_webhook_logs`, `knowledge_search`, `knowledge_get_note` (a note body is whatever the user pasted or clipped), `get_my_bookmarks` (titles and descriptions are fetched from the web). Calls in the same round do not taint each other.
- **Every quote taints, the bot's own included.** The bot account is not a trusted author: webhook alerts (`interface/events/channel_sink.rs`), link notices and reminders are all posted through it, and some carry text an outside caller chose. So a quote of any message, whoever sent it, is outside content. To confirm a held action the user answers in a message of their own ("yes"); the conversation history already holds the bot's question, and that turn is untainted. The friction: in a client where "reply" auto-quotes, a confirmation sent as a reply is refused once more, and the user must send it as a plain message (in a shared room, mentioning the bot or using `!ai`). Restoring quote-based confirmation needs a way to recognise the bot's own AI replies without trusting any text in them (a footer can be forged). Options, in the order to try them if the friction becomes a complaint: (1) a bounded in-memory set of the ids of messages sent as AI replies (no migration; lost on restart, which fails safe); (2) a table of AI reply ids (migration, `OWNED_TABLES` entry); (3) on Matrix only, AI replies could carry a custom content key, trusted only together with the homeserver-reported sender check of the replied-to event (Telegram has no hidden field). All need the sent message's id plumbed through each adapter, including edit-in-place replies. See the HANDOFF ruling.
- **Confirmation.** While tainted, a sensitive tool (`agent::sandbox::is_sensitive_tool`) is not run: trades, payments, deletions, `cancel_reminder` and `update_my_settings`. The model receives `{"error":"confirmation_required", …}` as the tool result and is told to ask the user; the user's next message starts an untainted turn and may perform it. A refused call is not counted in `tool_calls_executed` or `tool_results`. A plain turn runs sensitive tools as before. The same list drives agent approvals, so a background agent run now also needs approval for `cancel_reminder` and `update_my_settings`. `update_todo` is deliberately not sensitive.

**Success claims.** If no tool succeeded this turn (`any_tool_succeeded`: a tool returned `Ok`) and the reply *starts with* `✅` (the confirmation format), the turn sends one language-neutral correction message and asks again. A checkmark elsewhere in a reply, as in a fact-check, is ignored. A reply that still starts with `✅` afterwards, or a correction that used the last round, is replaced by a bilingual notice ("未记录任何操作 / No action was recorded."), never by an empty-reply message.

**Residual risks.**

- Taint is per turn. The quoted prompt stays in the conversation history, so a later plain turn can read it again and is not tainted by it.
- A user-supplied `system_prompt` replaces the static prompt, including the untrusted-content rule. The code-side taint check and confirmation still apply, so sensitive tools stay protected.
- MCP has no taint: the client is the user's own, and the tools it calls are owner-scoped.
- Calls issued in the same round are not tainted by each other, so a model can request an outside-content tool and a sensitive tool together. Only the later rounds are protected.
- OCR is wrapped and tainted, but the OCR prompt text still carries its own zh/en reply templates; the voice-transcript prompt is not wrapped or tainted.

### The system prompt

`llm/prompt.rs` renders one system message in two parts, so providers can cache the prefix:

1. **Static prefix** (`SYSTEM_PROMPT_STATIC`): a `const` with no interpolation, written in sections (role, language, tools and truthfulness, dates and times, money, rooms, quoted replies, untrusted content). Each rule appears once. It carries no hard-coded zh/en reply templates: after a successful action the model starts with ✅ and a short bold title, then bullets of the tool's key fields, in the user's language. It states that calls in one round run concurrently, so a call that needs an earlier result must wait for a later round. It never contains the owner id, a timestamp or the word UTC. A test proves the prefix is byte-identical across contexts.
2. **Context section** (`PromptContext`, appended after the prefix): time zone, local time at minute precision (`YYYY-MM-DD HH:MM (Weekday)`, no seconds, no zone suffix), default currency, default room and at most 20 rooms followed by "…and N more". Rooms beyond the cap still resolve by name through `RoomService::resolve_target_room`; the prompt is not their source of truth. The owner id is never put in the prompt: tools take the owner from the turn's authenticated account.

`build_system_prompt` only gathers data and calls the pure `render_system_prompt`. A user-supplied `system_prompt` (`ai_user_configs`) replaces both parts and is returned unchanged.

**Caching.** The static prefix is only about 550-650 tokens, so on its own it may sit below a provider's minimum cacheable length (about 1024 tokens for several providers). Providers serialise `tools` before `messages`, so the cached prefix is the tools array followed by the static prompt, which together is well above that minimum. The context section is appended after the prefix and changes whenever the minute, the room cache or the user's settings change; that never invalidates the prefix. The main remaining cache break is the conditional `web_search` tool: when intent gating adds or removes it, the tools array, and with it the whole prefix, changes.

### The agent knows no channel

`llm/` imports no channel SDK and nothing from `interface::channels` (`the_agent_knows_no_channel` in `tests/architecture.rs`), so Discord, Telegram or Email arrive without touching the loop: the agent talks to the world through `ProgressSink` going out and `AppServices` going in. The one place the AI addresses a channel on its own is the **outage alert**: when the provider fails, `interface/chat/llm_alert.rs` sends a notice to the owner's `dev_room_id` (parsed from its storage form) or, failing that, to the Matrix room named "dev", through `ChannelRegistry::send` and `find_conversation_by_name` (A4), so it works for whichever channel the room lives on and degrades to a warning log when no registry or adapter is available.

### Agent runs use native tool calling

The autonomous agent (`llm/agent/`, run by `interface/jobs/agent.rs`) does not use a text protocol. One model call evaluates one step:

- **Messages** (`prompts::build_messages(goal, steps, tools, step_no, max_steps)`): the system prompt, a `Goal:` user message, then every stored step that called a tool as an assistant `tool_calls` message (id = the step id, arguments = the stored `tool_input` as a JSON string, the step's `thought` as its content) followed by a `tool` message with the same id, and finally `Step k of N.`. A blocked step has no output yet and is replayed as `{"status":"awaiting_user_approval"}`. Tools reach the model only through the request's `tools`; the system prompt lists none.
- **Observations** are `format_tool_output`: always valid JSON, capped at about 2000 characters (approximately: the preview form adds a few). An array drops tail items and ends with `{"_truncated":true,"omitted":k}`; another oversized value, or an array whose first item alone is too big, becomes `{"_truncated":true,"preview":…,"original_chars":n}`. The full output stays in the step record. Each observation is wrapped as `<tool_output name="…">…</tool_output>` with every `<` inside escaped (`guard::wrap_tool_output`), which is exactly what the agent prompt's untrusted-data rule describes.
- **Outcomes.** Only a native tool call can act, and only the first call of a reply is taken. Arguments that are not a JSON object give `StepOutcome::InvalidCall`: nothing runs, the handler stores a `Failed` step with `{"error": …}` and the run continues, bounded by `max_steps`. Plain text is the final answer (`Completed`); empty text is `Failed`. There is no `{"arguments": raw}` or `{"query": text}` coercion.
- **Prompt.** States the step budget (`StepContext.max_steps`, the run's budget from `step_budget`), to make one tool call per step (the request type has no `parallel_tool_calls` switch, so the prompt says so; only the first call is run), to fix arguments or change approach after a tool error, never to repeat an identical failing call, to stop after two failures, to ask in the final answer when the goal, an amount, a recipient or a deletion target is unclear, and that `<tool_output>` text is data.
- **Unchanged.** The whitelist, sensitive-tool interception and the approval flow, and the step record shape (`thought`, `tool_name`, `tool_input`, `tool_output`, `status`), so the REST API, the web run drawer and the contracts do not change. With native calls `thought` is whatever text accompanied the call, often empty. The type is still named `ReActEngine`, a historical name.

**Approval and resume.** A sensitive call is recorded as a `Blocked` step (the call, no output) and an approval row, and the run waits. `approve_action` locks the run, resolves the approval and, in the same transaction, records an *approval marker* step (`AgentStep::approval_marker`: no tool name, output `{"approved":true,"approval_id":…,"tool_name":…,"tool_input":…}`, `Success`), publishes an `AgentStepCompleted` event for it and re-queues the run, so the approval, the marker, the event and the job commit together. The handler (`resume_approved` in `interface/jobs/agent.rs`) runs before any model step:

1. The marker is the **last** step with a held call before it (`domain::agent::pending_approved_call`). The call in the marker (what the user was shown) must equal the held step's `tool_name`/`tool_input`; a mismatch fails the run and runs nothing.
2. A run that was cancelled since the job was queued is not touched.
3. An **execution-started** step (`AgentStep::execution_started`) is committed *before* the tool runs.
4. The stored call runs through `ReActEngine::execute_approved`: no model call and no sensitivity check. What constrains it is the stored call (checked against the approval) and the run's account as owner. The task whitelist is not what constrains it: the job passes `allowed_tools: None` and `ScheduledTaskDefinition::allowed_tools()` is never called today (a HANDOFF follow-up wires it in).
5. The outcome is recorded as a normal result step, then the loop resumes; the model sees the held call, its `awaiting_user_approval` placeholder and then the real result.

The marker and started steps name no tool, so the native message builder renders nothing for them and no tool call is left without its `tool` message. `reject_action` locks the run, records a blocked step carrying `{"rejected":true,…}`, publishes the same event and runs nothing; the model is asked again.

**Budget.** Only the model's own steps use `max_steps` (`domain::agent::model_steps`): approval markers, started markers, the result step after a started marker and rejection steps do not count, and the "Step k of N" the model sees counts model steps only. After each approved execution the budget grows by one (`step_budget`) so the model can always report what happened, but never beyond twice `max_steps` in total, so approvals cannot extend a run indefinitely (at the cap the run fails with a `Max steps` message); a held call at the last permitted step is approved, runs once and the run ends `Completed` with a final answer.

**What is guaranteed.**

- *At most once:* if the process dies after the started step is committed and before the result step is, the retry finds the started step last, cannot know whether the tool ran, and **fails the run** telling the user to check manually; it never runs the tool again. A started step that was never followed by the tool (a crash right after it) is indistinguishable, so such a run also needs a manual check. Cost: a rare interrupted approved action needs manual verification instead of risking a double trade or payment.
- *Every status writer takes turns:* approve, reject, cancel, complete, fail and require-approval each lock the run row first (always the run row first, so there is one lock order) and the resume step's egin_approved_execution locks it, checks the run is still active and commits the started step in one transaction. Each locks the run row (`SELECT … FOR UPDATE`, `repositories::agent::lock_run`) before reading its status, so the status checked is the status written. Cancel then approve: the approve is refused with a conflict because the run is no longer `WaitingApproval`. Approve then cancel: the run ends `Cancelled`, and the handler does not run the held call (it checks the run again before starting it). Two concurrent approvals: the second finds the approval already resolved (the boolean from `resolve_approval` is honoured) and fails, so there is one marker and one job. An expired approval is refused by the domain.
- *Not guaranteed:* cancelling a run leaves its approval row pending (it can no longer be resolved); a cancel that arrives while the tool is already running does not stop it; run-status transitions (`waiting_approval`, `executing`) publish no event of their own, only the step events and `AgentApprovalRequired` / `AgentRunCompleted`.
- *Concurrent workers:* the job runtime claims a job with `SKIP LOCKED`, so one worker runs a given job. A job re-claimed after a lease expiry while the first worker is still running cannot double-record a step: the `(run_id, step_number)` unique constraint fails the second writer.
- *Step numbers:* steps are numbered by how many exist (`approvals.rs::next_step_number`), as the handler does; `run.current_step` is not advanced when steps are recorded.

### Agent rules

An agent may reason about a task, select tools, call several, iterate on results and coordinate capabilities. It must not bypass application boundaries. Every loop has explicit limits: iterations (`MAX_TOOL_ROUNDS` for chat, `max_steps` for agent runs), a timeout, a bound on concurrent tool calls, error handling and cancellation. Do not introduce an agent framework where a controlled loop suffices, and keep agent orchestration out of domain services. Long research requests need the generous router timeout (`TimeoutLayer` ≥ 180 s).

## Tools

Define tools with the standard OpenAI `type: "function"` schema (`ToolDefinition`), never a provider-specific tool variant. Each domain file exports `pub fn tool_definitions() -> Vec<ToolDefinition>` beside its executor, and `definitions.rs::all_tool_definitions()` only concatenates them. The same registry serves the Matrix/web assistant **and** MCP, so a tool is written once.

Adding a tool:

1. Add the schema and the executor case in `llm/tools/<domain>.rs`.
2. The executor calls the domain's **service** (`services.<f>`), never SQL and never a repository through a pass-through layer.
3. Resolve the owner from the turn's authenticated account; ignore any owner, account or resource-ownership value the model supplies.
4. Validate arguments as untrusted input (types, ranges, the 500-character cap) and return a structured error string the model can act on.
5. A mutation publishes its `SystemEvent` through the service (in its transaction, via the outbox), exactly like a chat command or REST call. A destructive operation needs explicit confirmation semantics where appropriate.
6. Test malformed arguments, an unknown tool name, tool failure and duplicate calls, and assert the owner scope.

Well-formed model output is still not authoritative. Tool results are fed back as JSON, prompts state the boundaries, and nothing the model says grants access.

### Knowledge tools

`llm/tools/knowledge.rs` exposes four tools over `services.knowledge_search` and `services.notes`:

- `knowledge_search { query, filter_types?, limit? }` returns `{"mode": "hybrid"|"lexical", "results": [...]}`. `lexical` means the embedder was unavailable and only keyword ranking took part.
- `knowledge_create_note { title, content, note_type?, tags? }`. `note_type` is one of `note`, `idea`, `fact`, `observation`, `decision` (default `note`); any other value is an error naming it. The note's `source_channel` comes from the conversation the call runs in (`mcp`, the chat platform, or `web`), never from the model, because the column only accepts those values.
- `knowledge_get_note { id }` returns `{id, title, note_type, tags, content, truncated, total_chars}` for the turn's owner. `content` is the Markdown cut to 8,000 characters; `truncated` and `total_chars` say when it was cut. Another account's id is reported exactly like a missing one. The tool is tainted like `knowledge_search`.
- `knowledge_list_tags`.

### Knowledge search and embeddings

Search ranks in two lists and fuses them (`application/knowledge/search.rs`): a **lexical** list (PostgreSQL full text plus trigram/substring matching, which also finds CJK text) and a **vector** list (pgvector cosine distance over `knowledge_embeddings`, best chunk per item). Reciprocal rank fusion (`rrf`, k = 60) merges them, with ties broken by the earlier lexical rank and then the lower item id, so a result list is deterministic. The vector side is optional: the query is embedded under a 3-second timeout, and if the embedder errors or times out the search returns the lexical list alone and reports `mode: "lexical"` (HTTP: `X-Search-Mode`; chat: "(keyword results only)"; LLM tool: the `mode` field). A note is therefore findable the moment it is saved, before any embedding exists.

The embedder is `multilingual-e5-small` (384 dimensions, int8 ONNX, run by `matrix-bot-inference`). E5 models are asymmetric, so the backend prefixes text before embedding: `query: ` for a search query and `passage: ` for every stored chunk (`embedding_prefixes`); the inference service adds nothing. Notes are chunked from the title plus `search_text` (500 characters, 100 overlap, sent to the embedder in slices of 64), not from raw Markdown. With no `EMBEDDING_BASE_URL` the embedder is unavailable: nothing is indexed, no fake vectors are stored, and search stays keyword-only. `EMBEDDING_MODEL` names the model; chunks are tagged with it and only chunks of the configured model are searched, so changing it re-embeds everything in the background while lexical search covers the gap. Vectors from the old placeholder embedder (a byte-folding hash, not a model) are meaningless and must be re-embedded the same way.

### Web search

`llm/search.rs` searches Tavily first and falls back to keyless DuckDuckGo scraping when no key is set or the API errors. `should_enable_web_search()` gates it by intent (fact-checking, verification, live information, or a quoted claim to verify) and skips routine productivity commands, so "add a todo" never pays for a search. A URL built from user input needs an SSRF guard (`infrastructure/web/ssrf.rs`).

## Chat commands

`!ai` / `!ask <prompt>`, `!ai reset`, `!ai model [name]`, `!ai config`, `!ai retro` (weekly retrospective). `!mcp` and its token subcommands are below.

## MCP server

A built-in Model Context Protocol server (protocol `2024-11-05`, `interface/mcp/protocol.rs`) lets Claude Desktop, Cursor and other clients use the same capabilities over JSON-RPC 2.0. It adds no business logic: `interface/mcp/service.rs` routes `tools/call` into the shared tool registry, which calls application services.

| Transport | Entry | Auth |
|---|---|---|
| stdio (local subprocess) | `matrix-bot mcp --owner <acct_…>` | the account id given on the command line |
| HTTP/SSE | `GET /api/v1/mcp/sse` + `POST /api/v1/mcp/messages`, or single-endpoint `POST /api/v1/mcp` | `Authorization: Bearer mb_mcp_…` personal access token |

- **`--owner` is required** and must parse as an account id (`acct_<uuid>`). MCP mode never falls back to "the oldest account": that would make an authorisation decision by accident. In stdio mode tracing goes to `stderr` so stdout stays a clean protocol channel; the process starts neither front door and ends at stdin EOF.
- **Tokens** (`mb_mcp_…`) are created with `!mcp token [name]` (DM only) or in the web UI (Settings → MCP; `GET/POST /api/v1/mcp/tokens`, `DELETE /api/v1/mcp/tokens/:id`), listed with `!mcp list` and revoked with `!mcp revoke <id_or_prefix>`. A token is scoped to its account and may carry an expiry. **An MCP token never satisfies step-up** (`VerifiedUser`): it is a machine credential, not a passkey assertion.
- **Tools** are the shared registry, so every tool is owner-scoped and every mutation publishes its `SystemEvent` through the service, updating Matrix and web clients live.
- **Resources** (`resources/list`, `resources/read`) give read-only JSON without spending tool calls: `matrixbot://todos/pending`, `matrixbot://finance/recent`, `matrixbot://portfolio/summary`, `matrixbot://habits/today`, `matrixbot://life/profile`.
- **Prompts** (`prompts/list`, `prompts/get`): `daily-briefing`, `weekly-retrospective`, `portfolio-review`.

Local stdio configuration for a client (substitute the account id):

```json
{
  "mcpServers": {
    "matrix-bot": {
      "command": "/usr/local/bin/matrix-bot",
      "args": ["mcp", "--owner", "acct_0123456789abcdef0123456789abcdef"],
      "env": { "DATABASE_URL": "postgres://user:pass@localhost:5432/matrix_bot" }
    }
  }
}
```

Remote clients (Claude Desktop, Cursor's `.cursor/mcp.json`) use `{"url": "https://<host>/api/v1/mcp/sse", "headers": {"Authorization": "Bearer mb_mcp_…"}}`.

## Verifying AI and agent changes

Beyond the standard gates in `AGENTS.md`, an AI or agent change is tested against malformed model output, tool failure, provider timeout, cancellation, round limits, duplicate tool calls, cross-account access attempts, and key redaction in logs and responses. Provider calls in tests hit a local mock server; agents never contact a real LLM, homeserver or search API.
