# Sub-project 2, Plan B: Discord adapter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** approved 2026-10-02; implemented (Tasks 1–7, HANDOFF §4 Plan B, rulings R85–R101).

**Goal:** Discord becomes the second `ChannelAdapter`. A linked Discord user runs `/todo`, `/expense`, `/portfolio`, `/ask`, `/link` and `/help` (and talks to the agent in a DM) against the same `BotModule` registry, services and account as on Matrix. Reminders and the link confirmation come back to the Discord conversation they started in. An unlinked Discord user can only `/help` and `/link`. Nothing outside `interface/channels/discord/`, the Markdown renderer, configuration and the composition root changes.

**Architecture:**
- **Config** `config/channels.rs` — `DiscordConfig { token, guild_id }`, read from `DISCORD_TOKEN` / `DISCORD_GUILD_ID`. The adapter exists only when `DISCORD_TOKEN` is set.
- **Renderer** `interface/channels/render/markdown.rs` — the module HTML subset → Discord Markdown, pure, table-tested.
- **Adapter** `interface/channels/discord/` — serenity gateway client (intents `GUILDS | DIRECT_MESSAGES`), slash-command registration, interaction deferral, a bounded work queue drained by a fixed worker pool that calls the shared `Inbound` pipeline, reply rendering as embeds, an interaction-editing `ProgressSink`, and `send` for jobs and the outbox.
- **Composition root** `app.rs` — builds the adapter when configured, attaches the shared pipeline, and supervises every adapter's `run` (not only Matrix's).

No migration, no new table, no SQL, no service, no HTTP route, no frontend change: plan A already made identities, linking, the identity list and outbound routing channel-neutral.

