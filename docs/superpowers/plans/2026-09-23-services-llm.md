# Extract `services/llm` — Implementation Plan (Plan 4 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the bot's LLM HTTP transport and the OpenAI chat-protocol wire types out of `modules/ai` into a new `services/llm`. `modules/ai` stays the business module and is **not** renamed. The AI must behave identically, proven by characterization tests and by the same live Grok tests before and after.

**Architecture:** `modules/ai/client.rs` mixes two concerns. It holds the transport (`AiClient`: chat completion, SSE streaming, key verification), and it holds `ResolvedAiConfig`, the business rule that merges env defaults with a user's saved AI settings. `modules/ai/models.rs` likewise mixes OpenAI wire types with business records.

This plan splits them along the spec's boundary. `modules/` is what the application does; `services/` is how external capabilities are reached; nothing in `services/` imports from `modules/`.

- The transport and wire types move to `services/llm`.
- `ResolvedAiConfig` stays in `modules/ai`, in its own file.
- It hands the transport a borrowed `LlmEndpoint` (base URL + key) instead of itself.

**Tech Stack:** Rust, reqwest 0.11, axum 0.7 (test servers), tokio, serde.

**Spec:** `docs/superpowers/specs/2026-09-22-inference-service-design.md`, sections "The modules / services boundary" and Non-goals: *"`services/llm` is a **move**, not a redesign"*, and *"`ai` should not become `llm`"*. A per-vendor provider enum is explicitly out of scope.

**Branch:** `refactor/services-llm`, from Gitea `master` (`432c381`). It is independent of the OCR and STT branches, so it can merge on its own.

## Global Constraints

- **`modules/ai` is not renamed and keeps all business logic:** the conversation and tool loop, prompts, per-user config resolution, repository, tools and web search. Only the transport and the wire types move.
- **`services/` never imports from `modules/`.**
- **Behaviour is unchanged.** The same requests go to the same URLs with the same headers and bodies, with the same retries and timeouts. The Task 1 characterization tests prove it: they are written against the current code, then move with it unchanged in substance.
- **No compatibility re-exports.** When a type moves, every importer is updated to the new path. `CLAUDE.md`: "Remove obsolete paths instead of adding compatibility layers".
- **Every commit compiles.** Run `cargo check --workspace --all-targets` before each commit. It must be warning-free.
- **Secrets never reach output.** The live tests read the Grok key from `.env` through `dotenvy` inside the test process. Never run `env`, `printenv`, `cat .env` or `docker compose config`, and never print a key. A type that holds a key must not derive `Debug`.
- `backend/openapi.json` must not change: none of the moved types is a `ToSchema`. Verify with `cargo run --bin generate-openapi && git diff --ignore-all-space --exit-code backend/openapi.json`, then `git checkout -- backend/openapi.json` to discard the CRLF rewrite.
- `cargo fmt --all -- --check` must be clean.
- No source file over 500 production lines, counted above `#[cfg(test)]`. `modules/ai/handler.rs` is already over the limit; this plan touches only its imports and must not grow it.
- Never call `sqlx::query*` directly.
- Commit types `feat` / `fix` / `refactor` / `docs` / `test` / `chore`. Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Three tests in `backend/src/modules/ai/handler.rs` race on process-global state. They are `test_ai_error_suppressed_from_chat_and_sent_once`, `test_ai_quoted_context_prompt_routing` and `test_ai_mention_natural_language_routing`. The issue is pre-existing and tracked separately; do not fix it here. Ignore their failures; any other failure is real.

## Live baseline

The five live Grok tests were run on untouched `master` before any change, and the result is recorded in the ledger:

- `modules::ai::client::tests::test_live_xai_verify_and_completion`
- the four `modules::ai::handler::tests::test_live_xai_grok_*`

Task 4 re-runs them. The after-result must match the baseline test for test.

## File Structure

