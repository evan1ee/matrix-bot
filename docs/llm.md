# LLM, agents and MCP

How the AI assistant, its tools and the MCP server are built. The governing rule: **the LLM is an untrusted orchestration component, not a source of truth.** Its output reaches application state only through a validated, independently authorized tool call into a deterministic application service, never through a direct mutation. Layers and dependency direction are in [`architecture.md`](architecture.md); the wire surface is in [`api.md`](api.md).

```text
Matrix DM / mention ─┐
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
| Tool definitions and executors, one file per domain | `llm/tools/<domain>.rs`; `definitions.rs` aggregates, `executor.rs` dispatches |
| Web search (Tavily, DuckDuckGo fallback) and the search-intent gate | `llm/search.rs` |
| Effective per-user configuration (env defaults merged with saved settings) | `llm/resolved_config.rs` |
| LLM HTTP transport and wire types (`ChatCompletionRequest`, `ToolDefinition`, …) | `infrastructure/inference/llm/{client,types}.rs` |
| Conversations, messages, user config (use cases, events) | `application/llm`, `domain/llm`, `infrastructure/postgres/repositories/llm.rs` |
| Matrix front door (`!ai`, DMs, mentions) | `interface/chat/llm.rs` |
| Web front door (`/ai/*`, `/account/ai-config`) | `interface/http/ai.rs` |
| MCP server | `interface/mcp/`, `interface/http/mcp.rs`, `interface/chat/mcp.rs`, `domain/mcp`, `infrastructure/postgres/repositories/mcp.rs` |

## Transport and configuration

`infrastructure/inference/llm` speaks any OpenAI-compatible chat-completions API (`/v1/chat/completions`, streaming or not) reached by **base URL alone**. There is deliberately no per-vendor provider enum and no vendor SDK: the base URL already reaches xAI, OpenAI, Groq, DeepSeek, OpenRouter, Ollama and vLLM. Everything that is the bot's own business (prompts, the tool loop, conversation storage, config merging) lives in `llm/` and `interface/chat/llm.rs`; the transport receives only `ResolvedAiConfig::endpoint()`. Text-to-speech is the one place a dialect exists (ElevenLabs); see [`architecture.md`](architecture.md#voice-replies-say).

- Settings read exactly one env name each: `AI_ENDPOINT` (base URL including `/v1`), `AI_API_KEY`, `AI_MODEL`; web search reads `TAVILY_API_KEY`.
- The AI transport's proxy chain is `AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy` → `HTTP_PROXY`/`http_proxy`. The web-search client has a separate, shorter chain (`AI_PROXY` → `HTTPS_PROXY`/`https_proxy` → `ALL_PROXY`/`all_proxy`, no `HTTP_PROXY`). Do not assume they share a list.
- A user may override endpoint, key, model, system prompt, temperature and token limit (`ai_user_configs`; `GET/PUT /account/ai-config`). A per-user base URL is an SSRF surface: one that resolves to a private host is refused unless `AI_ALLOW_PRIVATE_BASE_URL` is on (off by default). API keys are redacted in `GET` responses and never logged.
- Every external call has a timeout; a provider's types never enter the domain; a missing or unreachable provider degrades the AI features only, and the bot still boots.

## The turn

`llm::run_turn` runs one user turn for an `LlmTurnContext` (services, pool, env config, account, room and `StageReporter`) and a prompt:

1. **Trigger.** In a DM, any message no command module claims goes to the AI (`LlmModule` registers after the TTS module for that reason). In a shared room it triggers on `!ai <prompt>` or a mention. A quoted reply carries its text into the prompt through `QuotedMessage::to_context_prompt()`.
2. **Context.** The active conversation for `(owner, room)` is loaded; one idle longer than `SESSION_TTL_SECONDS` (1 h) is replaced by a fresh one, and `!ai reset` clears it. Immediate retries are deduped by checking the conversation's tail message before inserting.
3. **Tool loop.** The model may request tools; each round executes them, appends `tool` messages and calls the model again. Concurrent calls in a round run through `futures_util::future::join_all`. The loop stops after `MAX_TOOL_ROUNDS` (5); the Matrix front door rejects prompts over `MAX_PROMPT_LENGTH` (16384 bytes).
4. **Reply.** Matrix replies go out as `text_html(&plain, &html)`. When a `StageReporter` is present the reply streams as in-place message updates and falls back to a plain completion on any transport error. The web endpoint is `POST /ai/chat`. Failures reach the user through `interface::chat::error_reply` and never show a provider or database error.

### Agent rules

An agent may reason about a task, select tools, call several, iterate on results and coordinate capabilities. It must not bypass application boundaries. Every loop has explicit limits: iterations (`MAX_TOOL_ROUNDS`), a timeout, a bound on concurrent tool calls, error handling and cancellation. Do not introduce an agent framework where a controlled loop suffices, and keep agent orchestration out of domain services. Long research requests need the generous router timeout (`TimeoutLayer` ≥ 180 s).

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