**Tech Stack:** Rust, `serenity` 0.12.5 (gateway + HTTP; rustls; shares the workspace's reqwest 0.11), tokio + `tokio_util::sync::CancellationToken`, the plan A channel framework.

**Spec:** `docs/superpowers/specs/2026-10-01-omnichannel-design.md` (§3.2–§5, §8.2, §10–§12). Read it, `AGENTS.md`, and `docs/superpowers/HANDOFF.md` §5, §6, §8 and rulings R70–R84 first. This plan argues from the code plan A produced; it cites real types.

**Worktree:** a separate worktree of branch `feat/omnichannel` (stacked on `refactor/outbox`). Never work in the main checkout.

## Code this plan builds on (read before Task 1)

| Path | What it gives Discord |
|---|---|
| `backend/src/interface/channels/adapter.rs` | `ChannelAdapter` (`channel`, `is_connected`, `run`, `send`, defaulted `authorize_delivery`, `find_conversation_by_name`), `ChannelError { NotConnected(Channel), NotConfigured(Option<Channel>), Rejected(String), Transient(String) }`, `SentRef` |
| `backend/src/interface/channels/message.rs` | `IncomingMessage`, `OutgoingMessage { target, content, reply_to, visibility, idempotency_key, join_if_needed }`, `Target`, `Content`, `Attachment`, `Reply`, `Visibility`, `MessageRef` |
| `backend/src/interface/channels/inbound.rs` | `Inbound::handle(&IncomingMessage, InboundText, Option<Arc<dyn ProgressSink>>) -> InboundOutcome { Replied(Reply), Ignored }`; `link` before identity resolution; `Unlinked` gets only `start`/`help`/`link`; free text → `ai` where addressed; panic fence |
| `backend/src/interface/channels/registry.rs` | `ChannelRegistry::{from_unique, send, spawn_all, any_connected}`, `check_unique` |
| `backend/src/interface/channels/matrix/adapter.rs` | the reference adapter: `OnceLock<Arc<Inbound>>` + `attach_inbound` (A5), error mapping at the boundary |
| `backend/src/interface/channels/render/html.rs` | the HTML-subset sanitizer every renderer starts from |
| `backend/src/interface/chat/link.rs` | `handle_link` already words its refusal `/link` for non-Matrix channels and returns the URL as `Reply::private` |
| `backend/src/llm/progress.rs` | `ProgressSink { report_stage, report_tool_call, push_delta, flush, message_ref }` |
| `backend/src/domain/channels/mod.rs` | `Channel::Discord`, `PlatformUserId::parse(Discord, _)` (non-zero snowflake), `ConversationAddress::Discord(u64)` (storage and destination form `discord:<id>`), `provisioning_policy(Discord) == RequireLink` |
| `backend/src/config/channels.rs` | `parse_link_signup_allowlist` already reads `DISCORD_ALLOWED_USERS` (spec D4) |
| `backend/src/app.rs` | `Arc::new_cyclic` registry construction, `supervised`, `supervise_front_doors` (today Matrix-only) |

## Global Constraints

- **Layers.** `serenity` appears only under `interface/channels/discord/` (new architecture test, Task 6). `application/`, `llm/`, `interface/chat/` and `domain/` stay free of it (existing tests already list `serenity`). `interface/channels/**` runs no SQL. Only `app.rs` constructs `DiscordAdapter`.
- **Frozen behaviour elsewhere.** Every Matrix command, reply, job and outbox behaviour is unchanged. The existing suite passes unchanged in meaning; the only edited existing tests are the `app.rs` supervision tests whose signature changes in Task 6.
- **OpenAPI.** `backend/openapi.json` is byte-identical after every task. No `.sqlx/` change, no migration.
- **No network in tests.** Nothing contacts Discord. Everything platform-shaped is tested through pure functions over serenity model types deserialized from fixtures under `contracts/channels/` or over the plan's own small structs; the serenity glue that performs HTTP calls stays thin and untested by design (AGENTS §8: lowest level that proves the point).
- **Security** (spec §11): no owner key from a Discord id — every interaction goes through `Inbound::handle`, hence `IdentityService::resolve`; an unlinked identity reaches only `start`/`help`/`link`; `/link` is refused outside a DM; a `Private` reply is never shown visibly in a guild channel (invariant 10); every message the bot posts has empty `allowed_mentions`, so user text can never ping `@everyone`, a role or a user. Never log the token, the interaction token or message bodies.
- **Bounds.** Every Discord HTTP call has a timeout; work is queued on a bounded channel and run by a fixed number of workers; reconnects back off exponentially with a cap; every long-lived task holds a child `CancellationToken` and joins `app::run`'s shutdown.
- **File size.** No file in `backend/src/` exceeds 500 production lines; target < 300 per Discord file.
- **Error handling.** No `unwrap()`/`expect()` on runtime paths. serenity errors are mapped to `ChannelError` at the adapter boundary and never escape it.
- **Hard safety rules** (HANDOFF §6): never read `.env` or `data/` of the main checkout, never run the bot binary (only `cargo run --bin generate-openapi`), never contact Discord or any external service from tests, never push/merge/PR without the user's instruction, never bare `git stash`.
- **Commits.** Conventional Commits, one logical change each, ending with the committing model's trailer.

### Iteration discipline and the Gate

Same as plan A: narrow tests while iterating (`cargo test -p matrix-bot interface::channels::discord::`), the full Gate once per task just before its commit:

```bash
export TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
export DATABASE_URL=$TEST_DATABASE_URL
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo test  -p matrix-bot --all-targets
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # must stay byte-identical
git diff --check
```

Task 3 adds a dependency: also run `cargo build -p matrix-bot --release` once there (TLS backends and `links` conflicts show up at link time) and confirm `Cargo.lock` gains no second `reqwest`, `rustls` or `tokio-tungstenite` major line that the release build cannot reconcile.

## Plan clarifications (supersede the spec where they differ; record in HANDOFF as rulings R85+)

- **B1 — `full_body` is the bare form.** Modules parse `ctx.full_body` and accept only `!todo …` or bare `todo …` (`interface/chat/{todos,reminders}/handler.rs`, `finance/handler.rs::strip_command`, `portfolio`, `briefing`, `settings`). A slash command therefore becomes `InboundText::Command { command, raw_args: args, full_body: format!("{command} {args}").trim() }` — no `/`. Plan C (Telegram) must do the same when it strips `/todo@bot`.
- **B2 — guild interactions are ephemeral, except `/help`.** The spec defers ephemerally "when the command's reply could be private". The defer must happen within 3 s, before any module runs, and todos, expenses, portfolio and agent answers (the agent's tools read private data) are all personal, so in a guild every command but `/help` is deferred ephemeral; in a DM nothing is ephemeral (nobody else can read it). As a backstop for invariant 10, a `Visibility::Private` reply that would land on a public guild response is never posted there: the public response is edited to a fixed notice and the content goes out as an ephemeral follow-up (`route_reply`, Task 4).
- **B3 — DM free text goes to the agent.** `DIRECT_MESSAGES` delivers DM content without the privileged `MESSAGE_CONTENT` intent. A DM text message becomes `InboundText::FreeText`, which the pipeline turns into `ai` (spec §5 step 4, as on Matrix). DM text is never parsed as a command (slash commands are the command syntax); guild messages are not received at all (no `GUILD_MESSAGES` intent). Attachments in DMs are ignored (media ingestion is out of scope, spec §14).
- **B4 — embeds, no title, no colour.** Replies are embeds whose description is the Markdown rendering; the spec's "title line" is dropped (modules emit no separate title, and guessing one from the first line would reorder their text), and embeds carry no colour (Design.md is monochrome-first).
- **B5 — `Content.actions` are not rendered on Discord.** Discord buttons are out of scope (spec §14); actions are dropped with a `debug!`. Today no module emits actions (plan C's todo "Done" button is the first).
- **B6 — the adapter holds no `AppServices`.** It needs only its config, its HTTP client and the shared `Inbound` (which holds the services). The Matrix adapter holds services for Matrix-only reasons (room authorization).
- **B7 — gateway failure policy.** serenity reconnects a dropped gateway session itself. `run` rebuilds the client after `Client::start` returns an error, with exponential backoff (1 s doubling, capped at 5 min, reset after a session that stayed up 60 s). A non-retryable gateway error (`InvalidAuthentication`, `InvalidGatewayIntents`, `DisallowedGatewayIntents`) makes `run` return `Err`, and the supervisor stops the process like any other dead front door (a wrong token fails loudly at boot, as an API bind failure does).
- **B8 — bounded work queue.** serenity spawns a task per gateway event. The event handler only parses, defers and `try_send`s onto a bounded `mpsc` (capacity 32); `DISCORD_WORKERS = 4` workers owned by `run` drain it and call the pipeline. A full queue answers "busy" (interactions) or drops with a `warn!` (DMs). On shutdown the workers stop with `run`; an in-flight turn is dropped, as a Matrix turn is.
- **B9 — `ChannelSink::is_ready` stays "any adapter connected" (R81 revisited).** Discord is now a delivery destination (the `IdentityLinked` confirmation). While Discord is down and Matrix up, such a delivery is claimed, fails as not connected and backs off, costing attempts; only the one-line link confirmation is at stake, and narrowing `is_ready` needs the dispatcher to know a delivery's channel before claiming it. Revisit in plan C if Telegram adds a destination with real content.
- **B10 — `/help` shows the shared help text.** `CoreModule` prints commands with Matrix's `!` prefix; on Discord `!todo add x` reads as `/todo args:add x`. A per-channel help format is deferred (logged in HANDOFF §11).
- **B11 — an interaction always gets an answer.** Deferral shows "thinking…" until edited, so `InboundOutcome::Ignored` on an interaction edits the response to `NO_REPLY` ("Nothing to answer for that."). A DM that produces `Ignored` gets nothing, as on Matrix.
- **B12 — `is_mentioned = true` for interactions.** A slash command explicitly addresses the bot; a DM is `is_direct = true, is_mentioned = false`.
- **B13 — supervision covers every adapter.** `supervise_front_doors` takes the adapters' tasks as a list (completes R80(11)); any adapter ending unrequested stops the process, as the Matrix loop's does today.
- **B14 — attachments are files.** Audio, image and file attachments upload as Discord files (TTS replies are not reachable from the six slash commands but are from a DM `say`). A file above `MAX_UPLOAD_BYTES` (10 MiB, Discord's default limit) makes the send `Rejected`.
- **B15 — command options.** Every command has `dm_permission(true)`; `args` is an optional string with `max_length(500)` (the chat input cap); `/ask` has a required `prompt` with `max_length(4000)` (below `llm::turn::MAX_PROMPT_LENGTH`). The adapter re-checks both limits (Discord's limit is a client hint; a crafted interaction can exceed it).
- **B16 — dedupe.** Discord's `nonce` + `enforce_nonce` deduplicates a create within a few minutes. When `OutgoingMessage.idempotency_key` is set and is at most 25 characters (`outbox-<id>` fits), `send` sets both; otherwise none. `join_if_needed` has no meaning on Discord (a bot cannot join a channel) and is ignored.

---

## File map

| File | Responsibility | Task |
|---|---|---|
| `backend/src/config/channels.rs` | `DiscordConfig`, `parse_discord` | 1 |
| `backend/src/config/mod.rs` | `Config.discord`, validation, `for_test` | 1 |
| `backend/src/interface/channels/render/markdown.rs` | HTML subset → Discord Markdown | 2 |
| `backend/Cargo.toml`, `Cargo.lock` | `serenity` dependency | 3 |
| `backend/src/interface/channels/discord/mod.rs` | module docs, `pub mod` lines | 3 |
| `backend/src/interface/channels/discord/commands.rs` | the command table, defer mode, option → `InboundText`, `CreateCommand` glue | 3 |
| `backend/src/interface/channels/discord/inbound.rs` | interaction / DM → `IncomingMessage` (pure) | 3 |
| `contracts/channels/discord_command_interaction.json`, `discord_dm_message.json` | fixtures | 3 |
| `backend/src/interface/channels/discord/render.rs` | `Content` → embed posts, `route_reply` (pure) | 4 |
| `backend/src/interface/channels/discord/outbound.rs` | `send`, error classification, timeouts | 4 |
| `backend/src/interface/channels/discord/progress.rs` | `Throttle`, `InteractionProgress`, `TypingProgress` | 5 |
| `backend/src/interface/channels/discord/gateway.rs` | serenity `EventHandler`: ready, stage, interactions, DMs | 5 |
| `backend/src/interface/channels/discord/worker.rs` | the work queue's workers: pipeline call and reply delivery | 5 |
| `backend/src/interface/channels/discord/adapter.rs` | `DiscordAdapter: ChannelAdapter`, `run`, reconnect backoff | 5 |
| `backend/src/app.rs` | build/attach the adapter; supervise every adapter | 6 |
| `backend/tests/architecture.rs` | `serenity` confined; `DiscordAdapter::new` only in `app.rs` | 6 |
| docs (`architecture.md`, README, `.env.example`, spec, `SPEC.md` §9, HANDOFF) | | 7 |

Each task leaves the workspace compiling and the Gate green at its commit.

---

### Task 1: Discord configuration

**Files:**
- Modify: `backend/src/config/channels.rs`, `backend/src/config/mod.rs`

**Interfaces:**

```rust
// config/channels.rs
#[derive(Clone)]
pub struct DiscordConfig {
    pub token: String,          // DISCORD_TOKEN, trimmed, non-empty
    pub guild_id: Option<u64>,  // DISCORD_GUILD_ID: register commands to one guild (instant) instead of globally
}
impl std::fmt::Debug for DiscordConfig { /* prints guild_id and `token: "<redacted>"` */ }

/// `None` when DISCORD_TOKEN is unset or blank. `Err` when DISCORD_GUILD_ID is
/// set but not a non-zero integer.
pub(super) fn parse_discord(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<DiscordConfig>>;

// config/mod.rs
pub struct Config { /* … */ pub discord: Option<DiscordConfig> }
```

`Config::validate` additionally refuses a `DISCORD_ALLOWED_USERS` entry that `PlatformUserId::parse(Channel::Discord, _)` rejects, naming the entry (a typo would otherwise silently keep that person from signing up). `Config::for_test()` sets `discord: None`.

- [ ] **Step 1: Write failing tests** in `config/channels.rs`:
  - `discord_is_absent_without_a_token` (unset and `"  "` → `Ok(None)`);
  - `discord_reads_token_and_optional_guild` (`" abc "` → token `"abc"`, guild `None`; with `DISCORD_GUILD_ID="123"` → `Some(123)`);
  - `a_malformed_guild_id_is_refused` (`"0"`, `"abc"`, `"-1"` → `Err` naming `DISCORD_GUILD_ID`);
  - `the_discord_token_never_appears_in_debug_output` (`format!("{:?}", cfg)` lacks the token).
  In `config/mod.rs`: `validate_refuses_a_malformed_discord_allowlist_entry`.
- [ ] **Step 2: Run them** (`cargo test -p matrix-bot config::`), see them fail.
- [ ] **Step 3: Implement**; `Config::from_env` calls `channels::parse_discord(|n| env::var(n).ok())?`. `test_only_config_reads_the_environment` keeps passing (all reads stay in `config/`).
- [ ] **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(config): Discord settings (DISCORD_TOKEN, DISCORD_GUILD_ID)`

---

### Task 2: HTML subset → Discord Markdown

**Files:**
- Create: `backend/src/interface/channels/render/markdown.rs`
- Modify: `backend/src/interface/channels/render/mod.rs` (`pub mod markdown;`)

**Interfaces:**

```rust
/// Renders the module HTML subset (`b strong i em u code pre br p ul ol li a blockquote`)
/// as Discord Markdown. Text content is Markdown-escaped, entities are decoded,
/// unknown tags are dropped keeping their text. Never fails.
pub fn html_to_markdown(html: &str) -> String;

/// Escapes Discord Markdown metacharacters (`\ * _ ~ ` | > # - [ ] ( )` and a
/// leading `@`/`<` sequence) in plain text that must render literally.
pub fn escape_markdown(text: &str) -> String;
```

Start from `render/html.rs` (read it first: reuse its tokenizer/sanitizer rather than writing a second HTML scanner; if its tokenizer is private, expose a `pub(super)` token iterator from it — one HTML scanner in the crate).

Mapping: `b/strong` → `**…**`; `i/em` → `*…*`; `u` → `__…__`; `code` → `` `…` `` (content not escaped; a backtick inside switches to double backticks); `pre` → fenced ``` block on its own lines (content not escaped); `br` → `\n`; `p` → paragraph separated by one blank line; `ul > li` → `- item`; `ol > li` → `1. item` numbered in order; `a href` → `[text](url)` when the scheme is `http`/`https`, else just the text; `blockquote` → each line prefixed `> `. Runs of 3+ newlines collapse to 2; result trimmed.

- [ ] **Step 1: Write the failing table test** `renders_the_module_subset` with at least these rows (input → expected): bold/italic/underline; `a<br/>b`; two `<p>`s; `<ul><li>x</li><li>y</li></ul>` → `- x\n- y`; `<ol>` numbering; `<a href="https://x.y/a_b">t</a>` → `[t](https://x.y/a_b)`; `<a href="javascript:alert(1)">t</a>` → `t`; `<code>a*b</code>` → `` `a*b` ``; `<pre>x\n*y*</pre>` fenced and unescaped; `&lt;b&gt; &amp; *` → `<b> & \*` (decoded, Markdown-escaped, never re-interpreted as a tag); `<script>x</script>` → `x`; `@everyone` → escaped so it renders literally; a real module reply (paste `format_todo_list`'s output for two items from `interface/chat/todos`) renders without stray tags.
- [ ] **Step 2: Run** (`cargo test -p matrix-bot render::markdown`), see it fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(channels): HTML subset to Discord Markdown renderer`

---

### Task 3: serenity, the command table and inbound normalization

**Files:**
- Modify: `backend/Cargo.toml`, `Cargo.lock`, `backend/src/interface/channels/mod.rs` (`pub mod discord;`, module docs list)
- Create: `backend/src/interface/channels/discord/{mod,commands,inbound}.rs`
- Create: `contracts/channels/discord_command_interaction.json`, `contracts/channels/discord_dm_message.json`

**Dependency:**

```toml
serenity = { version = "0.12.5", default-features = false, features = ["builder", "client", "gateway", "http", "model", "rustls_backend", "chrono"] }
```

No `cache`, `framework` or `standard_framework`: the adapter reads everything it needs from the event. Verify after `cargo update -p serenity` that serenity's `reqwest` resolves to the workspace's 0.11 line (one `reqwest` 0.11 in `cargo tree -d`).

**Interfaces:**

```rust
// discord/commands.rs
pub const ARGS_MAX: usize = 500;
pub const PROMPT_MAX: usize = 4000;

pub struct CommandSpec {
    pub name: &'static str,          // the slash command
    pub dispatches_as: &'static str, // the registry command ("ai" for /ask)
    pub description: &'static str,
    pub option: &'static str,        // "args" | "prompt"
    pub required: bool,
    pub max_length: usize,
}

/// `/todo /expense /portfolio /ask /link /help` (spec §8.2).
pub const COMMANDS: &[CommandSpec];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferMode { Public, Ephemeral }

/// B2: in a guild everything but `help` is ephemeral; in a DM nothing is.
pub fn defer_mode(command: &str, in_guild: bool) -> DeferMode;

#[derive(Debug, PartialEq, Eq)]
pub enum CommandRefusal { Unknown(String), TooLong { max: usize }, MissingPrompt }
impl CommandRefusal { pub fn reply_text(&self) -> String; }   // shown ephemerally, never dispatched

/// The slash command and its one option value → the registry's `InboundText` (B1).
pub fn inbound_text(name: &str, value: Option<&str>) -> Result<InboundText, CommandRefusal>;

/// serenity builders for registration (glue; covered by the table test on `COMMANDS`).
pub fn create_commands() -> Vec<serenity::builder::CreateCommand>;

// discord/inbound.rs
pub struct Parsed { pub message: IncomingMessage, pub text: InboundText }

/// A slash command → `IncomingMessage` (conversation `Discord(channel_id)`,
/// `is_direct = guild_id.is_none()`, `is_mentioned = true` (B12),
/// `message_id = MessageRef("interaction:<id>")`, sender = the invoking user).
pub fn from_command(interaction: &CommandInteraction) -> Result<Parsed, CommandRefusal>;

/// A DM text message → free text (B3). `None` for bot authors, guild messages,
/// and messages with no text. `message_id = MessageRef("message:<channel>:<id>")`.
pub fn from_direct_message(message: &Message) -> Option<Parsed>;

/// Display name: guild nickname, else global name, else username.
fn identity(user: &User, member: Option<&Member>) -> PlatformIdentity;
```

`InboundText` must be constructible here; it is already `pub` in `inbound.rs`.

- [ ] **Step 1: Write failing tests.**
  - `commands.rs`: `the_table_matches_the_spec` (names exactly `todo expense portfolio ask link help`; `ask` dispatches as `ai` with a required `prompt` of `PROMPT_MAX`; every other command an optional `args` of `ARGS_MAX`); `every_dispatched_command_is_claimed_by_the_registry` (for each `dispatches_as` other than `link`, `build_modules(&Config::for_test())` + a linked test account dispatching `inbound_text(name, None)` through `Inbound::handle` with a fake-registry returns `Replied`, i.e. B1's bare `full_body` is accepted by the real modules; the account holds no data, so `portfolio` lists nothing and fetches no prices, and `ai` without a prompt answers its help without calling the LLM — reuse `inbound.rs`'s test helpers by making `pipeline_with` and friends `pub(crate)` under `#[cfg(test)]`); `defer_mode_is_ephemeral_in_guilds_except_help` (table: 6 commands × {guild, DM}); `inbound_text_builds_the_bare_full_body` (`("todo", Some("add milk"))` → `Command { command: "todo", raw_args: "add milk", full_body: "todo add milk" }`, `("todo", None)` → `full_body: "todo"`, `("ask", Some("hi"))` → command `ai`); `over_long_and_missing_values_are_refused` (501-char args, 4001-char prompt, `ask` without prompt, unknown name).
  - `inbound.rs`: deserialize `contracts/channels/discord_command_interaction.json` (a guild `/todo args:"add milk"` interaction with a member nickname) and assert every `IncomingMessage` field; the same fixture with `guild_id`/`member` removed (mutate the `serde_json::Value`) is direct and named by the user's `global_name`; `discord_dm_message.json` yields free text, `is_direct`, conversation `Discord(<channel>)`; a bot author or an empty content yields `None`. Load fixtures with `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../contracts/channels/…"))` as `matrix/inbound.rs` does.
  - Write the fixtures from Discord's documented payload shapes (application command interaction type 2; message create), with fake snowflakes; they must deserialize into serenity 0.12.5's `CommandInteraction` / `Message`.
- [ ] **Step 2: Run** (`cargo test -p matrix-bot interface::channels::discord`), see them fail.
- [ ] **Step 3: Implement.** Platform-id validation goes through `PlatformUserId::parse(Channel::Discord, &user.id.to_string())`; a failure (impossible for a real snowflake) is a refusal, never a panic.
- [ ] **Step 4: Gate**, plus `cargo build -p matrix-bot --release` and `cargo tree -d -p matrix-bot` reviewed for new duplicate majors.
- [ ] **Step 5: Commit** — `feat(discord): slash command table and inbound normalization`

---

### Task 4: Outbound rendering and `send`

**Files:**
- Create: `backend/src/interface/channels/discord/{render,outbound}.rs`

**Interfaces:**

```rust
// discord/render.rs
pub const EMBED_DESCRIPTION_MAX: usize = 4096;   // characters
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
pub const PRIVATE_NOTICE: &str = "Sent privately.";
pub const NO_REPLY: &str = "Nothing to answer for that.";

/// One Discord message: one embed description plus, on the last post only, the files.
#[derive(Debug, PartialEq)]
pub struct Post { pub description: Option<String>, pub files: Vec<Attachment> }

/// `Content` → posts: Markdown from `html` (else `escape_markdown(plain)`),
/// split at line boundaries into chunks ≤ EMBED_DESCRIPTION_MAX characters
/// (a single longer line is split on a char boundary; a ``` fence open at a
/// split is closed and reopened); files on the last post. `Rejected` for a file
/// over MAX_UPLOAD_BYTES. Actions are dropped (B5). Empty content with no files
/// yields one post with NO_REPLY.
pub fn posts(content: &Content) -> Result<Vec<Post>, ChannelError>;

#[derive(Debug, PartialEq)]
pub enum Route {
    /// Edit the deferred response with the first post; the rest are follow-ups
    /// with the same ephemerality as the defer.
    EditOriginal,
    /// B2 backstop: edit the public response to PRIVATE_NOTICE and send every post
    /// as an ephemeral follow-up.
    PrivateFollowUp,
}
pub fn route_reply(visibility: Visibility, in_guild: bool, deferred: DeferMode) -> Route;

// discord/outbound.rs
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// `ChannelAdapter::send`: `Conversation(Discord(id))` posts to that channel;
/// `Direct(identity)` on Discord opens (or reuses) the DM channel first; any other
/// target is `Rejected`. `reply_to` = `MessageRef("message:<channel>:<id>")` becomes
/// a message reference (other refs are ignored). Every message sets empty
/// `allowed_mentions`. B16 nonce when the key fits. Each HTTP call is wrapped in
/// SEND_TIMEOUT (`Transient` on expiry). Returns the last message's ref.
pub async fn send(http: &Http, message: OutgoingMessage) -> Result<SentRef, ChannelError>;

/// Builders shared with the reply path: one `Post` → `CreateMessage` /
/// `EditInteractionResponse` / `CreateInteractionResponseFollowup`, each with
/// empty allowed mentions.
pub(super) fn create_message(post: Post) -> CreateMessage;
pub(super) fn edit_response(post: Post) -> EditInteractionResponse;
pub(super) fn follow_up(post: Post, ephemeral: bool) -> CreateInteractionResponseFollowup;

/// HTTP status → error class: 400/401/403/404/413 `Rejected`; 429, 5xx `Transient`.
pub(super) fn classify_status(status: u16, detail: &str) -> ChannelError;
/// serenity's error → `ChannelError` (an unsuccessful request goes through
/// `classify_status`; transport and gateway errors are `Transient`; model/format errors `Rejected`).
pub(super) fn classify(err: &serenity::Error) -> ChannelError;
```

- [ ] **Step 1: Write failing tests** (pure only):
  - `posts`: short HTML → one post with the Markdown; 9 000 characters of lines → three posts, each ≤ 4096 chars, no line split mid-way, concatenation equals the input; one 5 000-char line → split on a char boundary (use multi-byte text such as `字`); a fenced block crossing a split stays fenced in both parts; an audio attachment rides on the last post only; an 11 MiB attachment → `Rejected`; actions are ignored; empty → `[Post { description: Some(NO_REPLY), .. }]`.
  - `route_reply`: the full table (Normal/Private × guild/DM × Public/Ephemeral); only `Private` + guild + `Public` is `PrivateFollowUp` (spec invariant 10).
  - `classify_status`: 400, 403, 404, 413 → `Rejected`; 429, 500, 502 → `Transient`.
  - Builders: serialize `create_message(post)` with `serde_json::to_value` (serenity's builders implement `Serialize`) and assert `allowed_mentions` is present with an empty `parse` list and no `users`/`roles` (pin the exact JSON serenity 0.12.5 emits) and the embed description is the post's; same for `edit_response` and `follow_up(_, true)` (flags carry EPHEMERAL).
  - `send` target validation without a network: a `Telegram` conversation and a `Direct` Matrix identity → `Rejected` before any HTTP call (build `Http::new("x")`; the call must return before using it).
- [ ] **Step 2: Run**, see them fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(discord): embed rendering, private-reply routing and outbound send`

---

### Task 5: `DiscordAdapter` — gateway, work queue, progress, run loop

**Files:**
- Create: `backend/src/interface/channels/discord/{progress,gateway,worker,adapter}.rs`

**Interfaces:**

```rust
// discord/progress.rs
/// At most one edit per `gap` (pure; tested with explicit instants).
pub struct Throttle { gap: Duration, last: Option<Instant> }
impl Throttle { pub fn new(gap: Duration) -> Self; pub fn ready(&mut self, now: Instant) -> bool; }

pub const EDIT_GAP: Duration = Duration::from_millis(1500);
pub const TYPING_GAP: Duration = Duration::from_secs(8);

/// Edits the deferred interaction response: stage and tool lines as
/// `⏳ <text>`, streamed text (tail-truncated to EMBED_DESCRIPTION_MAX) while it
/// grows; throttled by EDIT_GAP; `flush` clears the buffer. Failures are logged at
/// debug and ignored (a sink never fails the turn). The final reply edit replaces it.
pub struct InteractionProgress { /* Arc<Http>, CommandInteraction, Mutex<(Throttle, String)> */ }
impl ProgressSink for InteractionProgress { /* … */ }

/// DMs: pulses the typing indicator (at most every TYPING_GAP) on any progress.
pub struct TypingProgress { /* Arc<Http>, ChannelId, Mutex<Throttle> */ }
impl ProgressSink for TypingProgress { /* … */ }

// discord/gateway.rs
pub(super) enum Work {
    Command { interaction: Box<CommandInteraction>, parsed: Parsed, mode: DeferMode },
    Direct { channel: ChannelId, message: MessageId, parsed: Parsed },
}

pub const QUEUE_CAPACITY: usize = 32;
pub const BUSY_REPLY: &str = "I'm busy right now; try again in a minute.";

/// serenity's event handler. Does no business work: parse, defer, enqueue.
pub(super) struct Gateway {
    pub queue: mpsc::Sender<Work>,
    pub connected: Arc<AtomicBool>,
    pub guild_id: Option<u64>,
}
#[async_trait] impl EventHandler for Gateway {
    // ready: connected = true; register `create_commands()` to the guild when
    //        configured, else globally; a registration failure is a warn!, not fatal.
    // shard_stage_update: connected = (new stage == Connected).
    // interaction_create(Interaction::Command): `from_command`; a refusal is answered
    //        immediately with an ephemeral message and never enqueued; otherwise defer
    //        (`defer_ephemeral` / `defer` per `defer_mode`) FIRST, then `try_send`;
    //        a full queue edits the response to BUSY_REPLY. A failed defer is logged and dropped.
    // message: `from_direct_message`; `try_send`; full → warn! and drop.
}

// discord/worker.rs
pub const DISCORD_WORKERS: usize = 4;

/// Drains the queue until it closes or `cancel` fires. Per item: build the
/// progress sink, `inbound.handle(..)`, deliver the outcome:
/// - Command: `route_reply` + `posts`, edit the original response, follow-ups after;
///   `Ignored` → edit to NO_REPLY (B11).
/// - Direct: post the reply into the DM channel with a reference to the message;
///   `Ignored` → nothing.
/// Delivery failures are logged (no token or body in the log) and never retried:
/// an interaction token lives 15 minutes and a turn is not idempotent.
pub(super) async fn run_worker(queue: Arc<Mutex<mpsc::Receiver<Work>>>, inbound: Arc<Inbound>, http: Arc<Http>, cancel: CancellationToken);

// discord/adapter.rs
pub struct DiscordAdapter {
    config: DiscordConfig,
    http: Arc<Http>,                       // built in `new` from the token; also used by `send`
    connected: Arc<AtomicBool>,
    inbound: OnceLock<Arc<Inbound>>,       // A5, as MatrixAdapter
}
impl DiscordAdapter {
    pub fn new(config: DiscordConfig) -> Self;
    pub fn attach_inbound(&self, inbound: Arc<Inbound>);
}

/// B7: 1 s, 2 s, 4 s … capped at 300 s; `attempt` resets after a session that lasted ≥ 60 s.
pub(super) fn reconnect_delay(attempt: u32) -> Duration;
/// B7: the gateway errors that retrying cannot fix.
pub(super) fn is_fatal(err: &serenity::Error) -> bool;

#[async_trait] impl ChannelAdapter for DiscordAdapter {
    fn channel(&self) -> Channel { Channel::Discord }
    fn is_connected(&self) -> bool;        // the AtomicBool
    /// Spawns DISCORD_WORKERS workers into a JoinSet under a child token, then loops:
    /// build `Client::builder(token, GUILDS | DIRECT_MESSAGES).event_handler(Gateway{..})`,
    /// `select!` { cancel → `shard_manager.shutdown_all()`, connected = false, join the
    /// workers, Ok(()) ; `client.start()` ended → connected = false; fatal → Err;
    /// else sleep `reconnect_delay` (cancellable) and rebuild }.
    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()>;
    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>; // outbound::send
    // authorize_delivery and find_conversation_by_name keep their defaults: the outbox
    // sends Discord only `IdentityLinked` (not owner-chosen, R82), and the AI outage
    // alert targets Matrix.
}
```

- [ ] **Step 1: Write failing tests.**
  - `Throttle`: first call ready; within the gap not ready; after the gap ready again.
  - `reconnect_delay`: 0→1 s, 1→2 s, 8→256 s, 9 and 30 → 300 s (no overflow at `u32::MAX`).
  - `is_fatal`: the three gateway errors are fatal; `GatewayError::Closed(None)` and an HTTP error are not.
  - `DiscordAdapter`: `channel()` is Discord; a fresh adapter is disconnected and `run` without an attached pipeline returns `Err` naming the pipeline (no network: the check precedes the client build); `ChannelRegistry::send` to `Conversation(Discord(1))` through a disconnected adapter is `NotConnected(Discord)` (the registry gate, no HTTP).
  - Worker, without serenity: extract the outcome → delivery decision into a pure function `fn plan_command_reply(outcome: InboundOutcome, in_guild: bool, mode: DeferMode) -> Result<CommandReplyPlan, ChannelError>` (`CommandReplyPlan { original: Post, follow_ups: Vec<Post>, follow_ups_ephemeral: bool }`) and test: `Ignored` → original NO_REPLY; a `Private` reply in a guild with a `Public` defer → original PRIVATE_NOTICE, all posts as ephemeral follow-ups; a long Normal reply → original is the first chunk, follow-ups keep the defer's ephemerality.
  - End to end through the real pipeline (pure seam): with `Inbound` from the inbound test helpers, a fixture interaction from an **unlinked** Discord user for `/todo` goes `from_command` → `Inbound::handle` → `plan_command_reply` and the original post contains "not linked yet" (spec invariants 1–2 on Discord); the same user linked (`identities::bind`) reaches the stub module; `/link` in a guild fixture gets the DM refusal mentioning `<code>/link</code>` rendered as `` `/link` `` and mints no token (`link_tokens::count == 0`, invariant 3).
- [ ] **Step 2: Run**, see them fail.
- [ ] **Step 3: Implement.** Keep each file < 300 lines; the serenity calls (`defer`, `edit_response`, `create_followup`, `send_message`, `broadcast_typing`, `set_commands`) live only in `gateway.rs`, `worker.rs`, `progress.rs`, `outbound.rs`, each wrapped in `tokio::time::timeout(SEND_TIMEOUT, …)`.
- [ ] **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(discord): DiscordAdapter with deferred interactions, bounded workers and reconnect backoff`

---

### Task 6: Composition root and boundaries

**Files:**
- Modify: `backend/src/app.rs`, `backend/tests/architecture.rs`

**Interfaces:**

```rust
// app.rs
/// One adapter's supervised `run` task.
struct AdapterTask { name: &'static str, handle: JoinHandle<()> }

/// The name the supervisor and logs use for an adapter's task.
fn front_door_name(channel: Channel) -> &'static str;   // Matrix → "Matrix sync loop", Discord → "Discord gateway",
                                                        // Telegram → "Telegram updates", Email → "Email poller"

async fn supervise_front_doors(
    shutdown: &CancellationToken,
    api: &mut JoinHandle<()>,
    adapters: &mut Vec<AdapterTask>,   // replaces `matrix`; `select_all` over them (pending when empty)
    jobs: &mut JoinHandle<()>,
    events: &mut JoinHandle<()>,
) -> Option<&'static str>;
```

In `run`: `let discord = config.discord.clone().map(|c| Arc::new(DiscordAdapter::new(c)));`; the channel list for `check_unique` is Matrix plus Discord when present; inside `Arc::new_cyclic` one `Arc<Inbound>` is attached to every adapter (`matrix.attach_inbound`, `discord.attach_inbound`) and `ChannelRegistry::from_unique` gets both; `spawn_all` results all become `AdapterTask`s via `supervised(front_door_name(ch), handle)`; each is an `OwnedTask` in the shutdown list (after the API server, Matrix first). `log_registration_mode` logs whether Discord is enabled and whether commands go to a guild (never the token).

- [ ] **Step 1: Adapt the existing supervision tests** to pass `&mut vec![AdapterTask { name: "Matrix sync loop", handle }]` (meaning unchanged), and add `an_adapter_other_than_matrix_ending_unrequested_stops_the_process` (two adapter tasks, the second ends → `Some("Discord gateway")`, `shutdown` cancelled) and `front_door_names_are_distinct_per_channel`.
- [ ] **Step 2: Architecture tests.** Add `"DiscordAdapter::new("` to `only_the_composition_root_builds_adapters_and_the_registry`, and:

```rust
/// serenity is the Discord adapter's private dependency.
#[test]
fn serenity_stays_inside_the_discord_adapter() {
    let hits: Vec<_> = rust_files(&src(""))
        .into_iter()
        .filter(|f| !relative_to_src(f).starts_with("interface/channels/discord/"))
        .flat_map(|f| {
            let text = fs::read_to_string(&f).expect("readable source file");
            let rel = relative_to_src(&f);
            production_hits(&text, &["serenity"])
                .into_iter()
                .map(move |(n, l)| format!("{rel}:{n}: {l}"))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(hits.is_empty(), "serenity outside interface/channels/discord:\n{}", hits.join("\n"));
}
```

  (`app.rs` names `DiscordAdapter` and `DiscordConfig`, never a serenity type.)
- [ ] **Step 3: Run** `cargo test -p matrix-bot app::` and `cargo test -p matrix-bot --test architecture`; see the new ones fail, then implement until green.
- [ ] **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(app): register the Discord adapter and supervise every channel`

---

### Task 7: Docs and hand-off

**Files:**
- Modify: `docs/architecture.md` (*Chat channels*: Discord adapter — intents, commands, deferral and ephemerality (B2), work queue (B8), failure policy (B7), no buttons (B5)); `README.md` (configuration table + "Enabling Discord": create an application, copy the bot token into `DISCORD_TOKEN`, invite with scopes `bot applications.commands` and permissions *Send Messages*, *Embed Links*, *Attach Files*, *Read Message History*; no privileged intents; `DISCORD_GUILD_ID` for instant command updates while testing, global commands can take up to an hour to appear; `DISCORD_ALLOWED_USERS` for signup); `.env.example` (`DISCORD_TOKEN`, `DISCORD_GUILD_ID` commented out next to `DISCORD_ALLOWED_USERS`); `docs/superpowers/specs/2026-10-01-omnichannel-design.md` (amendment bullet for plan B: B1–B16 by ruling number); `SPEC.md` §9 where it describes Discord; `docs/superpowers/HANDOFF.md` (§1 row, §4 plan B task table with commits and final verification, §7 next action = plan C, §9 rulings R85+ from B1–B16 and any implementation rulings, §11: per-channel `/help` text (B10), `is_ready` cost (B9)).
- [ ] **Step 1: Write the docs.**
- [ ] **Step 2: Final verification** — the full Gate, `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1`, `cargo build -p matrix-bot --release`, `npm --prefix frontend run build` (unchanged frontend, sanity only). Record the numbers in HANDOFF §4.
- [ ] **Step 3: Commit** — `docs: omnichannel plan B hand-off, architecture and setup`

---

## What the user verifies (not an agent task)

A live check needs a real Discord application and therefore the user (HANDOFF §6 forbids agents contacting external services). After merge into the branch, with a test bot token and `DISCORD_GUILD_ID` set, on a non-production deployment:

1. Boot logs "Discord enabled (guild commands)"; the six commands appear in the guild.
2. Unlinked: `/todo` → "not linked yet" (ephemeral); `/link` in the guild → "send /link in a direct message"; `/link` in a DM → the link URL; finishing it on the web posts "✓ linked" in the DM.
3. Linked: `/todo args:add milk` in the guild is ephemeral; `/todo` in the DM lists it; the same todo is visible in the web UI and on Matrix.
4. `/ask prompt:...` shows progress edits, then the answer; a DM text message gets an agent answer.
5. A todo with a due time set from Discord fires its reminder in that Discord conversation.
6. A message containing `@everyone` (e.g. `/todo args:add @everyone`) is echoed without pinging anyone.

## Self-review against the spec

| Spec item | Where |
|---|---|
| §8.2 intents `GUILDS \| DIRECT_MESSAGES` | Task 5 (`run`) |
| §8.2 commands, global or `DISCORD_GUILD_ID` | Task 3 (table), Task 5 (`ready`) |
| §8.2 one `args` option; `/ask` required `prompt` (D6) | Task 3, B15 |
| §8.2 defer immediately; ephemeral when private in a guild; edit original; progress via edit | Tasks 4–5, B2 |
| §8.2 `is_direct` = DM; `/link` refuses guilds | Task 3 (`from_command`), existing `handle_link`, Task 5 test |
| §8.2 embeds via Markdown renderer; Design.md palette | Tasks 2, 4, B4 |
| §8.2 config `DISCORD_TOKEN`, `DISCORD_GUILD_ID`, `DISCORD_ALLOWED_USERS`; registered only with a token | Task 1, Task 6 |
| §3.3 2000-char / embed limits | Task 4 (4096-char embed chunks; plain `content` is never used) |
| §4 `run` reconnects with bounded backoff, returns on cancel | Task 5, B7 |
| §5 shared pipeline; free text in a DM → agent | Task 5, B3 |
| §9 reply-to-origin (reminders, link confirmation) | unchanged code paths: `discord:<id>` storage form + `ChannelRegistry::send` → `DiscordAdapter::send` (Task 4) |
| §10 errors mapped at the boundary; 429 → `Transient` | Task 4 (`classify`) |
| §11 invariants 1, 2, 3, 10 on Discord; no mass mentions | Tasks 4–5 tests |
| §12 fixtures under `contracts/channels/`; no network | Tasks 3–5 |
| §12 architecture: platform crate confined, adapters built only in `app.rs` | Task 6 |