| Path | After this plan |
|---|---|
| `backend/src/services/mod.rs` | **New.** Boundary doc comment; `pub mod llm;`; `#[cfg(test)] mod test_support;` |
| `backend/src/services/test_support.rs` | **New.** Byte-identical copy of the STT branch's file (`spawn_server`), so the branches merge cleanly. |
| `backend/src/services/llm/mod.rs` | **New.** Module doc and re-exports: `pub use client::{shared_client, LlmClient, LlmEndpoint, LlmError, StreamedCompletion};` and `pub use types::*;` |
| `backend/src/services/llm/types.rs` | **New.** The 13 OpenAI wire types, moved verbatim from `modules/ai/models.rs`. |
| `backend/src/services/llm/client.rs` | **New.** The transport, moved from `modules/ai/client.rs` and renamed. |
| `backend/src/modules/ai/resolved_config.rs` | **New.** `ResolvedAiConfig`, moved from `modules/ai/client.rs`, plus `endpoint()`. |
| `backend/src/modules/ai/client.rs` | **Deleted.** |
| `backend/src/modules/ai/models.rs` | Loses the wire types; keeps every business type. |
| `backend/src/lib.rs` | `pub mod services;` |
| Importers | `api/ai.rs`, `modules/ai/{mod,handler,stream,config_cmd,prompt,search}.rs`, all 13 `modules/ai/tools/**/*.rs` files, `modules/mcp/adapter.rs` if it names a wire type. |

---

## Task 1: Pin the transport's behaviour with characterization tests

Write tests against the **current** `AiClient`, in place, before anything moves. They must pass now. Tasks 2 and 3 carry them along, and their staying green is what proves "no behaviour change".

**Files:**
- Modify: `backend/src/modules/ai/client.rs` (test module only)

**Interfaces:**
- Consumes: the current `AiClient::new(Option<&str>)`, `chat_completion(&self, &ResolvedAiConfig, &ChatCompletionRequest)`, `chat_completion_stream(&self, &ResolvedAiConfig, &ChatCompletionRequest, on_delta)`, `verify_api_key(&self, &str, Option<&str>, Option<&str>)`, `AiClientError::Api { status, message }`.
- Produces: five tests that Task 3 moves.

- [ ] **Step 1: Add the test helpers**

Inside `mod tests` in `backend/src/modules/ai/client.rs`, add below the existing `use` lines:

```rust
    use axum::{http::HeaderMap, routing::{get, post}, Json, Router};
    use std::sync::{Arc, Mutex};

    /// A throwaway server on an ephemeral port; returns its base URL.
    async fn spawn_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("ephemeral port should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have an address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{}", addr)
    }

    fn user_message(text: &str) -> ChatCompletionMessage {
        ChatCompletionMessage {
            role: "user".to_string(),
            content: Some(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn request(model: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![user_message("ping")],
            temperature: Some(0.5),
            max_tokens: Some(16),
            tools: None,
            tool_choice: None,
            stream: None,
        }
    }

    fn config_for(base: String, key: Option<&str>) -> ResolvedAiConfig {
        ResolvedAiConfig {
            api_base_url: base,
            api_key: key.map(str::to_owned),
            model_name: "test-model".to_string(),
            system_prompt: None,
            temperature: 0.5,
            max_tokens: 16,
        }
    }

    /// What the fake provider received: the Authorization header and the JSON body.
    type Seen = Arc<Mutex<Option<(Option<String>, serde_json::Value)>>>;

    fn record(seen: &Seen, headers: &HeaderMap, body: serde_json::Value) {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        *seen.lock().unwrap() = Some((auth, body));
    }
```

`ChatCompletionRequest` is not yet imported by the test module. Add it to the existing `use super::super::models::ChatCompletionMessage;` line.

- [ ] **Step 2: Write the five tests**

