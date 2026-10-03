# Sub-project 2 — Omnichannel design

**Status:** approved 2026-10-01 (interview in session; decisions D1–D8 below).
**Branch:** `feat/omnichannel`, stacked on `refactor/outbox` (sub-project 1 is not yet merged to `master`; the user chose to start anyway).
**Roadmap:** `SPEC.md` Phase 4, §9. Where this document and `SPEC.md` §9 disagree, this document wins; `SPEC.md` §9 is updated in the plan's last task.

**Amended after implementation (plan A, 2026-10-02).** The sections below are the design; where the code differs, the code and the ruling named here win (`docs/superpowers/HANDOFF.md` §9):

- Migrations are `0006`–`0008` (plan clarification A1, R73); `0005` was already taken by `drop_webhook_outbox`.
- `Attachment.bytes` is `Vec<u8>`, not `bytes::Bytes`, and carries an optional neutral `waveform` for voice notes (R76, R78). `render/` holds only `html.rs` for now; the Discord, Telegram and plain renderers arrive with their plans.
- `ChannelError` variants carry data (`NotConnected(Channel)`, `NotConfigured(Option<Channel>)`), and `ChannelAdapter` has two defaulted hooks the sketch in §4 omits: `authorize_delivery(owner, to) -> Result<(), String>` (the Matrix webhook-alert room check, with the old error texts) and `find_conversation_by_name` (the AI outage alert, A4). `OutgoingMessage` gains `idempotency_key` and `join_if_needed`; outbox sends never join a room (R81). `ChannelSink` skips `authorize_delivery` for `IdentityLinked` (R82), and `DeliverySink::is_ready` means "any adapter connected" (R81).
- The inbound pipeline holds a `Weak<ChannelRegistry>` because the registry owns the adapters that own the pipeline (A5/R73); job handlers keep the narrow `Outbound` trait.
- Linking: `claim` also merges the account an identity was last unlinked from when it still holds data, and there is no separate confirm-merge step (R82); signup is `LinkService::sign_up_with_passkey` / `claim_for_new_account`, and `/link/signup/finish` takes `{ceremony_id, token, credential, label?}` (R83); `register_finish` became atomic too (R83).
- The web `/link` page is chosen by pathname, the SPA having no router (R84).

**Amended after implementation (plan B, Discord, 2026-10-02)** — rulings R85–R100 (= plan clarifications B1–B16):

- A slash command reaches the registry as the bare `<command> <args>` modules parse (R85); guild interactions are ephemeral except `/help`, and a `Private` reply is never posted on a public response (R86); DM text goes to the agent (R87).
- Embeds carry no title and no colour (R88); `Content.actions` are not rendered on Discord (R89); the adapter holds no `AppServices` (R90).
- `run` rebuilds the gateway client with capped exponential backoff and returns an error only for an invalid token or disallowed intents (R91); work runs on a bounded queue with four workers (R92); the supervisor watches every adapter (R97).
- `ChannelSink::is_ready` still means "any adapter connected" (R93); `/help` shows the shared `!`-prefixed text (R94); attachments are files capped at 10 MiB (R98); option limits are 500 / 4000 characters, re-checked server side (R99); the outbox key becomes Discord's nonce when it fits (R100).

**Amended after implementation (plan C, Telegram, 2026-10-03)** — rulings R102–R124 (= plan clarifications T1–T16):

- Uses `teloxide-core` 0.13 without full teloxide; polling and webhook modes (R102); bare commands without `@bot` mentions (R103).
- Slash commands + direct mentions + replies in groups; privacy mode preserved (R104); unhandled `/start` redispatches as `/help` (R105).
- Interactive todo "Done" inline keyboard buttons constructed from `Content.actions` (R108); callback queries executed under presser identity (R107); private responses refused in groups (R109).
- Webhook ingress at `/api/webhooks/telegram` with constant-time secret comparison and rate limiting (R110, R111).
- Outbound sends voice notes via `sendVoice` for Ogg Opus, attachments over 50 MB produce notice (R117, R119).

**Amended after implementation (plan D, Email, 2026-10-03)** — rulings R125–R138 (= plan clarifications E1–E14):

