# Sub-project 2, Plan C: Telegram adapter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** draft for the user's approval (2026-10-02). No code before approval.

**Goal:** Telegram becomes the third `ChannelAdapter`. A linked Telegram user runs `/todo`, `/expense`, `/portfolio`, `/help`, `/link` (and any other registry command typed as `/name …`) and talks to the agent in a private chat, against the same `BotModule` registry, services and account as on Matrix and Discord. In groups the bot answers only commands addressed to it and mentions. The todo list carries a "Done" button per item whose press is re-dispatched **as the presser**. Reminders and the link confirmation come back to the Telegram chat they started in. An unlinked Telegram user can only `/start`, `/help` and `/link`.

**Architecture:**
- **Config** `config/channels.rs` — `TelegramConfig { token, mode: Webhook { secret } | Polling }` from `TELEGRAM_BOT_TOKEN` / `TELEGRAM_MODE` / `TELEGRAM_WEBHOOK_SECRET`. The adapter exists only when `TELEGRAM_BOT_TOKEN` is set.
- **Renderer** `interface/channels/render/telegram_html.rs` — module HTML subset → Telegram's HTML parse mode, split into ≤ 4096-character messages with tags balanced across splits. Pure, table-tested.
- **Adapter** `interface/channels/telegram/` — `teloxide-core` types and `Bot`; update normalization (messages, commands, mentions, callback queries); a bounded update queue fed by either the webhook endpoint or long polling, drained by a fixed worker pool that calls the shared `Inbound` pipeline; reply rendering with inline keyboards from `Content.actions`; an `editMessageText` `ProgressSink`; `send` for jobs and the outbox.
- **HTTP** `interface/http/telegram_webhook.rs` — `POST /api/webhooks/telegram` (outside `/api/v1`): constant-time secret check before parsing, per-IP rate limit, hands the raw body to the adapter's queue and answers at once. The Bot-API **emulation** shim moves to `interface/http/webhooks/telegram_compat.rs`, contract unchanged.
- **Module** `interface/chat/todos` — the list reply carries one `Action { label: "Done #<id>", command: "todo done <id>" }` per item (Matrix and Discord ignore actions).
- **Composition root** `app.rs` — builds the adapter when configured, attaches the shared pipeline, hands the webhook ingress to the API state.

No migration, no new table, no SQL, no service, no `/api/v1` route, no frontend change, `backend/openapi.json` byte-identical (T12).

**Tech Stack:** Rust, `teloxide-core` 0.13 (Bot API types + `Bot`; `rustls`; brings `reqwest` 0.12, see T1), axum 0.7, tokio + `tokio_util::sync::CancellationToken`, the plan A channel framework, plan B's Discord adapter as the second reference.

**Spec:** `docs/superpowers/specs/2026-10-01-omnichannel-design.md` (§3.2–§5, §8.3, §9–§12, invariants 7, 9, 10). Read it, `AGENTS.md`, plan B (`plans/2026-10-02-omnichannel-b-discord.md`) and `docs/superpowers/HANDOFF.md` §5, §6, §8 and rulings R70–R101 first. This plan cites real types from plans A and B.

**Worktree:** a separate worktree of branch `feat/omnichannel` (stacked on `refactor/outbox`). Never work in the main checkout.

## Code this plan builds on (read before Task 1)

| Path | What it gives Telegram |
|---|---|
| `backend/src/interface/channels/adapter.rs` | `ChannelAdapter` (`channel`, `is_connected`, `run`, `send`, defaulted `authorize_delivery`, `find_conversation_by_name`), `ChannelError { NotConnected, NotConfigured, Rejected, Transient }`, `SentRef` |
| `backend/src/interface/channels/message.rs` | `IncomingMessage`, `OutgoingMessage { target, content, reply_to, visibility, idempotency_key, join_if_needed }`, `Content { plain, html, attachments, actions }`, `Action { label, command }`, `Reply::with_actions`, `Visibility`, `MessageRef` |
| `backend/src/interface/channels/inbound.rs` | `Inbound::handle` → `InboundOutcome { Replied, Ignored }`; `link` before resolution; `Unlinked` gets only `start`/`help`/`link` (`UNLINKED_REPLY` + hint for `start`/`help`); free text → `ai` where addressed; panic fence |
| `backend/src/interface/channels/discord/` | the second reference adapter: `commands.rs` (bare `full_body`, R85), `inbound.rs` (fixtures → `IncomingMessage`), `render.rs`/`worker.rs` (pure `plan_*_reply`, private routing R86), `gateway.rs`+`worker.rs` (bounded queue + 4 workers, R92), `adapter.rs` (`OnceLock<Arc<Inbound>>`, reconnect backoff R91), `outbound.rs` (`classify`, timeouts) |
| `backend/src/interface/channels/render/{html,markdown}.rs` | the one HTML scanner (reuse its token iterator, as `markdown.rs` does) |
| `backend/src/interface/chat/link.rs` | `handle_link` words its refusal `/link` off Matrix and returns the URL as `Reply::private` |
| `backend/src/interface/chat/todos/{view,mutate}.rs` | `format_todo_list`, `handle_done(.., "done <id>")` — the "Done" button's command |
| `backend/src/interface/http/webhooks_telegram.rs` | the Bot-API emulation shim (`/bot:token/sendMessage`), moved in Task 6 |
| `backend/src/interface/http/rate_limit.rs` | `InMemoryRateLimiter`, the per-purpose `check_*_limit` pattern |
| `backend/src/domain/channels/mod.rs` | `Channel::Telegram`, `PlatformUserId::parse(Telegram, _)` (i64 > 0), `ConversationAddress::Telegram(i64)` (any non-zero; `telegram:<id>`), `RequireLink` |
| `backend/src/config/channels.rs` | `DiscordConfig`/`parse_discord` (the pattern), `parse_link_signup_allowlist` already reads `TELEGRAM_ALLOWED_USERS` |
| `backend/src/app.rs` | `front_door_name(Telegram) == "Telegram updates"`, `AdapterTask` supervision of every adapter (R97), `Arc::new_cyclic` attach |