```rust
    #[tokio::test]
    async fn test_chat_completion_posts_to_chat_completions_with_bearer_and_body() {
        let seen: Seen = Arc::default();
        let app = Router::new().route(
            "/chat/completions",
            post({
                let seen = seen.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        record(&seen, &headers, body);
                        Json(serde_json::json!({
                            "id": "c1", "model": "test-model",
                            "choices": [{
                                "index": 0,
                                "message": { "role": "assistant", "content": "pong" },
                                "finish_reason": "stop"
                            }]
                        }))
                    }
                }
            }),
        );
        let base = spawn_server(app).await;

        let response = AiClient::new(None)
            .chat_completion(&config_for(base, Some("sk-test")), &request("test-model"))
            .await
            .expect("completion should succeed");

        assert_eq!(response.choices[0].message.content.as_deref(), Some("pong"));
        let (auth, body) = seen.lock().unwrap().clone().expect("the provider was called");
        assert_eq!(auth.as_deref(), Some("Bearer sk-test"));
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["content"], "ping");
        assert!(body.get("stream").is_none(), "an unset stream flag is omitted");
    }

    #[tokio::test]
    async fn test_chat_completion_non_success_is_an_api_error_with_status_and_body() {
        let app = Router::new().route(
            "/chat/completions",
            post(|| async { (axum::http::StatusCode::TOO_MANY_REQUESTS, "rate limited") }),
        );
        let base = spawn_server(app).await;

        let err = AiClient::new(None)
            .chat_completion(&config_for(base, None), &request("test-model"))
            .await
            .expect_err("a 429 is an error");

        match err {
            AiClientError::Api { status, message } => {
                assert_eq!(status, 429);
                assert!(message.contains("rate limited"));
            }
            other => panic!("expected an API error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_chat_completion_stream_assembles_text_and_tool_calls() {
        const SSE: &str = concat!(
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"add_\",\"arguments\":\"{\\\"a\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"todo\",\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let seen: Seen = Arc::default();
        let app = Router::new().route(
            "/chat/completions",
            post({
                let seen = seen.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        record(&seen, &headers, body);
                        ([("content-type", "text/event-stream")], SSE)
                    }
                }
            }),
        );
        let base = spawn_server(app).await;
        let deltas: Arc<Mutex<Vec<String>>> = Arc::default();

        let result = AiClient::new(None)
            .chat_completion_stream(&config_for(base, None), &request("test-model"), |d| {
                let deltas = deltas.clone();
                async move {
                    deltas.lock().unwrap().push(d);
                }
            })
            .await
            .expect("stream should succeed");

        assert_eq!(*deltas.lock().unwrap(), vec!["Hel".to_string(), "lo".to_string()]);
        assert_eq!(result.content, "Hello");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_1");
        assert_eq!(result.tool_calls[0].function.name, "add_todo");
        assert_eq!(result.tool_calls[0].function.arguments, "{\"a\":1}");
        let (_, body) = seen.lock().unwrap().clone().expect("the provider was called");
        assert_eq!(body["stream"], true, "streaming forces stream=true");
    }

    #[tokio::test]
    async fn test_verify_api_key_accepts_a_200_from_models() {
        let app = Router::new().route("/models", get(|| async { "[]" }));
        let base = spawn_server(app).await;

        let result = AiClient::new(None)
            .verify_api_key(&base, Some("sk-test"), Some("test-model"))
            .await;

        assert!(result.is_ok(), "{:?}", result);
    }

    #[tokio::test]
    async fn test_verify_api_key_rejects_a_401_as_invalid_key() {
        let app = Router::new().route(
            "/models",
            get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "bad key") }),
        );
        let base = spawn_server(app).await;

        let err = AiClient::new(None)
            .verify_api_key(&base, Some("sk-wrong"), Some("test-model"))
            .await
            .expect_err("a 401 is an invalid key");

        assert!(err.starts_with("Invalid API key (HTTP 401)"), "{}", err);
    }
```

- [ ] **Step 3: Run them. They must pass against the unchanged code.**

Run: `cargo test -p matrix-bot modules::ai::client`

Expected: PASS for all the tests, old and new.