- Uses `mail-parser` 0.11, `lettre` 0.11, `async-imap` 0.12, `tokio-rustls` 0.26 with explicit `ring` provider (R125).
- Inbound body extracted from text/plain or converted HTML, quotes and signatures stripped (R126).
- Strict DMARC pass enforcement against `EMAIL_AUTHSERV_ID` (R127, Invariant 8); unauthenticated emails and auto-replies silently dropped without bounce (R127, R128).
- Outbound sent as `multipart/alternative` via lettre SMTP, threading preserved (`In-Reply-To`, `References`, `Re: <subject>`), actions rendered as text list (R130, R131).
- IMAP polling searches `UNSEEN`, marks `\Seen`, drains stream properly, and moves to `EMAIL_PROCESSED_FOLDER` if set (R133).
- Email adapter isolated with no-op progress sink (`NoProgress`) and supervised front door (R135, R138).

## 1. Goal

One application and domain, many interfaces. Matrix, Discord, Telegram and Email become protocol adapters that turn platform traffic into normalized messages, call the same application services and `BotModule`s, and render normalized replies back into platform-native form.

```text
inbound:   platform event → <Channel>Adapter → IncomingMessage → identity → command registry / agent → services → domain → PostgreSQL
outbound:  services / jobs / outbox → OutgoingMessage → ChannelRegistry → <Channel>Adapter → platform
```

An adapter may receive platform events, validate protocol input, resolve the sender's identity through `IdentityService`, convert messages, call application capabilities, and render replies. It may not hold business rules, run SQL, touch repositories, construct `AppServices`, or contain agent logic.

## 2. Decisions

| # | Decision | Why |
|---|---|---|
| D1 | **Linking is chat-initiated for every channel.** `/link` (`!link` on Matrix) in a direct conversation mints a single-use token; the browser finishes it with a passkey. The web-minted `!link <code>` flow and `matrix_link_challenges` are deleted. | One flow for every channel; the passkey is the proof of account ownership, the DM is the proof of identity ownership. |
| D2 | **Unlinked Discord/Telegram/Email identities get only `start`, `help` and `link`.** No account is provisioned for them. Matrix keeps auto-provisioning (behind its invite allowlist). | Anyone can message a public Telegram/Discord bot or an email address; auto-provisioning would let strangers create accounts and spend LLM credits. |
| D3 | **Outbound is reply-to-origin.** A reminder set on Telegram fires on Telegram. Jobs and the outbox route by conversation address through `ChannelRegistry`. A per-account preferred-channel setting is sub-project 4. | A Telegram-only user must still receive reminders. |
| D4 | **New accounts from a link token only for allowlisted identities.** An identity in `DISCORD_ALLOWED_USERS` / `TELEGRAM_ALLOWED_USERS` / `EMAIL_ALLOWED_SENDERS` may register a passkey on the link page and get a fresh account; anyone else must sign in to an existing account. | Keeps a personal bot closed to strangers. |
| D5 | **The `BotModule` registry becomes channel-neutral and is shared by every channel** (approach A). Commands are parsed once. | No per-channel copy of argument parsing or formatting. |
| D6 | **Discord slash commands take a single `args` string** (`/todo args:"add buy milk"` ≡ `!todo add buy milk`). Typed slash options are a later improvement. | Reuses the registry unchanged; YAGNI. |
| D7 | **Email is a full channel (plan D): IMAP polling inbound, SMTP submission outbound.** Inbound commands require DMARC alignment from the mailbox provider's `Authentication-Results`. | No public endpoint or provider lock-in; `From` is forgeable, so authentication is mandatory. |
| D8 | **An email body is commands + AI.** A first line that parses as a command dispatches deterministically; anything else is an AI prompt, as in a Matrix DM. | Same behaviour as a direct message on every other channel. |

## 3. Types

### 3.1 Domain: `domain/channels/` (pure)

The application layer (identity, linking) and `domain/events::Destination` need these, and `application` may not import `interface`, so they live in the domain.

```rust
pub enum Channel { Matrix, Discord, Telegram, Email }   // as_str: "matrix" | "discord" | "telegram" | "email"

pub struct PlatformUserId(String);                        // validated per channel: MXID / snowflake / i64 / lowercase addr-spec
pub struct PlatformIdentity { channel: Channel, user_id: PlatformUserId, display_name: Option<String> }

pub enum ConversationAddress {
    Web,
    Matrix(String),          // "!room:server"
    Discord(u64),            // channel id (guild channel or DM channel)
    Telegram(i64),           // chat id
    Email(String),           // counterpart address; threading travels in reply_to
}

pub enum ProvisioningPolicy { ProvisionUnknown, RequireLink }
pub fn provisioning_policy(channel: Channel) -> ProvisioningPolicy   // Matrix → ProvisionUnknown, rest → RequireLink
```