## Global Constraints

- **Layers.** `teloxide_core` appears only under `interface/channels/telegram/` (new architecture test, Task 8). `interface/http/telegram_webhook.rs` sees only the adapter's `TelegramIngress` (raw bytes in, no teloxide type). `application/`, `llm/`, `interface/chat/`, `domain/` stay free of it (existing tests already list `teloxide`). `interface/channels/**` runs no SQL. Only `app.rs` constructs `TelegramAdapter`.
- **Frozen behaviour elsewhere.** Every Matrix and Discord command, reply, job and outbox behaviour is unchanged; the todo list's HTML is byte-identical (actions are a separate field). The existing suite passes unchanged in meaning.
- **OpenAPI.** `backend/openapi.json` byte-identical after every task (T12). No `.sqlx/` change, no migration.
- **No network in tests.** Nothing contacts Telegram. Platform-shaped code is tested through pure functions over teloxide-core types deserialized from fixtures under `contracts/channels/`; the HTTP glue stays thin. The webhook endpoint is tested in-process with axum (`oneshot`), not over a socket to Telegram.
- **Security** (spec §11): no owner key from a Telegram id — every update goes through `Inbound::handle`; an unlinked identity reaches only `start`/`help`/`link`; `/link` is refused outside a private chat; a webhook without the right secret is refused **before the body is parsed** (invariant 7); a callback is dispatched as the presser, its data never trusted (invariant 9); a `Private` reply is never posted in a group (invariant 10). Never log the bot token, the webhook secret, callback data or message bodies.
- **Bounds.** Every Bot API call has a timeout; updates go through a bounded channel to a fixed worker pool; polling and webhook registration retry with capped exponential backoff; recent `update_id`s are de-duplicated in a bounded set; every long-lived task holds a child `CancellationToken` and joins `app::run`'s shutdown.
- **File size.** No file in `backend/src/` exceeds 500 production lines; target < 300 per Telegram file. `config/mod.rs` is at ~490: Telegram settings and their validation live in `config/channels.rs`.
- **Error handling.** No `unwrap()`/`expect()` on runtime paths. teloxide errors map to `ChannelError` at the adapter boundary and never escape it.
- **Hard safety rules** (HANDOFF §6): never read `.env` or `data/` of the main checkout, never run the bot binary (only `cargo run --bin generate-openapi`), never contact Telegram or any external service from tests, never push/merge/PR without the user's instruction, never bare `git stash`.
- **Commits.** Conventional Commits, one logical change each, ending with the committing model's trailer.

### Iteration discipline and the Gate

Same as plans A and B: narrow tests while iterating (`cargo test -p matrix-bot interface::channels::telegram::`), the full Gate once per task just before its commit:

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

Task 3 adds a dependency: also run `cargo build -p matrix-bot --release` once there and review `cargo tree -d -p matrix-bot` (T1).

## Plan clarifications (supersede the spec where they differ; record in HANDOFF as rulings R102+)