A failure here means a test misdescribes current behaviour, so fix the **test**, not the client. This task changes no production code. Report any test you had to adjust and why.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add backend/src/modules/ai/client.rs
git commit -m "test: pin the LLM transport's behaviour before moving it

Characterization tests against the unchanged AiClient: the completion URL,
bearer header and body; a non-2xx becoming an Api error with its status;
SSE streaming assembling text deltas and a tool call split across chunks;
and key verification against /models. They travel with the transport when it
moves to services/llm, and their staying green is the proof that the move
changed nothing.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 2: Move the OpenAI wire types to `services/llm`

**Files:**
- Create: `backend/src/services/mod.rs`, `backend/src/services/llm/mod.rs`, `backend/src/services/llm/types.rs`
- Modify: `backend/src/lib.rs`, `backend/src/modules/ai/models.rs`, and every importer of a moved type

**Interfaces:**
- Produces: `crate::services::llm::{ChatCompletionRequest, ChatCompletionMessage, ToolDefinition, FunctionDefinition, ToolCall, FunctionCall, ChatCompletionResponse, ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionDelta, ToolCallChunk, FunctionCallChunk}`. The types are unchanged, including `ToolDefinition::function`.

- [ ] **Step 1: Create the services layer**

`backend/src/services/mod.rs`:

```rust
//! Adapters for capabilities provided outside this process.
//!
//! `modules/` is what the application does; `services/` is how external
//! capabilities are reached. Nothing in here may import from `modules/`.

pub mod llm;
```

`backend/src/services/llm/mod.rs`:

```rust
//! The LLM transport: any OpenAI-compatible chat-completions API (xAI, OpenAI,
//! Ollama, vLLM, OpenRouter...), reached by base URL alone.
//!
//! Only the wire protocol lives here. Which model a user gets, their saved
//! settings, prompts and the tool loop are `modules::ai`'s business.

pub mod types;

pub use types::*;
```

Add `pub mod services;` to `backend/src/lib.rs`, keeping the existing declarations in alphabetical order.

- [ ] **Step 2: Move the types**

Cut the whole block of wire types out of `backend/src/modules/ai/models.rs`. It starts at the `// OpenAI Protocol Payload Structures` banner and runs from `ChatCompletionRequest` through `FunctionCallChunk`, including `impl ToolDefinition`. Paste it unchanged into `backend/src/services/llm/types.rs`, headed by:

```rust
//! OpenAI chat-completions wire types, shared by every compatible provider.

use serde::{Deserialize, Serialize};
```

Leave every other type in `models.rs` where it is. Remove any import in `models.rs` that becomes unused.

- [ ] **Step 3: Update every importer**

List them:

```bash
grep -rnE "ChatCompletion|ToolDefinition|FunctionDefinition|ToolCall|FunctionCall" backend/src --include=*.rs | grep -E "use |models::"
```

Rewrite each import of a moved type from `crate::modules::ai::models::X` or `super::models::X` to `crate::services::llm::X`, including inline paths such as `super::models::ToolCall` in `modules/ai/client.rs`. A file that imports both moved and staying types from `models` gets two `use` lines. Add no re-export in `models.rs`.

- [ ] **Step 4: Verify**

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test -p matrix-bot modules::ai
```

Expected: warning-free; every `modules::ai` test passes, including Task 1's five.

Also confirm that `grep -rn "services" backend/src/services` shows no `crate::modules` import.

- [ ] **Step 5: Commit**

```bash
git add -A backend/src
git commit -m "refactor: move the OpenAI wire types to services/llm

ChatCompletionRequest and its family are the chat-completions protocol, not
the AI module's business, so they move to the new services layer. models.rs
keeps what is: the per-user AI config row, conversation records and API DTOs.
Every importer points at the new path; there is no re-export left behind.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 3: Move the transport to `services/llm`, and keep config resolution in `modules/ai`