`ConversationAddress` has two encodings, each with exactly one parse/format pair:

- **Storage form** (the existing `room_id` columns on todos, reminders, transactions, ai_conversations, recurring_bills, …): `web`, a bare Matrix room id `!…`, or `discord:<id>` / `telegram:<id>` / `email:<addr>`. A Matrix room id carries its own `!` sigil, so existing rows are already valid storage form and **no data-rewriting migration is needed**. *Ruling R-O1:* bare Matrix ids stay canonical in storage — a rewriting migration over every `room_id` column buys nothing — cost if wrong: one more prefix to strip in a future migration.
- **Destination form** (`outbox_deliveries.destination`): always `<channel>:<id>`; the existing `matrix:<room>` keys are unchanged.

`Channel` variants exist only for channels that are implemented in this sub-project (Email is plan D, so it is included).

### 3.2 Interface: `interface/channels/`

```text
interface/channels/
  mod.rs          re-exports; module docs
  message.rs      IncomingMessage, OutgoingMessage, Content, Action, MessageRef, Visibility
  adapter.rs      ChannelAdapter trait, ChannelError, SentRef
  registry.rs     ChannelRegistry
  inbound.rs      the shared inbound pipeline (§5)
  render/         html.rs (subset sanitizer), markdown.rs (Discord), telegram_html.rs, plain.rs
  matrix/  discord/  telegram/  email/
```

```rust
pub struct IncomingMessage {
    pub channel: Channel,
    pub conversation: ConversationAddress,
    pub sender: PlatformIdentity,
    pub message_id: MessageRef,              // opaque, adapter-defined
    pub text: String,
    pub reply_to: Option<MessageRef>,
    pub quoted: Option<domain::chat::QuotedMessage>,
    pub is_direct: bool,
    pub is_mentioned: bool,
}

pub struct OutgoingMessage {
    pub target: Target,                      // Conversation(ConversationAddress) | Direct(PlatformIdentity)
    pub content: Content,
    pub reply_to: Option<MessageRef>,
    pub visibility: Visibility,              // Normal | Private
}

pub struct Content {
    pub plain: String,
    pub html: Option<String>,                // the rich format: the HTML subset modules already emit
    pub attachments: Vec<Attachment>,        // { kind: Audio|Image|File, mime, filename, bytes }
    pub actions: Vec<Action>,                // { label, command } — a command string re-dispatched as the clicker
}
```

There is **no generic metadata map**. Platform-specific data (Matrix event content, Discord interaction token, Telegram `message_id`, email `Message-ID`/`References`/`Subject`) stays inside the adapter, reachable through the opaque `MessageRef` the adapter minted. *Ruling R-O2:* a metadata bag invites email-shaped fields into every channel — cost if wrong: a field added to `OutgoingMessage` later.

`Visibility::Private` marks a reply carrying private data (portfolio, expenses, journal, settings, account). Rendering per channel: Discord guild interaction → ephemeral; Telegram group → refused with "message me privately"; Matrix → unchanged (today's behaviour, rooms are already authorized); Email → always private.

### 3.3 Rich content

Modules keep emitting the HTML subset they emit today (`b i u code pre br p ul ol li a blockquote`), always via `html_escape()` for user values. Each adapter owns a renderer:

| Channel | Rendering |
|---|---|
| Matrix | `text_html(plain, html)` unchanged |
| Telegram | HTML parse mode; `ul/li/br/p` mapped to text; unknown tags stripped; 4096-char split |
| Discord | HTML → Markdown; embed for replies with a title line; 2000-char / embed limits |
| Email | `multipart/alternative`: the HTML wrapped in a minimal template + `plain` |

Renderers are pure functions with table tests.

## 4. Adapter and registry

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn channel(&self) -> Channel;
    fn is_connected(&self) -> bool;
    /// Long-lived: connects, receives, reconnects with bounded backoff, returns when `cancel` fires.
    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()>;
    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>;
}