- **T1 — `teloxide-core`, not `teloxide`.** The spec names "`teloxide` types and `Bot`"; both live in `teloxide-core` 0.13 (MSRV 1.82). The `teloxide` crate adds only the dispatcher framework (dptree, its own axum 0.8 webhook server) that this adapter replaces with the shared pipeline, so it is not taken. Dependency: `teloxide-core = { version = "0.13", default-features = false, features = ["rustls"] }`. Cost, stated up front: it requires `reqwest` 0.12, a third `reqwest` line next to the workspace's 0.11 and matrix-sdk's 0.13 (hyper 1 and rustls 0.23 are already in the lock, so the increment is mostly reqwest itself). The alternative — hand-written Bot API structs over the workspace's `reqwest` 0.11 — saves that line but means maintaining `Update`/`Message`/`CallbackQuery` models by hand; rejected for this plan, and the user may overrule at approval.
- **T2 — `full_body` is the bare form (R85).** `/todo@MyBot add milk` → `InboundText::Command { command: "todo", raw_args: "add milk", full_body: "todo add milk" }`. Any `/name` is parsed as a command (the registry ignores unknown names), so every module is reachable by typing; `setMyCommands` advertises `start help todo expense portfolio link`. A command addressed to another bot (`/todo@OtherBot`) is ignored.
- **T3 — addressed text.** Private chat: non-command text → `FreeText`, `is_direct = true`. Group/supergroup: commands addressed to us (no suffix or `@<our username>`) → `Command`, `is_mentioned = true`; text containing `@<our username>` → `FreeText` with the mention stripped, `is_mentioned = true`; a reply to one of the bot's messages → `FreeText`, `is_mentioned = true`; anything else → dropped before the queue. Channels (broadcast) and edited messages are ignored. The bot's username comes from `getMe` at start-up.
- **T4 — `/start` for a linked user is `/help`.** The pipeline already answers `start` for unlinked identities (`UNLINKED_REPLY` + hint). No module claims `start`, so for a linked user `Inbound::handle` returns `Ignored`; the worker then re-dispatches the same message as `help` (deep-link payloads are ignored). The adapter needs no identity lookup of its own.
- **T5 — quoted messages.** A message that replies to another message carries `quoted = QuotedMessage` built from the replied-to text (as Matrix does), so `/todo add` / `/remind` in reply to a message use it as the body.
- **T6 — callbacks are re-dispatched as the presser (invariant 9).** `callback_data` is exactly an `Action.command` (≤ 64 bytes, T7). A callback query is answered at once (`answerCallbackQuery`, empty text) to stop the client spinner, then its data is parsed by the same command parser **without** a leading `/` and dispatched through `Inbound::handle` with `sender = query.from` and `conversation = the button message's chat`, `is_direct` from that chat, `is_mentioned = true`. Ownership is therefore checked by the services exactly as if the presser had typed the command: pressing someone else's "Done #12" in a group answers "not found" for the presser and changes nothing of the owner's. A callback whose message is inaccessible (too old) is answered and dropped.
- **T7 — actions render as an inline keyboard.** One button per row, label truncated to 64 characters, at most 10 buttons on the last message of a reply; an action whose `command` exceeds 64 bytes is dropped with a `debug!` (never truncated — a truncated command could mean something else). The todo list is the first producer: `TodoModule`'s list reply gets `Done #<id>` actions for at most the 10 items shown.
- **T8 — group privacy (invariant 10).** In a group or supergroup, a `Visibility::Private` reply is never posted; the adapter posts instead `PRIVATE_IN_GROUP` ("That answer is private — message me directly.") replying to the command. Private chats get every reply. (R78 decides which replies are private; todos are Normal.)
- **T9 — webhook ingress.** `POST /api/webhooks/telegram` is mounted outside `/api/v1` only when Telegram runs in webhook mode (else 404). Order: per-IP rate limit (`check_telegram_webhook_limit`, 120/min) → `X-Telegram-Bot-Api-Secret-Token` compared in constant time against the configured secret (missing or wrong → 401, body never read into the parser) → `TelegramIngress::submit(Bytes)` (`try_send` onto the bounded queue) → `200` on accept, `503` on a full queue (Telegram redelivers later, nothing is lost). The global 1 MiB body limit applies.
- **T10 — constant-time comparison.** No crate in the workspace provides it; add `utils::secure::constant_time_eq(a: &[u8], b: &[u8]) -> bool` (length-checked XOR fold, `#[inline(never)]`), unit-tested, used by the webhook check. Not a new dependency.
- **T11 — one update stream, two sources.** The adapter owns a bounded `mpsc<Bytes>` (`QUEUE_CAPACITY = 64`) and `TELEGRAM_WORKERS = 4` workers. Webhook mode: `run` calls `setWebhook(url = {WEB_URL}/api/webhooks/telegram, secret_token, allowed_updates = [message, callback_query])` with backoff until it succeeds, then waits for cancel. Polling mode: `run` calls `deleteWebhook`, then loops `getUpdates(offset, timeout = 50 s, allowed_updates)` and feeds each update's JSON into the same queue (full queue → the worker pool is saturated; polling waits for capacity instead of dropping). Workers deserialize with teloxide-core's `Update`, de-duplicate on `update_id` (bounded set of the last 1024; webhook redelivery and polling restarts are at-least-once), normalize, call the pipeline, deliver.
- **T12 — the new endpoint is not in OpenAPI.** Like `/health/*`, `/api/webhooks/telegram` is infrastructure for one caller (Telegram), not application API; it is documented in `docs/api.md`. The compat shim's move keeps its `#[utoipa::path]` and operation ids, so `openapi.json` stays byte-identical (verified in Task 6).
- **T13 — `Config::validate`.** Webhook mode requires `TELEGRAM_WEBHOOK_SECRET` (1–256 chars of `A–Z a–z 0–9 _ -`, Telegram's rule) and an `https://` `WEB_URL` (Telegram only calls HTTPS); polling mode requires neither. A `TELEGRAM_ALLOWED_USERS` entry that `PlatformUserId::parse(Telegram, _)` rejects is refused by name, as plan B Task 1 does for `DISCORD_ALLOWED_USERS`. Validation code lives in `config/channels.rs`.
- **T14 — no outbox dedupe on Telegram.** The Bot API has no idempotency key; `idempotency_key` and `join_if_needed` are ignored. A re-claimed outbox delivery (lease expiry after a send) can post twice; only the one-line link confirmation goes through the outbox to Telegram (R93 stays: `ChannelSink::is_ready` remains "any adapter connected").
- **T15 — progress.** Private chats and groups: `TelegramProgress` sends `sendChatAction(typing)` at most every 4 s, and for stage/tool reports sends one status message (`⏳ <text>`) on the first report, then edits it (`editMessageText`) at most every 1.5 s; streamed text is shown tail-truncated. When the reply is delivered the status message is deleted (`deleteMessage`; on failure it is edited to `✓`). A sink never fails the turn.
- **T16 — attachments.** `audio/ogg` → `sendVoice` (voice note, as Matrix's MSC3245), other audio → `sendAudio`, images → `sendPhoto`, anything else → `sendDocument`; over 50 MB (the Bot API upload limit) the send is `Rejected`. The caption is sent as its own text message before the file (Telegram captions are capped at 1024 chars).

---

## File map

| File | Responsibility | Task |
|---|---|---|
| `backend/src/config/channels.rs`, `config/mod.rs` | `TelegramConfig`, `TelegramMode`, parsing and validation; `Config.telegram` | 1 |
| `backend/src/utils/secure.rs` (+ `utils/mod.rs`) | `constant_time_eq` | 1 |
| `backend/src/interface/channels/render/telegram_html.rs` | HTML subset → Telegram HTML, `split_messages` | 2 |
| `backend/Cargo.toml`, `Cargo.lock` | `teloxide-core` | 3 |
| `backend/src/interface/channels/telegram/{mod,commands,inbound}.rs` | command parsing (T2–T4), update → `IncomingMessage` (T3, T5, T6) | 3 |
| `contracts/channels/telegram_{private_text,group_command,callback_query}.json` | fixtures | 3 |
| `backend/src/interface/channels/telegram/{render,outbound}.rs` | `Content` → message plans + keyboard (T7, T8, T16), `send`, `classify` | 4 |
| `backend/src/interface/channels/telegram/{ingress,worker,progress,adapter}.rs` | queue + dedupe (T11), workers, `TelegramProgress` (T15), `TelegramAdapter` | 5 |
| `backend/src/interface/http/telegram_webhook.rs`, `interface/http/webhooks/telegram_compat.rs`, `interface/http/{mod,routes,openapi,rate_limit}.rs`, `interface/http/auth/types.rs` | webhook endpoint (T9), compat move | 6 |
| `backend/src/interface/chat/todos/*` | "Done" actions on the list reply (T7) | 7 |
| `backend/src/app.rs`, `backend/tests/architecture.rs` | build/attach/supervise; ingress into `ApiState`; boundaries | 8 |
| docs (`architecture.md`, `api.md`, README, `.env.example`, spec, `SPEC.md` §9, HANDOFF) | | 9 |

Each task leaves the workspace compiling and the Gate green at its commit.

---

### Task 1: Telegram configuration and constant-time comparison

**Files:** Modify `backend/src/config/channels.rs`, `backend/src/config/mod.rs`, `backend/src/utils/mod.rs`; create `backend/src/utils/secure.rs`.

**Interfaces:**

```rust
// config/channels.rs
#[derive(Clone)]
pub enum TelegramMode { Webhook { secret: String }, Polling }

#[derive(Clone)]
pub struct TelegramConfig { pub token: String, pub mode: TelegramMode }
impl std::fmt::Debug for TelegramConfig { /* mode name only; token and secret "<redacted>" */ }

/// `None` when TELEGRAM_BOT_TOKEN is unset or blank. `Err` naming the variable for
/// an unknown TELEGRAM_MODE (`webhook` default | `polling`), or webhook mode without
/// a valid TELEGRAM_WEBHOOK_SECRET (T13).
pub(super) fn parse_telegram(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<TelegramConfig>>;
/// T13: webhook mode needs an https WEB_URL; allowlist entries must be valid Telegram ids.
pub(super) fn validate_telegram(cfg: Option<&TelegramConfig>, web_url: &str, allowlist: &[String]) -> anyhow::Result<()>;

// config/mod.rs
pub struct Config { /* … */ pub telegram: Option<TelegramConfig> }   // validate() calls validate_telegram

// utils/secure.rs
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool;
```

- [ ] **Step 1: Failing tests.** `config/channels.rs`: absent without a token; default mode is webhook; `polling` needs no secret; webhook without secret / with `"bad secret!"` / 257 chars → `Err` naming `TELEGRAM_WEBHOOK_SECRET`; unknown mode → `Err` naming `TELEGRAM_MODE`; Debug shows neither token nor secret; `validate_telegram` refuses `http://` WEB_URL in webhook mode, accepts it in polling mode, refuses allowlist entry `"abc"` by name. `utils/secure.rs`: equal, unequal same length, different lengths, empty.
- [ ] **Step 2: Run** (`cargo test -p matrix-bot config:: utils::secure`), see them fail.
- [ ] **Step 3: Implement.** `Config::for_test()` sets `telegram: None`.
- [ ] **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(config): Telegram settings (TELEGRAM_BOT_TOKEN, TELEGRAM_MODE, TELEGRAM_WEBHOOK_SECRET)`

---

### Task 2: HTML subset → Telegram HTML

**Files:** Create `backend/src/interface/channels/render/telegram_html.rs`; modify `render/mod.rs`.

**Interfaces:**

```rust
pub const MESSAGE_MAX: usize = 4096;   // visible characters per message

/// Module HTML subset → Telegram HTML parse mode: `b strong`→`b`, `i em`→`i`, `u`, `code`,
/// `pre`, `blockquote`, `a href` (http/https/mailto only, else text); `br`→`\n`;
/// `p`→paragraph separated by a blank line; `ul>li`→`• item`; `ol>li`→`1. item`;
/// unknown tags dropped keeping their text. Text keeps only `&lt; &gt; &amp; &quot;`
/// (other entities decoded to characters). Runs of 3+ newlines collapse to 2. Never fails.
pub fn html_to_telegram(html: &str) -> String;

/// Splits rendered Telegram HTML into messages of ≤ MESSAGE_MAX visible characters
/// (entities count as one), at line boundaries where possible, else on a char boundary
/// outside a tag or entity; tags open at a split are closed and reopened in the next part.
pub fn split_messages(rendered: &str) -> Vec<String>;
```

Reuse `render/html.rs`'s token iterator (as `markdown.rs` does) — one HTML scanner in the crate.

- [ ] **Step 1: Failing table tests:** each mapping above; `javascript:` href → text; `<script>x</script>` → `x`; `&nbsp;`/`&#39;` decoded; `&lt;b&gt;` stays escaped; `@everyone` unchanged (Telegram has no mass mention); a real `format_todo_list` output for two items renders without stray tags. `split_messages`: short → one; 9 000 chars of lines → three parts, each ≤ 4096 visible, concatenated visible text equals the input's; one 5 000-char line of `字` splits on a char boundary; a `<b>` open across a split is closed and reopened; entities are never split.
- [ ] **Step 2: Run** (`cargo test -p matrix-bot render::telegram_html`), see it fail. **Step 3: Implement. Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(channels): HTML subset to Telegram HTML renderer`

---

### Task 3: teloxide-core, command parsing and inbound normalization

**Files:** Modify `backend/Cargo.toml`, `Cargo.lock`, `interface/channels/mod.rs`; create `interface/channels/telegram/{mod,commands,inbound}.rs` and three fixtures.

**Dependency:** `teloxide-core = { version = "0.13", default-features = false, features = ["rustls"] }` (T1).

**Interfaces:**

```rust
// telegram/commands.rs
pub const ADVERTISED: &[(&str, &str)];   // setMyCommands: start help todo expense portfolio link, with descriptions
pub const ARGS_MAX: usize = 500;

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed { Command { command: String, args: String }, NotForUs, NotACommand }

/// `/name[@bot] args` → Command (name lower-cased; `@bot` must equal `our_username`,
/// case-insensitive, else NotForUs); anything not starting with `/` → NotACommand.
pub fn parse_command(text: &str, our_username: &str) -> Parsed;
/// Callback data (no leading `/`, T6) → Command or NotACommand.
pub fn parse_callback(data: &str) -> Parsed;
/// T2: Command → InboundText with the bare full_body; args over ARGS_MAX → Err(TooLong).
pub fn inbound_text(command: &str, args: &str) -> Result<InboundText, CommandRefusal>;

// telegram/inbound.rs
pub struct Normalized { pub message: IncomingMessage, pub text: InboundText, pub kind: UpdateKind }
pub enum UpdateKind { Message { chat_id: i64, message_id: i32 }, Callback { query_id: String, chat_id: i64, message_id: i32 } }

/// One teloxide-core `Update` → what to dispatch, or None (T3: not addressed, bot author,
/// edited message, channel post, empty). `MessageRef("message:<chat>:<id>")`.
pub fn normalize(update: &Update, our_username: &str) -> Option<Result<Normalized, (UpdateKind, CommandRefusal)>>;
```

Identity: `PlatformUserId::parse(Telegram, &from.id.0.to_string())`, display name = first + last name, else username; `from.is_bot` → None. Conversation `Telegram(chat.id.0)`, `is_direct = chat.is_private()`.

- [ ] **Step 1: Failing tests.** `commands.rs`: `/todo add milk`, `/todo@MyBot add milk` (case-insensitive), `/todo@OtherBot …` → NotForUs, `/Todo` → `todo`, plain text → NotACommand, 501-char args refused, `inbound_text("todo","add milk")` → bare `full_body` `todo add milk`, `parse_callback("todo done 12")` → Command. `inbound.rs` with fixtures (`include_str!` as `matrix/inbound.rs`): private text → FreeText, direct; group `/todo@MyBot add milk` → Command, `is_mentioned`, not direct; group text without mention → None; group text with `@MyBot` → FreeText with the mention stripped; a reply carrying `reply_to_message` → `quoted` set (T5); callback fixture → `UpdateKind::Callback`, sender = `from` (the presser), conversation = the message's chat; a bot author → None; fixtures must deserialize into teloxide-core 0.13's `Update`.
- [ ] **Step 2: Run** (`cargo test -p matrix-bot interface::channels::telegram`), see them fail. **Step 3: Implement.**
- [ ] **Step 4: Gate**, plus `cargo build -p matrix-bot --release` and `cargo tree -d -p matrix-bot`: record the duplicate majors added (expected: `reqwest` 0.12 only, T1) in the commit body.
- [ ] **Step 5: Commit** — `feat(telegram): command parsing and update normalization`

---

### Task 4: Outbound rendering and `send`

**Files:** Create `interface/channels/telegram/{render,outbound}.rs`.

**Interfaces:**

```rust
// telegram/render.rs
pub const PRIVATE_IN_GROUP: &str = "That answer is private — message me directly.";
pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_BUTTONS: usize = 10;
pub const CALLBACK_DATA_MAX: usize = 64;

#[derive(Debug, PartialEq)]
pub enum Part { Text(String) /* Telegram HTML */, File(Attachment) }

#[derive(Debug, PartialEq)]
pub struct Plan { pub parts: Vec<Part>, pub keyboard: Vec<(String, String)> /* (label, data) on the last text part */ }

/// T8: Private in a group → PRIVATE_IN_GROUP only. Else: caption/text parts from
/// `split_messages(html_to_telegram(html or escaped plain))`, then files (T16; Rejected
/// over MAX_UPLOAD_BYTES), keyboard from actions (T7).
pub fn plan(content: &Content, visibility: Visibility, in_group: bool) -> Result<Plan, ChannelError>;

// telegram/outbound.rs
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// `Conversation(Telegram(id))` → that chat; `Direct(identity)` on Telegram → chat id = user id
/// (works once the user has started the bot; 403 → Rejected); other targets → Rejected before
/// any call. `reply_to` = `message:<chat>:<id>` becomes reply parameters (other refs ignored).
/// Every call wrapped in CALL_TIMEOUT. Returns the last message's ref. T14: no dedupe.
pub async fn send(bot: &Bot, message: OutgoingMessage) -> Result<SentRef, ChannelError>;
/// teloxide `RequestError` → ChannelError: `Api` user errors (bot blocked, chat not found,
/// bad request) → Rejected; `RetryAfter`, network, 5xx → Transient.
pub(super) fn classify(err: &RequestError) -> ChannelError;
```

- [ ] **Step 1: Failing tests (pure):** `plan` — short HTML one text part; 9 000 chars → three; Private in group → exactly `[Text(PRIVATE_IN_GROUP)]`, no keyboard; Private in a private chat → full content; actions → keyboard on the last text part, 12 actions → 10, a 65-byte command dropped, label > 64 chars truncated; `audio/ogg` → File part after the caption; 51 MB → Rejected. `classify` table over constructed `RequestError`s. `send` target validation (Discord conversation, Matrix identity) → Rejected with no call (`Bot::new("x")` unused).
- [ ] **Step 2: Run**, see them fail. **Step 3: Implement. Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(telegram): message planning, inline keyboards and outbound send`

---

### Task 5: `TelegramAdapter` — queue, dedupe, workers, progress, polling and webhook registration

**Files:** Create `interface/channels/telegram/{ingress,worker,progress,adapter}.rs`.

**Interfaces:**

```rust
// telegram/ingress.rs
pub const QUEUE_CAPACITY: usize = 64;
#[derive(Clone)]
pub struct TelegramIngress { tx: mpsc::Sender<Bytes>, secret: Option<Arc<str>> }
impl TelegramIngress {
    /// T9 order is the HTTP handler's; this only checks the secret and enqueues.
    pub fn secret_matches(&self, header: Option<&[u8]>) -> bool;   // constant_time_eq
    pub fn submit(&self, body: Bytes) -> Result<(), Full>;           // try_send
}
/// Bounded `update_id` de-duplication (T11): remembers the last `capacity` ids.
pub struct RecentUpdates { /* VecDeque + HashSet */ }
impl RecentUpdates { pub fn new(capacity: usize) -> Self; pub fn first_time(&mut self, id: u32) -> bool; }

// telegram/progress.rs — T15
pub struct TelegramProgress { /* Bot, ChatId, Mutex<state> */ }
impl ProgressSink for TelegramProgress { /* … */ }
impl TelegramProgress { pub async fn finish(&self); }   // delete or ✓ the status message

// telegram/worker.rs
pub const TELEGRAM_WORKERS: usize = 4;
/// Pure: outcome → what to send (Ignored → nothing; Replied → `render::plan`).
pub fn plan_reply(outcome: InboundOutcome, in_group: bool) -> Result<Option<Plan>, ChannelError>;
pub(super) async fn run_worker(rx: Arc<Mutex<mpsc::Receiver<Bytes>>>, ctx: WorkerCtx, cancel: CancellationToken);

// telegram/adapter.rs
pub struct TelegramAdapter { config: TelegramConfig, bot: Bot, web_url: String, connected: Arc<AtomicBool>,
                             ingress: TelegramIngress, rx: Mutex<Option<mpsc::Receiver<Bytes>>>,
                             inbound: OnceLock<Arc<Inbound>> }
impl TelegramAdapter {
    pub fn new(config: TelegramConfig, web_url: String) -> Self;   // Bot with a reqwest 0.12 client with timeouts
    pub fn attach_inbound(&self, inbound: Arc<Inbound>);
    /// The webhook handler's handle; None in polling mode.
    pub fn ingress(&self) -> Option<TelegramIngress>;
}
pub(super) fn retry_delay(attempt: u32) -> Duration;   // 1 s doubling, capped at 300 s
#[async_trait] impl ChannelAdapter for TelegramAdapter {
    // channel = Telegram; is_connected = the AtomicBool (true after getMe + setWebhook/first poll)
    // run: take rx (Err if taken twice or no pipeline attached); getMe (username) with retry;
    //      setMyCommands(ADVERTISED); spawn TELEGRAM_WORKERS workers under a child token; then
    //      Webhook: setWebhook with retry, wait for cancel. Polling: deleteWebhook, getUpdates loop
    //      (timeout 50 s; feed raw JSON per update into the queue with `send().await`).
    //      Unauthorized from getMe → Err (fatal, as R91: a wrong token fails at boot).
    // send = outbound::send
}
```

Worker per update: deserialize `Update` (malformed → `warn!` without the body, drop); `RecentUpdates::first_time` else drop; `normalize`; callback → `answerCallbackQuery` first; refusal → reply with its text; `start` whose outcome is `Ignored` → re-dispatched once as `help` (T4); build `TelegramProgress` for `ai`/FreeText; `Inbound::handle`; `plan_reply`; deliver as replies to the triggering message; `progress.finish()`.

- [ ] **Step 1: Failing tests.** `RecentUpdates` (dup rejected, eviction after capacity). `retry_delay` table (0→1 s … ≥9 → 300 s, no overflow). `TelegramIngress`: right secret, wrong secret, missing header, different length; full queue → `Err(Full)`. `plan_reply`: Ignored → None; Private in group → PRIVATE_IN_GROUP. Adapter: `channel()`; fresh adapter disconnected; `run` without a pipeline → `Err` before any network call; `ChannelRegistry::send` to `Conversation(Telegram(1))` through a disconnected adapter → `NotConnected(Telegram)`. End to end through the real pipeline (pure seam, fixtures + `Inbound` from the shared test helpers, no network): unlinked private `/todo` → "not linked yet" (invariants 1–2); `/link` in a group fixture → the DM refusal, no token minted (invariant 3); linked user's `/todo` reaches the module; a callback `todo done <id>` pressed by user B for user A's todo → B's "not found", A's todo still open (invariant 9); a Private reply planned for a group → PRIVATE_IN_GROUP (invariant 10); a linked `/start` → help text.
- [ ] **Step 2: Run**, see them fail. **Step 3: Implement** (teloxide calls only in `worker.rs`, `progress.rs`, `outbound.rs`, `adapter.rs`, each wrapped in `CALL_TIMEOUT`). **Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(telegram): TelegramAdapter with bounded workers, dedupe, polling and webhook registration`

---

### Task 6: Webhook endpoint; the compat shim moves

**Files:** Create `interface/http/telegram_webhook.rs`; move `interface/http/webhooks_telegram.rs` → `interface/http/webhooks/telegram_compat.rs`; modify `interface/http/{mod,routes,openapi,rate_limit}.rs`, `interface/http/auth/types.rs` (`ApiState.telegram: Option<TelegramIngress>`).

**Interfaces:**

```rust
// rate_limit.rs
pub fn check_telegram_webhook_limit(ip: &str) -> bool;   // 120 / 60 s

// telegram_webhook.rs — not a #[utoipa::path] (T12)
/// T9: rate limit → constant-time secret check (401, body unread) → submit → 200 | 503.
/// 404 when Telegram is not in webhook mode. Logs neither the header nor the body.
pub async fn telegram_update(State(state): State<ApiState>, ConnectInfo(addr): ConnectInfo<SocketAddr>,
                             headers: HeaderMap, body: Bytes) -> StatusCode;
```

Mount `POST /api/webhooks/telegram` next to `/webhooks/:token` (outside `/api/v1`). Cookie CSRF does not apply (no session; the secret is the credential).

- [ ] **Step 1: Failing tests** (axum `oneshot` against the router with a test `ApiState` whose ingress has a known secret and a receiver the test holds): right secret → 200 and the exact bytes are on the queue; missing / wrong secret → 401 and nothing queued (invariant 7); full queue → 503; no Telegram configured → 404; rate limit exceeded → 429. The compat shim's existing tests pass unchanged after the move.
- [ ] **Step 2: Run**, see them fail. **Step 3: Implement.**
- [ ] **Step 4: Gate** — `openapi.json` must be byte-identical (T12).
- [ ] **Step 5: Commit** — `feat(http): Telegram webhook endpoint; Bot-API emulation shim moves to webhooks/telegram_compat.rs`

---

### Task 7: The todo list's "Done" buttons

**Files:** Modify `interface/chat/todos/{mod,view}.rs` (and the list call sites that build the reply).

The list reply becomes `Reply::from(html).with_actions(done_actions(&items))` where `done_actions` (pure, in `view.rs`) yields `Action { label: format!("Done #{id}"), command: format!("todo done {id}") }` for at most 10 shown items. The HTML is unchanged byte for byte.

- [ ] **Step 1: Failing tests:** `done_actions` for 0, 3 and 12 items (10 max, ids in list order, commands ≤ 64 bytes); the list reply's `html_body()` equals the previous output for the same items (pin it); the action command dispatched through `Inbound::handle` as the owner completes the todo.
- [ ] **Step 2: Run**, see them fail. **Step 3: Implement. Step 4: Gate** (Matrix and Discord suites unchanged: Matrix ignores actions, Discord drops them, B5).
- [ ] **Step 5: Commit** — `feat(todos): Done actions on the task list`

---

### Task 8: Composition root and boundaries

**Files:** Modify `backend/src/app.rs`, `backend/src/interface/http/mod.rs` (`start_api_server` receives the ingress), `backend/tests/architecture.rs`.

In `run`: `let telegram = config.telegram.clone().map(|c| Arc::new(TelegramAdapter::new(c, config.web_url.clone())));` Telegram joins `check_unique`, gets the shared `Arc<Inbound>`, goes into `ChannelRegistry::from_unique`, and its `run` is supervised as `"Telegram updates"` (`front_door_name`). `telegram.as_ref().and_then(|t| t.ingress())` is passed into `ApiState`. `log_registration_mode` logs whether Telegram is enabled and its mode (never the token or secret).

- [ ] **Step 1: Failing tests:** `app::` — a Telegram adapter task ending unrequested stops the process with `Some("Telegram updates")`. Architecture: add `"TelegramAdapter::new("` to `only_the_composition_root_builds_adapters_and_the_registry`, and `teloxide_stays_inside_the_telegram_adapter` (same shape as `serenity_stays_inside_the_discord_adapter`, needle `teloxide`).
- [ ] **Step 2: Run** (`cargo test -p matrix-bot app::` and `--test architecture`), see the new ones fail. **Step 3: Implement. Step 4: Gate.**
- [ ] **Step 5: Commit** — `feat(app): register the Telegram adapter and its webhook ingress`

---

### Task 9: Docs and hand-off

**Files:** `docs/architecture.md` (*Chat channels*: Telegram — webhook vs polling, ingress order T9, dedupe T11, groups and privacy T3/T8, callbacks as the presser T6, keyboards T7, progress T15); `docs/api.md` (`POST /api/webhooks/telegram`: secret header, 401/429/503/404, not in OpenAPI T12; the compat shim's new home); `README.md` (configuration table + "Enabling Telegram": create a bot with @BotFather, `TELEGRAM_BOT_TOKEN`; webhook mode needs a public `https` `WEB_URL` and `TELEGRAM_WEBHOOK_SECRET`; `TELEGRAM_MODE=polling` for local testing; group privacy mode stays on; `TELEGRAM_ALLOWED_USERS` for signup); `.env.example`; spec amendment bullet for plan C (T1–T16 by ruling number); `SPEC.md` §9 Telegram; `docs/superpowers/HANDOFF.md` (§1 row, §4 plan C task table with commits and final verification, §7 next action = plan D (Email), §9 rulings R102+ from T1–T16 and implementation rulings, §11: per-channel `/help` still Matrix-flavoured (B10), outbox duplicates possible on Telegram (T14)).

- [ ] **Step 1: Write the docs.**
- [ ] **Step 2: Final verification** — full Gate, `live_postgres`, `cargo build -p matrix-bot --release`, `npm --prefix frontend run build` (frontend unchanged, sanity only). Record the numbers in HANDOFF §4.
- [ ] **Step 3: Commit** — `docs: omnichannel plan C hand-off, architecture and setup`

---

## What the user verifies (not an agent task)

A live check needs a real Telegram bot (HANDOFF §6 forbids agents contacting external services). On a non-production deployment with a test bot from @BotFather:

1. **Polling first:** `TELEGRAM_MODE=polling`; boot logs "Telegram enabled (polling)"; the six commands appear in the bot's menu.
2. Unlinked: `/todo` → "not linked yet"; `/link` in a group → "send /link in a direct message"; `/link` in the private chat → the link URL; finishing it on the web posts "✓ linked" in the private chat.
3. Linked: `/todo add milk`, then `/todo` shows the list with a "Done #<id>" button; pressing it completes the task; the same task is visible on the web and on Matrix.
4. In a group: `/todo@<bot> add x` works; a portfolio command answers "That answer is private — message me directly."; another member pressing your "Done" button changes nothing of yours.
5. A text message in the private chat gets an agent answer with a typing indicator and a status line that disappears when the answer arrives.
6. A reminder set from Telegram fires in that Telegram chat.
7. **Webhook mode:** a public `https` `WEB_URL`, `TELEGRAM_WEBHOOK_SECRET` set, mode `webhook`; boot registers the webhook; steps 2–6 behave the same; `curl -X POST {WEB_URL}/api/webhooks/telegram` without the header returns 401.

## Self-review against the spec

| Spec item | Where |
|---|---|
| §8.3 webhook outside `/api/v1`, constant-time secret, rate-limited, 200 at once, bounded queue | Task 6 (T9, T10), Task 5 (T11) |
| §8.3 `setWebhook` at boot | Task 5 |
| §8.3 polling mode for development | Task 5 (T11) |
| §8.3 commands via `setMyCommands`; DM free text → agent; groups: commands and mentions only | Tasks 3, 5 (T2, T3) |
| §8.3 inline keyboards from actions, ≤ 64-byte data else dropped and logged; callback as the presser; todo "Done" first producer | Tasks 4, 5, 7 (T6, T7) |
| §8.3 compat shim renamed, contract unchanged | Task 6 (T12) |
| §8.3 config `TELEGRAM_BOT_TOKEN`, `TELEGRAM_MODE`, `TELEGRAM_WEBHOOK_SECRET` required in webhook mode, `TELEGRAM_ALLOWED_USERS` | Task 1 (T13) |
| §3.3 Telegram HTML parse mode, lists mapped to text, unknown tags stripped, 4096 split | Task 2 |
| §3.2 Private in a Telegram group refused | Task 4 (T8) |
| §4 `run` reconnects with bounded backoff, returns on cancel | Task 5 |
| §6 progress via `editMessageText` | Task 5 (T15) |
| §9 reply-to-origin | unchanged paths: `telegram:<id>` storage form + `ChannelRegistry::send` → `TelegramAdapter::send` |
| §10 errors at the boundary; `RetryAfter` → Transient | Task 4 (`classify`) |
| §11 invariants 1, 2, 3, 7, 9, 10 on Telegram | Tasks 5, 6 tests |
| §12 fixtures under `contracts/channels/`; no network; platform crate confined; adapter built only in `app.rs` | Tasks 3, 5, 8 |