**Files:**
- Create: `backend/src/services/llm/client.rs`, `backend/src/services/test_support.rs`, `backend/src/modules/ai/resolved_config.rs`
- Delete: `backend/src/modules/ai/client.rs`
- Modify: `backend/src/services/{mod,llm/mod}.rs`, `backend/src/modules/ai/{mod,handler,stream,config_cmd,prompt,search}.rs`, `backend/src/api/ai.rs`

**Interfaces:**
- Produces:
  - `services::llm::LlmClient`, renamed from `AiClient`.
  - `LlmClient::new(proxy: Option<&str>) -> LlmClient`.
  - `chat_completion(&self, endpoint: LlmEndpoint<'_>, request: &ChatCompletionRequest) -> Result<ChatCompletionResponse, LlmError>`.
  - `chat_completion_stream(&self, endpoint: LlmEndpoint<'_>, request: &ChatCompletionRequest, on_delta: F) -> Result<StreamedCompletion, LlmError>`.
  - `verify_api_key(&self, base_url: &str, api_key: Option<&str>, model: Option<&str>) -> Result<(), String>`. Unchanged.
  - `services::llm::LlmEndpoint<'a> { pub base_url: &'a str, pub api_key: Option<&'a str> }`. Derives `Clone, Copy` only, **no `Debug`**: it holds a key.
  - `services::llm::LlmError`, renamed from `AiClientError`, with the same variants.
  - `services::llm::StreamedCompletion { content: String, tool_calls: Vec<ToolCall> }`, renamed from `StreamTurnResult`.
  - `services::llm::shared_client(proxy: Option<&str>) -> LlmClient`, renamed from `get_ai_client`, with the same `OnceLock` behaviour outside tests.
  - `modules::ai::resolved_config::ResolvedAiConfig`, unchanged, plus `pub fn endpoint(&self) -> LlmEndpoint<'_>`.

- [ ] **Step 1: Split the file**

```bash
git mv backend/src/modules/ai/client.rs backend/src/services/llm/client.rs
```

Then cut `ResolvedAiConfig` and its whole `impl ResolvedAiConfig` block out of `services/llm/client.rs`. Paste them into a new `backend/src/modules/ai/resolved_config.rs`, headed by:

```rust
//! Which LLM a request goes to: the env defaults, overridden per user by their
//! saved AI settings. Business logic, so it lives here and hands the transport
//! only the connection half (`endpoint`).

use crate::services::llm::LlmEndpoint;

use super::models::AiUserConfig;
```

Add to `impl ResolvedAiConfig`:

```rust
    /// The connection half of this config, for `services::llm`.
    pub fn endpoint(&self) -> LlmEndpoint<'_> {
        LlmEndpoint {
            base_url: &self.api_base_url,
            api_key: self.api_key.as_deref(),
        }
    }
```

Move the three `ResolvedAiConfig` unit tests (`test_resolved_config_defaults`, `test_resolved_config_user_override`, `test_temperature_and_token_clamping`) and the live test `test_live_xai_verify_and_completion` into a `#[cfg(test)] mod tests` in `resolved_config.rs`. The live test exercises config resolution plus the transport, and it belongs beside the resolution. In it, `AiClient::new` becomes `crate::services::llm::LlmClient::new`, and `.chat_completion(&config, &req)` becomes `.chat_completion(config.endpoint(), &req)`.

In `backend/src/modules/ai/mod.rs`, replace `pub mod client;` with `pub mod resolved_config;`.

- [ ] **Step 2: Reshape the transport**

In `backend/src/services/llm/client.rs`:

- Rename `AiClient` → `LlmClient`, `AiClientError` → `LlmError`, `StreamTurnResult` → `StreamedCompletion`, `get_ai_client` → `shared_client`, `AI_CLIENT` → `LLM_CLIENT`.
- Add, above `LlmClient`:

```rust
/// Where a request goes and how it authenticates. Borrowed from the caller's
/// config for the length of one call. Deliberately not `Debug`: it holds a key.
#[derive(Clone, Copy)]
pub struct LlmEndpoint<'a> {
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
}
```