pub enum ChannelError {
    NotConnected,        // job defers (today's MATRIX_RETRY semantics), outbox retries
    Rejected(String),    // permanent: bad target, forbidden, too large
    Transient(String),   // retry with backoff
}
```

A trait object is justified: four implementations, chosen at boot from configuration.

`ChannelRegistry` holds `HashMap<Channel, Arc<dyn ChannelAdapter>>`, built once in `app.rs` from the adapters whose configuration is present. It:

- routes `send(OutgoingMessage)` by the target's channel (`NotConnected` when that channel is not configured or not connected);
- spawns each adapter's `run` under a child `CancellationToken` and joins it into `app::run`'s bounded shutdown;
- **replaces** `interface/jobs::Delivery`/`MatrixDelivery` (job handlers take `Arc<ChannelRegistry>`, or a narrow `Outbound` trait the registry implements so handler tests keep their fake) and the outbox's `MatrixRoomSink` (one `ChannelSink` delivers `Destination::Conversation` through the registry; webhook-alert room authorization stays a Matrix-adapter check).

It contains no business logic. Adapters receive `AppServices` and the registry handle as constructor parameters; none builds a service, pool or LLM client.

## 5. Inbound pipeline (shared)

`interface/channels/inbound.rs` is the one place every adapter hands an `IncomingMessage` to:

1. **Parse.** Command syntax is channel-specific and is the adapter's job: Matrix `!todo …` or a mention; Telegram `/todo …` (`@botname` suffix stripped); Discord interaction `todo` + `args`; Email first body line. The adapter yields `command` + `raw_args`, or "free text".
2. **`link` first.** `link` is handled before identity resolution (the existing load-bearing ordering, `AGENTS.md` §3.4), never provisions, and refuses a non-direct conversation.
3. **Resolve.** `IdentityService::resolve(&sender)` applies the channel's `ProvisioningPolicy`. `Unlinked` → only `start`/`help`/`link` are answered; everything else gets "link this account first: send `/link`".
4. **Dispatch.** Build the neutral `CommandContext` and run `ModuleRegistry::dispatch`; free text in a direct conversation (or with a mention) goes to the AI module, exactly as a Matrix DM does today.
5. **Reply.** The module's `Reply` becomes an `OutgoingMessage` (reply-to the incoming message) sent through the adapter, with progress reported through the adapter's `ProgressSink` while the agent runs.

`CommandContext` becomes channel-neutral: `conversation: ConversationAddress`, `channel`, `owner_id`, `sender: PlatformIdentity`, `command`, `raw_args`, `full_body`, `is_direct`, `is_mentioned`, `quoted`, `progress: Option<Arc<dyn ProgressSink>>`. `room_id`/`sender_mxid` disappear; the two commands that act on a Matrix account read `sender` when `channel == Matrix`.

`BotModule::handle_command` returns `Option<Reply>` where `Reply { content: Content, visibility: Visibility }`; `error_reply` returns a `Reply`.

Channel-specific features stay inside their adapter: Matrix E2EE, verification, invites, reactions and `bot_message_targets`, typing indicators, image/voice attachment ingestion (OCR, STT); Discord deferral; Telegram callback queries; email threading and DMARC.

## 6. Agent boundary

`llm/` must not import `matrix_sdk`, `serenity`, `teloxide`, `lettre`, an IMAP crate, or any `interface::channels::<platform>` module (architecture test). Today it does; the plan removes:

- `StageReporter` → a `ProgressSink` trait defined in `llm/` (`report_stage`, `report_tool_call`, `push_delta`, `flush`). Matrix implements it by editing its stage message; Telegram by `editMessageText`; Discord by editing the deferred interaction response; Email by a no-op.
- `interface::channels::matrix::quote::QuotedMessage` imports → `domain::chat::QuotedMessage` (already neutral).
- `reaction_targets::record_ai_tool_target` inside `llm/turn.rs` → the targets are returned in `AiTurnResult`, and the Matrix adapter records them.
- `llm/alert.rs` (`matrix_sdk::Client`) → the AI-error alert is an `OutgoingMessage` to the configured dev conversation through the registry.

`interface/chat/{tts,account,llm}` lose their `matrix_sdk::Client` handles: TTS returns an audio `Attachment` that the adapter uploads (Matrix: MSC3245 voice; Telegram: `sendVoice`; Discord: file; Email: attachment); account announcements become `OutgoingMessage`s.

## 7. Identity and linking

### 7.1 Schema — migrations `0006` (DDL), `0007` (data), `0008` (drop)

```sql
-- 0006_platform_identities.sql
CREATE TABLE platform_identities (
    id               BIGSERIAL PRIMARY KEY,
    account_id       UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    platform         TEXT NOT NULL CHECK (platform IN ('matrix','discord','telegram','email')),
    platform_user_id TEXT NOT NULL,
    display_name     TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform, platform_user_id)
);
CREATE INDEX idx_platform_identities_account ON platform_identities(account_id);