- `chat_completion` and `chat_completion_stream` take `endpoint: LlmEndpoint<'_>` in place of `config: &ResolvedAiConfig`. Inside, `config.api_base_url` becomes `endpoint.base_url` and `config.api_key` becomes `endpoint.api_key`. Adjust the `if let Some(ref key)` pattern for a `Option<&str>`. Nothing else in either method changes: same URL, headers, retries, timeouts, logging and parsing.
- `shared_client` takes `proxy: Option<&str>` instead of `&AiEnvConfig`. Both the `#[cfg(not(test))]` and `#[cfg(test)]` versions change the same way. It passes `proxy` straight to `LlmClient::new`.
- The `info!` in `new` says "LLM client configured with proxy".
- Delete `parse_sse_line` and its test `test_parse_sse_line`. It has no production caller; the streaming code uses `parse_sse_chunk`. Make `parse_sse_chunk` private (`fn`); its test stays in the same module.
- Replace every `super::models::` path with `super::types::`. The file must not import from `crate::modules` or `crate::config`.

In `backend/src/services/llm/mod.rs`, add `pub mod client;` and `pub use client::{shared_client, LlmClient, LlmEndpoint, LlmError, StreamedCompletion};`.

- [ ] **Step 3: Move the characterization tests onto the shared helper**

Create `backend/src/services/test_support.rs` as a **byte-identical** copy of the STT branch's file, so the two branches merge without conflict:

```bash
git show refactor/inference-stt:backend/src/services/test_support.rs > backend/src/services/test_support.rs
```

Add `#[cfg(test)] mod test_support;` below `pub mod llm;` in `services/mod.rs`.

In `services/llm/client.rs`'s test module, delete the local `spawn_server` and `config_for`, and add `use crate::services::test_support::spawn_server;`. Replace each `&config_for(base, key)` argument with `LlmEndpoint { base_url: &base, api_key: key }`, and rename `AiClient` → `LlmClient` and `AiClientError` → `LlmError`. The five tests are otherwise unchanged.

- [ ] **Step 4: Update the callers**

Find them:

```bash
grep -rnE "ai::client|super::client|get_ai_client|AiClient|StreamTurnResult|ResolvedAiConfig" backend/src --include=*.rs
```

- `ResolvedAiConfig` imports become `crate::modules::ai::resolved_config::ResolvedAiConfig`, or `super::resolved_config::ResolvedAiConfig` inside `modules/ai`.
- `modules/ai/handler.rs`: delete `pub use super::client::get_ai_client;`. Where it builds a client (`get_ai_client(ai)`), use `crate::services::llm::shared_client(ai.proxy.as_deref())`.
- `api/ai.rs`: drop `get_ai_client` from its `handler::{...}` import, and call `crate::services::llm::shared_client(state.ai.proxy.as_deref()).verify_api_key(...)`.
- `modules/ai/stream.rs`: use `LlmClient` and `StreamedCompletion` from `crate::services::llm`. Wherever it passes a `&ResolvedAiConfig` to `chat_completion` or `chat_completion_stream`, pass `config.endpoint()` instead.

- [ ] **Step 5: Add a live streaming test**

The four live handler tests exercise the full turn. The streaming transport should also be proven live on its own. In `resolved_config.rs`'s test module, beside the existing live test, add:

```rust
    #[ignore]
    #[tokio::test]
    async fn test_live_xai_streaming_completion() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::dotenv();
        let ai_env = crate::config::AiEnvConfig::from_env();
        let config = ResolvedAiConfig::from_user_config(&ai_env, None);
        if config.api_key.as_deref().map_or(true, |k| k.trim().is_empty()) {
            return; // no live key configured
        }

        let request = crate::services::llm::ChatCompletionRequest {
            model: config.model_name.clone(),
            messages: vec![crate::services::llm::ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Count from one to five in words.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            temperature: Some(0.0),
            max_tokens: Some(60),
            tools: None,
            tool_choice: None,
            stream: None,
        };
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(0usize));

        let result = crate::services::llm::LlmClient::new(ai_env.proxy.as_deref())
            .chat_completion_stream(config.endpoint(), &request, |_| {
                let deltas = deltas.clone();
                async move {
                    *deltas.lock().unwrap() += 1;
                }
            })
            .await
            .expect("live streaming completion should succeed");

        assert!(!result.content.trim().is_empty());
        assert!(*deltas.lock().unwrap() > 1, "a streamed reply arrives in several deltas");
    }
```

- [ ] **Step 6: Verify**

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test -p matrix-bot
grep -rn "crate::modules\|crate::config" backend/src/services || echo "services imports nothing from modules or config"
cargo run --bin generate-openapi && git diff --ignore-all-space --exit-code backend/openapi.json
git checkout -- backend/openapi.json
```

Expected:
- `cargo check` is warning-free.
- The full suite passes, apart from the known flaky three.
- Task 1's five characterization tests pass from their new home.
- `services` imports nothing from `crate::modules` or `crate::config`. In tests it may use `crate::services::test_support`.
- `openapi.json` is unchanged.

- [ ] **Step 7: Commit**

```bash
git add -A backend/src
git commit -m "refactor: move the LLM transport to services/llm

AiClient becomes services::llm::LlmClient. It now takes a borrowed LlmEndpoint
(base URL and key) instead of the AI module's ResolvedAiConfig, so the
transport knows nothing about per-user settings. ResolvedAiConfig -- the rule
that merges env defaults with a user's saved AI config -- stays in modules/ai,
in its own file, and hands the transport only endpoint().

modules/ai is not renamed and keeps all of its business logic. The
characterization tests moved with the transport unchanged in substance, and
still pass. Drops parse_sse_line, which only its own test called.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 4: Document the boundary and prove it live

**Files:**
- Modify: `CLAUDE.md`, `AGENTS.md`, `README.md`

- [ ] **Step 1: `CLAUDE.md`**

In "Backend module plugin system", after its first paragraph, add:

```markdown
`backend/src/services/` is where external capabilities are reached;
`modules/` is what the application does, and nothing in `services/` may
import from `modules/`. `services/llm` is the LLM transport: any
OpenAI-compatible chat-completions API reached by base URL alone, plus the
wire types (`ChatCompletionRequest`, `ToolDefinition`, ...). `modules/ai`
keeps everything that is the bot's business -- prompts, the tool loop,
conversation storage, and `resolved_config.rs`, which merges the env defaults
with a user's saved AI settings and hands the transport only its
`endpoint()`. There is deliberately no per-vendor provider enum: base URL
already reaches xAI, OpenAI, Ollama, vLLM and the rest.
```

- [ ] **Step 2: `AGENTS.md` and `README.md`**

In `AGENTS.md`, add to the conventions list:

```markdown
- **`services/` never imports from `modules/`.** `modules/` is what the
  application does; `services/` is how external capabilities are reached. A
  wire type and a domain type that happen to look alike stay separate.
```

In `README.md`'s project-tree section, add `services/llm/` under `backend/src/`, with a one-line description.

- [ ] **Step 3: Re-run the live tests**

```bash
cargo test -p matrix-bot live_xai -- --ignored --test-threads=1
```

This makes real calls to the configured provider (Grok) with the key from `.env`. The tests print whether a key is present, never the key itself. Do not run anything else that could print the environment.

Expected: every test that passed in the baseline (recorded in the dispatch) passes again, now including `test_live_xai_streaming_completion`. Report each test's outcome beside its baseline outcome. A live-model test can fail for reasons outside the code, such as a model declining to call a tool. If one differs from the baseline, re-run it once, and report both runs; don't retry silently.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all -- --check
git add CLAUDE.md AGENTS.md README.md
git commit -m "docs: document the services/llm boundary

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