CREATE TABLE link_tokens (
    token_hash       TEXT PRIMARY KEY,          -- SHA-256 of the token; the token itself is never stored
    platform         TEXT NOT NULL,
    platform_user_id TEXT NOT NULL,
    display_name     TEXT,
    conversation     TEXT NOT NULL,             -- storage-form address, for the confirmation message
    created_at       TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,      -- created_at + 10 minutes
    consumed_at      TIMESTAMPTZ
);
```

`0007_platform_identities_backfill.sql` starts with a guard that names any `matrix_links` row whose `account_id` has no `accounts` row, copies every `matrix_links` row as `platform = 'matrix'`, then drops `matrix_links`; `0008_drop_matrix_link_challenges.sql` drops the old challenge table once the web-minted flow is deleted. It is rehearsed on a production copy before release (`AGENTS.md` §6) and the cutover runbook gains a step.

Merge bookkeeping (`domain/accounts`): `platform_identities` joins `ACCOUNT_ID_FOLLOWS` (replacing `matrix_links`), `link_tokens` has no `account_id` and is not listed, `matrix_link_challenges` leaves `ACCOUNT_ID_DISCARDED`. The schema-catalog test keeps the lists honest.

### 7.2 Services

- **`IdentityService`** (`application/identity`): `resolve(&PlatformIdentity) -> Resolution { Account(AccountId) | Unlinked }` (provisions under `ProvisionUnknown`, refreshing `display_name`); `list(account)`; `unlink(account, channel, user_id)` (refuses removing the last way into an account that has no passkey). Replaces `AccountService::{resolve_or_provision, find_by_mxid, linked_mxids, unlink, issue_link_challenge, claim_link}`.
- **`LinkService`** (`application/identity/link.rs`):
  - `start(identity, conversation, is_direct) -> Result<LinkStart, AppError>` — refuses non-direct; mints a 32-byte random token (base64url), stores its hash with a 10-minute expiry, returns the token. One live token per identity: starting again consumes the previous one.
  - `inspect(token, viewer: Option<AccountId>) -> LinkPreview` — `Bind`, `AlreadyYours`, `NeedsMerge { preview }`, `SignupAllowed`, `SignInRequired`, `Expired`.
  - `claim(token, account) -> LinkOutcome` — in one transaction: lock the token row (`FOR UPDATE`), check unexpired/unconsumed, bind or merge (existing `merge_within`), consume the token, write the `identity_linked` security event, and publish `SystemEvent::IdentityLinked` with destination `Destination::Conversation(token.conversation)` so the chat receives "✓ linked".
  - `claim_with_signup(token, passkey registration)` — only when the identity is on its channel's allowlist (D4); creates the account and binds in the same transaction as the credential.
  - The in-memory `PendingMerges` map in `BotHandles` is deleted: a pending merge is the unconsumed token.

### 7.3 HTTP and web

| Route | Auth | Purpose |
|---|---|---|
| `GET /api/v1/account/identities` | `AuthUser` | list linked identities (replaces `GET /account/matrix`) |
| `POST /api/v1/account/identities/unlink` | `VerifiedUser` + `SENSITIVE` | replaces `/account/matrix/unlink` |
| `POST /api/v1/link/inspect` | optional session | `LinkPreview` for the landing page |
| `POST /api/v1/link/claim` | `VerifiedUser` + `SENSITIVE` | bind or confirm merge |
| `POST /api/v1/link/signup/{start,finish}` | none (token + allowlist), rate-limited | passkey registration that creates the account |

The four `/account/matrix/*` routes are removed. This is an **intentional OpenAPI change**: `backend/openapi.json`, the generated frontend client and `frontend/e2e/fake-api.ts` are regenerated/updated in the same task. The frontend gains a `/link` route (landing page: inspect → sign in / sign up / confirm merge → done) and the Security panel lists identities of every channel.

## 8. Channels

### 8.1 Matrix (plan A)

`MatrixAdapter` wraps today's `MatrixBot`: sync loop, E2EE, `ensure_self_verified` on every boot, verification, invites, reactions, attachments. Text events become `IncomingMessage`s fed to the shared pipeline; `send` covers `Conversation(Matrix)` (join-then-send, as `MatrixDelivery` does today) and `Direct(matrix identity)` (today's `send_dm_html`). The Matrix `ProgressSink` is the existing stage indicator. Behaviour is frozen: every existing Matrix test passes unchanged in meaning.

### 8.2 Discord (plan B) — `serenity`

- Gateway intents: `GUILDS | DIRECT_MESSAGES` (slash commands need no `MESSAGE_CONTENT`).
- Commands registered at boot: globally, or to `DISCORD_GUILD_ID` when set (instant updates in development): `/todo /expense /portfolio /ask /link /help`, each with one optional string option `args` (`/ask` has a required `prompt`).
- Every interaction is deferred immediately (Discord's 3-second limit) — ephemeral when the command's reply could be private and the interaction is in a guild — then completed with an edit of the original response; long agent turns report progress through that edit.
- `is_direct` = interaction in a DM channel. `/link` refuses guild channels.
- Replies are embeds (title + description) using the Markdown renderer; colours follow `Design.md`'s neutral palette.
- Config: `DISCORD_TOKEN`, `DISCORD_GUILD_ID` (optional), `DISCORD_ALLOWED_USERS`. The adapter is registered only when `DISCORD_TOKEN` is set.

### 8.3 Telegram (plan C) — `teloxide` types and `Bot`

- Production: `POST /api/webhooks/telegram`, outside `/api/v1`, verifies `X-Telegram-Bot-Api-Secret-Token` against `TELEGRAM_WEBHOOK_SECRET` in constant time, is rate-limited, acknowledges `200` immediately and processes the update on a bounded queue. The webhook URL and secret are registered with `setWebhook` at boot.
- Development: `TELEGRAM_MODE=polling` runs `getUpdates` long polling inside the adapter's `run`.
- Commands (registered with `setMyCommands`): `/start /help /todo /expense /portfolio /link`. Free text in a private chat goes to the agent; in groups the bot answers only commands and mentions (privacy mode).
- Inline keyboards render `Content.actions`; `callback_data` is the action's command string (≤ 64 bytes, else the action is dropped and logged). A callback is re-dispatched through the inbound pipeline **as the user who pressed it** — the data is never trusted as an authority. The first producer is the todo list's "Done" button.
- The existing Bot-API *emulation* endpoint `interface/http/webhooks_telegram.rs` (`/bot{token}/sendMessage`, an inbound-webhook compatibility shim) is renamed `interface/http/webhooks/telegram_compat.rs`; its contract is unchanged.
- Config: `TELEGRAM_BOT_TOKEN`, `TELEGRAM_MODE` (`webhook` | `polling`, default `webhook`), `TELEGRAM_WEBHOOK_SECRET` (required in webhook mode; `Config::validate` refuses to boot without it), `TELEGRAM_ALLOWED_USERS`.

> **Spec amendments (plan C, 2026-10-03; rulings R102–R117):**
> - **T1 (R102):** Dependency is `teloxide-core` 0.13 (`rustls`), not the full `teloxide` framework; brings `reqwest` 0.12 with `takecell` pinned to 0.1.1 for rustc 1.94 compatibility.
> - **T2 (R103):** Commands reach the registry as bare `full_body` without `/` (matching Discord R85); `/todo@BotName` mention suffix is stripped; any typed `/name` is parsed as a command, with the 6 core commands advertised via `setMyCommands`.
> - **T3 (R104):** Addressed text: in groups/supergroups, only commands, `@bot` mentions, and direct replies to bot messages are processed; channels and edited messages are ignored.
> - **T4 (R105):** `/start` for an unlinked user answers with link guidance; for a linked user, if unhandled it redispatches once as `help`.
> - **T5 (R106):** Replied-to messages populate `quoted = QuotedMessage`, supporting reply-as-body for commands.
> - **T6 (R107):** Callbacks are answered at once via `answerCallbackQuery` and re-dispatched through the inbound pipeline as the user who pressed the button (invariant 9).
> - **T7 (R108):** `Content.actions` render as an inline keyboard (max 8 rows of 1 button, callback data ≤ 64 bytes). Todo list generates `Done #<id>` buttons for at most 10 open items.
> - **T8 (R109):** In groups/supergroups, `Visibility::Private` replies are refused with `PRIVATE_IN_GROUP` ("That answer is private — message me directly.") and never posted publicly (invariant 10).
> - **T9 (R110):** Webhook ingress at `POST /api/webhooks/telegram` verifies `X-Telegram-Bot-Api-Secret-Token` in constant time before parsing the body; 120/min rate limit per IP; returns 200 on enqueue, 503 on queue full.
> - **T10 (R111):** Constant-time comparison implemented in `utils::secure::constant_time_eq`.
> - **T11 (R112):** 4 worker tasks drain a bounded queue (32 slots) fed by either webhook ingress or long polling; updates deduplicated by `RecentUpdates` (1024-id sliding window).
> - **T12 (R113):** Webhook route is not in OpenAPI (infrastructure endpoint). Compat shim `/bot<token>/sendMessage` moved to `interface/http/webhooks/telegram_compat.rs` with OpenAPI schema unchanged.
> - **T13 (R114):** `Config::validate` requires `TELEGRAM_WEBHOOK_SECRET` and HTTPS `WEB_URL` in webhook mode; validates `TELEGRAM_ALLOWED_USERS` entries as positive i64.
> - **T14 (R115):** Telegram Bot API has no idempotency keys; outbox deliveries are at-least-once.
> - **T15 (R116):** Progress edits a status message at most every 1.5 s and sends chat action typing indicators every 4 s in direct chats.
> - **T16 (R117):** Attachments: Ogg Opus voice notes sent via `sendVoice`; other audio `sendAudio`; photos `sendPhoto`; documents `sendDocument`. Files over 50 MB produce a text warning notice.

### 8.4 Email (plan D) — IMAP inbound, SMTP outbound

- **Inbound:** the adapter's `run` polls `EMAIL_IMAP_URL` (IMAPS only) every `EMAIL_POLL_SECONDS` (default 60): fetch `UNSEEN`, parse with `mail-parser`, process, mark `\Seen` (moving to a processed folder when `EMAIL_PROCESSED_FOLDER` is set). Bounded: at most N messages per poll, body capped (the 500-char command cap applies to the command; AI prompts use the existing prompt cap), attachments ignored.
- **Authentication:** a message is accepted only if the `Authentication-Results` header added by `EMAIL_AUTHSERV_ID` reports `dmarc=pass` for the `From` domain. Anything else is dropped and logged (no reply, so forged mail cannot make the bot spam a victim). Auto-replies (`Auto-Submitted`, `Precedence: bulk/list`, mailer-daemon) are dropped.
- **Mapping:** sender = `PlatformIdentity(Email, lowercase addr-spec)`; conversation = `Email(addr)`; `message_id` = `Message-ID`; `reply_to` = `In-Reply-To`; `is_direct = true`; text = the first `text/plain` part with quoted history and signature stripped (falls back to HTML → text).
- **Outbound:** `lettre` SMTP submission (`EMAIL_SMTP_URL`, STARTTLS/TLS required) from `EMAIL_FROM`; replies set `In-Reply-To`/`References` and `Re: <subject>`; unsolicited messages (reminders, briefings, link confirmation) use a subject derived from the first line of `plain`. `Content.actions` render as a "reply with: `todo done 12`" list.
- **Linking:** `link` by email mails the token URL back to the `From` address — proof of mailbox control even if the inbound message were forged.
- Config: `EMAIL_IMAP_URL`, `EMAIL_SMTP_URL`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`, `EMAIL_FROM`, `EMAIL_AUTHSERV_ID`, `EMAIL_ALLOWED_SENDERS`, optional `EMAIL_POLL_SECONDS`, `EMAIL_PROCESSED_FOLDER`. Registered only when `EMAIL_IMAP_URL` is set.

## 9. Outbound routing (D3)

- Job payloads that carry a delivery target (`reminder.fire`, `briefing.send`, `portfolio.check_alerts`) keep the storage-form address they carry today; handlers parse it into a `ConversationAddress` and send through the registry. `web` targets and unparseable addresses behave as today (logged, treated as delivered).
- Direct notifications (today `send_dm(mxid)`) become `Target::Direct(identity)`, choosing the account's identity on the channel the triggering record came from, falling back to its Matrix identity.
- `domain/events::Destination` gains `Conversation(ConversationAddress)`; `MatrixRoom(String)` becomes `Conversation(ConversationAddress::Matrix(_))` with the identical `matrix:<room>` key, so in-flight outbox rows stay readable.

## 10. Errors

Each adapter maps its client errors (`matrix_sdk::Error`, `serenity::Error`, `teloxide::RequestError`, `lettre` / IMAP errors) to `ChannelError` at its own boundary; none reaches `application` or `domain`. Inbound handling failures become replies only via `error_reply`; database/external details are logged, never shown. Rate limits (Discord 429, Telegram `RetryAfter`) map to `Transient` with the provider's delay honoured up to a bound.

## 11. Security invariants (each needs a test)

1. No owner key is ever derived from a platform id: every inbound path goes through `IdentityService::resolve`.
2. An `Unlinked` identity reaches no module except `start`/`help`/`link`.
3. `link` refuses non-direct conversations and burns nothing it did not mint.
4. A link token is single-use, expires after 10 minutes, is stored only as a hash, and a claim locks its row.
5. Signup through a token is refused for an identity not on its channel's allowlist.
6. `/link/claim` requires `VerifiedUser` and is in `SENSITIVE`.
7. A Telegram webhook without the correct secret is refused before parsing.
8. An email without `dmarc=pass` from the configured authserv-id is never processed and never answered.
9. A Telegram callback is dispatched as the presser, never as the identity that produced the button.
10. A `Private` reply is never posted visibly to a Discord guild channel or a Telegram group.

## 12. Testing

- **Architecture** (`tests/architecture.rs`): `domain/channels` pure; `application` and `llm` import no platform crate and no `interface::channels::<platform>`; `interface/chat` imports no platform crate; `interface/channels/**` runs no SQL; only `app.rs` constructs adapters and the registry.
- **Domain:** `ConversationAddress` both encodings round-trip; `PlatformUserId` validation per channel; provisioning policy.
- **Repositories:** `platform_identities` round-trip, `UNIQUE(platform, platform_user_id)`, owner isolation; `link_tokens` consume-once.
- **Services:** resolve per policy for all four channels → the same `AccountId` once linked; link start/inspect/claim/merge/signup/expiry/reuse; mid-transaction failure leaves the token unconsumed; `IdentityLinked` event published.
- **Migration:** `0007` backfill on a seeded legacy fixture; guard fires on an orphan.
- **Normalization (pure, fixture-driven, no network):** Matrix event → `IncomingMessage`; Discord interaction JSON → `IncomingMessage`; Telegram `Update` JSON → `IncomingMessage`; raw RFC 5322 mail → `IncomingMessage` (incl. DMARC pass/fail, auto-reply, quoted-history stripping). Fixtures under `contracts/channels/`.
- **Outbound (pure render functions):** `OutgoingMessage` → Matrix content, Discord embed/response, Telegram `sendMessage` params + keyboard, MIME message. No test contacts a homeserver, Discord, Telegram or a mail server.
- **Inbound pipeline:** fake adapter end-to-end through the real registry and a test database.
- **Existing Matrix suite** stays green throughout.

## 13. Plans

| Plan | Scope | Written |
|---|---|---|
| **A** `plans/2026-10-01-omnichannel-a-framework.md` | domain/channels; message/adapter/registry; neutral `CommandContext`/`Reply`; agent decoupled; migrations 0006–0008; `IdentityService`/`LinkService`; link HTTP + web page; Matrix onto `MatrixAdapter`; registry replaces `Delivery` and `MatrixRoomSink`; docs | done (2026-10-02) |
| **B** Discord `plans/2026-10-02-omnichannel-b-discord.md` | §8.2 | done (2026-10-02) |
| **C** Telegram `plans/2026-10-02-omnichannel-c-telegram.md` | §8.3 | done (2026-10-03) |
| **D** Email `plans/2026-10-03-omnichannel-d-email.md` | §8.4 | done (2026-10-03) |

Each later plan is written against the code A produced, so it cites real types instead of guesses.

## 14. Out of scope

Typed Discord slash options; Discord buttons; media ingestion (receipts, voice) on Discord/Telegram/Email; a preferred notification channel setting and UI (sub-project 4); making `account_rooms` channel-neutral (it remains Matrix room authorization for webhook alerts); Slack/Signal/WhatsApp; storing attachments (`StorageService`, sub-project 3).
