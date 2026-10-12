# Matrix Bot (Rust) 🦀

A personal AI/life workspace built around Matrix: a chat bot (Matrix, plus optional Discord, Telegram and Email), an embedded REST/SSE API, an MCP server and a React web UI in **one binary**. Built on [`matrix-sdk`](https://github.com/matrix-org/matrix-rust-sdk), [`axum`](https://github.com/tokio-rs/axum), [`sqlx`](https://github.com/launchbadge/sqlx) and [`webauthn-rs`](https://github.com/kanidm/webauthn-rs), with PostgreSQL for storage.

Every feature is reachable through several surfaces (chat commands, the web UI, AI tool calls and MCP) and all of them land on the same data.

---

## 🌟 Features

- 🤖 **Core**: greetings (`!hello`), latency ping (`!ping`), module-driven help (`!help`).
- 💬 **Channels**: Matrix (end-to-end encrypted) plus optional Discord, Telegram and Email adapters behind one inbound pipeline; one account links identities from all of them. Setup is under *Enabling Discord / Telegram / Email* below.
- 📋 **To-dos**: priorities, due dates, lead-time reminders, personal and room-wide views, editing, pagination, and quoted-message capture (replying `!todo add` uses the quoted text).
- ⏰ **Reminders**: one-off and recurring (daily, weekly, monthly, yearly), persisted across restarts with downtime catch-up and exponential retry backoff.
- 💰 **Finance**: expense and income ledger with custom categories and monthly summaries. Amounts are integer minor units throughout, with no floating point.
- 📈 **Portfolio**: holdings with weighted-average cost basis, realized and unrealized P&L, live price fetching, price alerts, cash-balance gating, and a tamper-evident SHA-256 audit chain.
- 🧾 **Recurring bills**: schedules, advance reminders, auto-pay marking and payment history.
- 🔖 **Bookmarks**: URL saving with OpenGraph scraping, reading-time estimates, tags, favorites and archive.
- 🎯 **Habits**: daily and weekly tracking, streaks, completion heatmaps.
- ⏳ **Life grid and journal**: lifespan visualization, daily mood/weather/notes, cross-module activity timelines.
- 📍 **Footprints**: travel and location check-ins with ratings and journey statistics.
- 🔔 **Webhooks**: inbound alert gateway with severity cards, Telegram Bot API drop-in compatibility and a durable retry queue. See [`docs/api.md`](docs/api.md#inbound-webhook-gateway).
- 🗒️ **Notes and knowledge base**: Markdown notes with tags, captured from any channel (including voice and photo capture) and found with hybrid keyword + semantic search (pgvector); Markdown zip export and import. See [`docs/llm.md`](docs/llm.md#knowledge-search-and-embeddings).
- 🧠 **AI assistant and web search**: multi-turn threads, tool calling over your own data, live search and fact-checking (Tavily, with DuckDuckGo fallback), per-user model configuration, and weekly retrospectives. See [`docs/llm.md`](docs/llm.md).
- 🛰️ **Autonomous agent**: scheduled and on-demand tool-calling runs with a step budget; sensitive actions pause for your approval and run at most once. See [`docs/llm.md`](docs/llm.md#agent-runs-use-native-tool-calling).
- 🔌 **MCP server**: exposes the same tools, resources and prompts to Claude Desktop, Cursor and other MCP clients over stdio or HTTP/SSE.
- 🔊 **Voice replies**: `!say <text>` speaks into the room as a native Matrix voice message (Ogg Opus), synthesized by the bundled Kokoro server or by OpenAI/ElevenLabs. Each user picks a voice.
- ⚙️ **Settings**: per-user timezone, default currency and voice, respected by every module.
- 🛡️ **Admin Console & Encrypted Secrets Vault**: 20 hot-reloadable runtime settings (`ArcSwap` snapshot) and an AES-256-GCM credentials vault with live provider probes, gated by CLI-bootstrapped `admin` role and 5-minute WebAuthn passkey step-up on mutations.
- 🔐 **Accounts and security**: passkey (WebAuthn) sign-in, linked Matrix identities, step-up assertions, delayed recovery and an audit log.
- 🗄️ **PostgreSQL 18** as the only application database, with versioned migrations applied at boot. See [`docs/database.md`](docs/database.md).
- 📦 **Decoupled Object Storage**: binary media (receipts, audio clips, attachments) stored in local filesystem or S3/R2/MinIO, isolated by account with presigned streaming.

---

## 🏛️ How it works

One capability is implemented once and reached through every interface:

```
                    ┌── Chat: Matrix, Discord, Telegram, Email
                    ├── REST API + SSE
 User / LLM ───────►├── MCP
                    ├── AI tool calls
                    └── Web UI
                           │
                      Application   (services: use cases, transactions, events)
                           │
                        Domain      (pure rules and invariants)
                           │
                      PostgreSQL
```

- **Two ways in.** Chat messages arrive as commands (`!todo …` on Matrix and Email, `/todo …` on Discord and Telegram; parsed deterministically, no LLM, zero token cost) or as natural language routed to the LLM, which calls tools. The embedded Axum server serves `/api/v1/*` and a live SSE stream at `/api/v1/events` for the web UI.
- **One implementation.** Chat commands, REST handlers, MCP and AI tools all call the same application services over `domain/` rules, so validation, ownership isolation and events are never duplicated.
- **Live updates.** Every mutation publishes a typed `SystemEvent` through the transactional outbox, delivered per owner over SSE.
- **Modular monolith.** No queues or microservices. Optional local OCR, speech-to-text, text-to-speech and text embeddings run in a separate `inference` server the bot reaches over HTTP; a missing one disables only that feature.

The reasoning behind the design is in [`docs/architecture.md`](docs/architecture.md).

---

## 🔐 Accounts and security

Every todo, reminder, transaction, bill, habit, bookmark and holding belongs to an **account**. Messaging the bot in Matrix and browsing the web are one owner with one dataset.

- **Sign-in:** passkeys only (WebAuthn), one button, no username or password. Sessions are server-side; only the SHA-256 of the cookie is stored. Programmatic clients send `Authorization: Bearer <token>`, and MCP clients use a personal access token (`mb_mcp_…`).
- **Passkeys:** attach several per account. `!passkey add` (alias `!web`) in a DM mints a 10-minute enrollment link for an account with no passkey yet; `!passkeys` lists them.
- **Linking:** send `!link` (Matrix, Email) or `/link` (Discord, Telegram, Email) to the bot **in a direct message**; it answers with a link that expires in 10 minutes and works once. Open it in the browser and confirm with a passkey. The direct message proves you own the chat identity and the passkey proves you own the account, so nothing is typed into either side. Linking an identity that already has data shows what will be merged before you confirm, and the bot confirms in the chat afterwards. An identity on a signup allowlist (below) may instead create a new account from that link. The old web-minted `!link <code>` is gone.
- **Step-up:** sensitive actions (managing passkeys, unlinking, signing out other devices, recovery codes) need a passkey assertion from the last 5 minutes.
- **Recovery:** ten single-use codes, or `!recover` from a linked Matrix account, which is announced immediately, waits 24 hours, can be cancelled from any signed-in device, and then grants a restricted session for 72 hours.
- **Audit:** every sensitive action is logged per account and sent as a DM to each linked identity.

Matrix cannot prompt for a passkey, so `!export`, `!sessions`, `!recoverycodes`, `!unlink` and `!passkey rm` are refused there and point to the web app. Details and local testing without a hardware key: [`docs/api.md`](docs/api.md#authentication).

---

## 📋 Command reference

Every feature module exposes chat commands, formatted in HTML for rich Matrix clients (Element, FluffyChat, Cinny) and rendered natively on the other channels. The table shows the Matrix spelling; Telegram and Email also accept `/` in place of `!`, and Discord offers six slash commands (`/todo /expense /portfolio /ask /link /help`) that take the rest of the command as one `args` text:

| Module | Commands | Description |
|---|---|---|
| **Core** | `!hello / !hi [name]`, `!ping`, `!help` | Greetings, ping/latency diagnostic, dynamic module command help |
| **To-Do** | `!todo add [[high\|low]] <task>`<br>↳ *Or quote reply:* `!todo add` (uses quoted body)<br>`!todo [list\|room] [page]`<br>`!todo done <id>`<br>`!todo priority <id> <low\|medium\|high>`<br>`!todo due <id> <time> [remind <lead>]`<br>`!todo edit <id> <new text>`<br>`!todo clear [room]` | Task management with priorities, due dates, advance reminders, room-wide or personal scoping, and pagination |
| **Reminder** | `!remind [daily\|weekly] <time> <message>`<br>↳ *Or quote reply:* `!remind <time>` (uses quoted body)<br>`!remind list`<br>`!remind cancel <id>` | One-off and recurring reminders, scheduled in user timezone with automatic restart recovery |
| **Finance** | `!expense add <amount> [CUR] <note> [-c <cat>]`<br>`!income add <amount> [CUR] <note> [-c <cat>]`<br>`!expense list [page]`<br>`!expense summary`<br>`!expense delete <id>`<br>`!category [add\|list\|remove] <name>` | Double-entry-style ledger, category breakdowns, monthly summaries, integer minor units (no float rounding errors) |
| **Portfolio** | `!portfolio [list]`<br>`!portfolio summary`<br>`!portfolio buy <sym> <qty> @ <price> [CUR]`<br>`!portfolio record <sym> <qty> @ <price> [CUR]`<br>`!portfolio sell <sym> <qty> @ <price>`<br>`!portfolio price <sym> <price>`<br>`!portfolio refresh`<br>`!portfolio note <sym> [text]`<br>`!portfolio remove <sym>`<br>`!portfolio cash [deposit\|withdraw] <amt> [CUR]` | Holdings tracking with cash balance gating, market value, realized P&L, manual price recording or automated live quotes (`!portfolio refresh`), and notes |
| **Recurring Bills** | `!bill add <name> <amt> <cur> <cat> [frequency]`<br>`!bill list`<br>`!bill pay <id>`<br>`!bill remind <id> <minutes\|off>`<br>`!bill delete <id>` | Fixed schedules, instalment tracking, advance due date reminders, and payment history |
| **Bookmarks** | `!save <url> [tag1,tag2]`<br>`!bookmarks [tag]`<br>`!bookmark fav <id>`<br>`!bookmark archive <id>`<br>`!bookmark rm <id>` | Save web links with OpenGraph title & description extraction, estimated reading time, tagging, favorites, and archiving |
| **Habits** | `!habit [list]`<br>`!habit add <name>`<br>`!habit done <name\|id>`<br>`!habit undo <name\|id>`<br>`!habit streak`<br>`!habit delete <id>` | Daily habit tracking, one-tap checkoffs, streak counters, and streaks leaderboard |
| **Life Grid** | `!life stats`<br>`!life today`<br>`!life set-birth <YYYY-MM-DD> [lifespan]`<br>`!life log <mood> [note]` | Memento Mori lifespan perspective (% elapsed, weeks left), daily status, and mood reflections |
| **Daily Log** | `!journal [date]` (or `!daily`)<br>`!journal log <mood> [\| weather] [\| note]`<br>`!journal activities [date]`<br>`!journal clear [date]` | Daily retrospective logging, mood tracking, weather notes, and cross-module daily activity summaries |
| **Footprints** | `!footprint list`<br>`!footprint add <name> [\| date] [\| rating] [\| note]`<br>`!footprint delete <id>` | Visited places log, travel timeline, ratings (1-5 stars), and journey statistics |
| **Webhooks** | `!webhook list`<br>`!webhook create <name> [room]`<br>`!webhook test <id>`<br>`!webhook revoke <id>` | Inbound webhook endpoints for external services, custom alert cards, and Telegram Bot API emulation |
| **AI Assistant** | `!ai <prompt>` or `@bot <prompt>`<br>↳ *Or quote reply:* `@bot fact check` / `true?` / `verify`<br>`!ai reset`<br>`!ai model [name]`<br>`!ai config`<br>`!ai retro` | Conversational LLM assistant, live web search & fact-checking (Tavily/DDG), quoted message analysis, model switching, and weekly retrospectives |
| **Notes** | `!note <text>` or `!note add <title> \| <content>`<br>↳ *Or send a voice note or photo captioned* `!note`<br>`!notes [tag]`<br>`!search <query>` | Capture Markdown notes from any channel; `!search` runs hybrid keyword + semantic search across notes, bookmarks, journal and footprints. Discord reaches notes through the assistant |
| **Agent** | `!agent run <goal>`<br>`!agent status [run_id]`<br>`!agent list`<br>`!agent confirm <approval_id>`<br>`!agent reject <approval_id> [reason]`<br>`!agent cancel [run_id]` | Start and supervise an autonomous run; sensitive tool calls wait for `confirm` |
| **Voice (TTS)** | `!say <text>` | Speaks the text into the room as a native Matrix voice message; voice chosen per user |
| **Settings** | `!settings`<br>`!settings timezone <IANA zone>`<br>`!settings currency <CODE>`<br>`!settings voice <voice\|none>` | Configure per-user timezone, default currency, and the voice `!say` speaks in (in whatever the configured TTS provider calls it), applied across all modules |
| **Account** | `!link` (`/link` on Discord, Telegram, and Email)<br>`!recover` | Get a 10-minute, single-use link that connects this chat account to your web identity (direct messages only), initiate delayed recovery |

`!export`, `!sessions`, `!recoverycodes`, `!unlink` and `!passkey rm` are
deliberately refused on Matrix and point at the web app (see *Accounts and security*).

---

## 🌐 REST API

Endpoints are mounted under `/api/v1`. Authentication is an HttpOnly session cookie (browser, with strict `Origin` checks) or `Authorization: Bearer <token>`. Errors are always `{ "error": "...", "code": 400 }`.

| Area | Base path | What it covers |
|---|---|---|
| Health | `/health/live`, `/health/ready` | Liveness and readiness probes (router root, no auth) |
| Auth | `/auth/*` | Passkey ceremonies, logout, profile, authenticated media, recovery-code redemption |
| Account | `/account/*` | Passkeys, linked chat identities (every channel), sessions, recovery, security events, AI config, webhook management |
| To-dos, reminders | `/todos`, `/reminders` | CRUD with filters and pagination |
| Finance | `/finance/*` | Transactions, summaries, categories, analytics |
| Portfolio | `/portfolio/*` | Holdings, trades, cash, prices and quotes, alerts, history, audit-chain verification |
| Recurring bills | `/recurring-bills` | Schedules and payments |
| Bookmarks, habits | `/bookmarks`, `/habits` | CRUD, stats, heatmaps |
| Life, journal, footprints | `/life/*`, `/footprints` | Lifespan grid, day entries, activity, check-ins |
| AI | `/ai/*` | Chat, conversations, connectivity check, retrospective |
| MCP | `/mcp/*` | HTTP/SSE transport and token management |
| Inbound gateway | `/webhooks/:token`, `/bot<token>/sendMessage` | Generic and Telegram-compatible webhook receivers |
| Link | `/link/*` | The landing page's inspect, claim and allowlisted signup calls |
| Notes, knowledge | `/notes`, `/knowledge/search` | Note CRUD, tags, Markdown export/import, hybrid search |
| Agent | `/agent/*` | Runs, approvals and scheduled tasks |
| Media | `/storage/:key`, `/ocr/scan`, `/voice/transcribe` | Stored binaries (session or signed URL), receipt OCR, speech-to-text |
| Settings, rooms | `/settings`, `/rooms` | Preferences and Matrix room picker |
| Admin | `/admin/*` | 20 hot-reloadable runtime settings, AES-256-GCM provider secrets, connectivity probes, audit log (`AdminSession` for reads, `AdminUser` 5-min step-up for writes/tests) |
| Live events | `/events` | SSE stream scoped to the authenticated owner |

`GET /api-docs/openapi.json` serves the generated OpenAPI 3.1 specification (committed as [`backend/openapi.json`](backend/openapi.json)); it is the authoritative endpoint list, and the frontend's TypeScript client is generated from it. Conventions, SSE events and authentication are in [`docs/api.md`](docs/api.md).

---

## 🛠️ Setup and running

You can run `matrix-bot` in any of the following ways:
1. **[Standalone Single Binary](#option-1-standalone-single-binary-recommended)** — zero-dependency executable with embedded Web SPA UI (fastest & easiest).
2. **[Docker Compose](#option-2-docker-compose-recommended-for-containers)** — containerized deployment with managed storage and healthchecks.
3. **[Docker Run](#option-3-docker-run)** — direct single-container command.
4. **[From Source (Development)](#option-4-development-from-source)** — development mode with hot-reloading.

---

### Configuration (`.env`)

Before running through any method, prepare your environment configuration:

```bash
cp .env.example .env
```

```env
# Matrix Bot Credentials
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USERNAME=your_bot_username
MATRIX_PASSWORD=your_bot_password
MATRIX_RECOVERY_KEY=optional_e2ee_recovery_key

# Database — PostgreSQL only; required, must be a postgres:// URL.
# matrix-sdk keeps its own SQLite crypto store under data/, unaffected by this.
DATABASE_URL=postgres://bot:bot@localhost:5432/matrix_bot

# Web & API Server
API_HOST=0.0.0.0
API_PORT=8080
# Registration Security Guard (Production defaults to closed)
INVITE_SECRET=optional_invite_secret_for_registration   # or API_REGISTRATION_SECRET
REGISTRATION_OPEN=false                                  # set true or pass --allow-open-registration to allow open registration

# WebAuthn / Passkeys
# WEBAUTHN_RP_ID is a bare domain (no port or scheme).
# WEBAUTHN_ORIGIN must match the exact URL the browser loads.
WEBAUTHN_RP_ID=localhost
WEBAUTHN_ORIGIN=http://localhost:8080

# Link signup allowlists (Optional) — chat identities that may create a new account
# from a link token. Comma-separated; unset or empty means nobody may, so an
# unknown Discord/Telegram/Email sender can only link to an existing account.
# Matrix needs none: it auto-provisions. WEB_URL (defaults to WEBAUTHN_ORIGIN)
# is the host the link points at.
# DISCORD_ALLOWED_USERS=123456789012345678
# TELEGRAM_ALLOWED_USERS=123456789
# EMAIL_ALLOWED_SENDERS=me@example.com

# Discord (Optional) — the adapter starts only when DISCORD_TOKEN is set.
# DISCORD_TOKEN=
# DISCORD_GUILD_ID=123456789012345678   # register commands to one server (instant updates)

# Telegram (Optional) — the adapter starts only when TELEGRAM_BOT_TOKEN is set.
# TELEGRAM_BOT_TOKEN=
# TELEGRAM_MODE=polling                  # polling (default) or webhook
# TELEGRAM_WEBHOOK_SECRET=               # required in webhook mode (alphanumeric, -, _)

# Email (Optional) — the adapter starts only when EMAIL_IMAP_URL is set.
# EMAIL_IMAP_URL=imaps://mail.example.com:993
# EMAIL_SMTP_URL=smtps://mail.example.com:465
# EMAIL_USERNAME=bot@example.com
# EMAIL_PASSWORD=app_password_here
# EMAIL_FROM=bot@example.com
# EMAIL_AUTHSERV_ID=mail.example.com
# EMAIL_POLL_SECONDS=60                  # Polling interval in seconds (default 60, min 5)
# EMAIL_PROCESSED_FOLDER=Archive        # Optional folder to move processed emails to

# AI Assistant & Live Web Search (Optional)
AI_ENDPOINT=https://api.x.ai/v1
AI_API_KEY=xai-...
AI_MODEL=grok-4.6
# AI_PROXY=http://127.0.0.1:7897        # Optional proxy for restricted networks
TAVILY_API_KEY=tvly-...                # Live Web Search (fallback to DuckDuckGo if unset)

# Text-to-Speech / !say voice replies (Optional — unset means !say is off;
# under `docker compose up` this defaults to the bundled inference server's Kokoro)
# TTS_BASE_URL=http://inference:8090/v1  # Local Kokoro (bundled), or a hosted OpenAI-compatible/ElevenLabs endpoint
# TTS_API_KEY=                           # Bearer token, or xi-api-key under TTS_DIALECT=elevenlabs
# TTS_MODEL=kokoro                       # Ignored by the bundled server; required for a hosted provider
# TTS_VOICE=0                            # House default voice; each user can override it in Settings
# TTS_DIALECT=openai                     # openai (default) or elevenlabs

# Knowledge-base embeddings (Optional — unset means keyword search only)
# EMBEDDING_BASE_URL=http://inference:8090  # OpenAI-compatible /v1/embeddings (the bundled inference server)
# EMBEDDING_MODEL=multilingual-e5-small     # Chunks are tagged with it; changing it re-embeds everything in the background
# EMBEDDING_MODEL_DIR=/app/data/embedding_models/multilingual-e5-small  # Read by the inference container: where it downloads the model

# Object storage for receipts, audio and exports (Optional; defaults to local disk)
# STORAGE_DRIVER=local                      # local (default) or s3
# STORAGE_LOCAL_DIR=data/storage
# STORAGE_SIGNING_SECRET=                   # signs presigned URLs; production refuses the default
# S3_BUCKET=  S3_REGION=  S3_ENDPOINT=  S3_ACCESS_KEY=  S3_SECRET_KEY=  S3_FORCE_PATH_STYLE=   # when STORAGE_DRIVER=s3

# Dynamic Configuration & Encrypted Secrets Vault (Optional in dev; required for storing secrets in the Admin Console)
# SECRETS_ENCRYPTION_KEY=                   # 32-byte master key (`openssl rand -base64 32` or `openssl rand -hex 32`)
# SECRETS_ENCRYPTION_KEYS=                  # Optional versioned keys for rotation (`1:<old_key>,2:<new_key>`)
```

> [!WARNING]
> **`WEBAUTHN_RP_ID` is baked into passkey credentials upon registration.**
> Changing it invalidates existing passkeys. If deploying to production (e.g. `bot.example.com`), set `WEBAUTHN_RP_ID=bot.example.com` and `WEBAUTHN_ORIGIN=https://bot.example.com` before creating passkeys.

---

### Admin Bootstrap & Runtime Configuration

Public registration and passkey enrollment never grant `admin` privileges automatically. To grant or revoke administrator access after an account has registered and enrolled at least one passkey:

```bash
# Grant admin role to a passkey-enrolled account
matrix-bot admin grant acct_<uuid>

# Revoke admin role and terminate all active sessions for that account
matrix-bot admin revoke acct_<uuid>
```

Once signed in, administrators can manage the **20 hot-reloadable runtime settings** (LLM defaults, Tavily/Brave search toggles, OCR/STT/TTS/Embedding endpoints and models, and link signup toggle) and **AES-256-GCM encrypted provider credentials** directly from the Web Admin Console without restarting the server. Restart-required settings (`EMBEDDING_MODEL`, `DISCORD_*`, `TELEGRAM_*`, `EMAIL_*`) remain in `.env`.

---

### Enabling Discord (optional)

1. Create an application in the Discord developer portal, add a bot, and copy its token into `DISCORD_TOKEN`. No privileged gateway intent is needed (the bot uses `GUILDS` and `DIRECT_MESSAGES`).
2. Invite the bot with the scopes `bot` and `applications.commands` and the permissions *Send Messages*, *Embed Links*, *Attach Files* and *Read Message History*.
3. Optionally set `DISCORD_GUILD_ID` to one server's id while testing: commands registered there update instantly, global ones can take up to an hour to appear.
4. People link their Discord account by sending `/link` to the bot in a direct message and opening the link it returns. List someone in `DISCORD_ALLOWED_USERS` to let them create a new account from that link; anyone else signs in to an existing one.

The bot answers `/todo`, `/expense`, `/portfolio`, `/ask`, `/link` and `/help` (each with one `args` text, e.g. `/todo args:add buy milk`; `/ask` takes a `prompt`), and talks to the assistant when you write to it in a direct message. In a server, every reply except `/help` is visible only to you.

---

### Enabling Telegram (optional)

1. Open Telegram, start a conversation with [@BotFather](https://t.me/BotFather), run `/newbot`, and follow the prompts to create your bot. Copy the bot token into `TELEGRAM_BOT_TOKEN`.
2. Keep group privacy mode enabled (the default in @BotFather): in groups and supergroups, the bot will only receive slash commands and direct mentions, protecting other group members' privacy.
3. Choose the operating mode in `TELEGRAM_MODE`:
   - `polling`: default, ideal for local testing and development behind NAT without a public domain. The bot long-polls Telegram for updates.
   - `webhook`: recommended for production. Requires a public HTTPS `WEB_URL` (or `WEBAUTHN_ORIGIN`) and `TELEGRAM_WEBHOOK_SECRET`. The bot calls `setWebhook` automatically at boot and receives updates at `POST /api/webhooks/telegram`.
4. People link their Telegram account by sending `/link` to the bot in a direct chat and opening the link it returns. List someone's numeric Telegram user ID in `TELEGRAM_ALLOWED_USERS` (comma-separated) to let them create a new account from that link; anyone else signs in to an existing one.

The bot advertises `/start`, `/help`, `/todo`, `/expense`, `/portfolio` and `/link` in Telegram's menu via `setMyCommands`; any other typed `/command` still reaches its module, and free text goes to the assistant. Direct messages route free-text to the AI assistant with typing indicators and status updates. In groups, commands are supported with `@botname` mentions, while private responses (e.g. portfolio/finance) are refused with a prompt to message the bot directly. The `/todo` task list includes interactive inline "Done" buttons to quickly complete tasks.

---

### Enabling Email (optional)

1. Configure a dedicated email account for the bot on your mail server.
2. Set `EMAIL_IMAP_URL` (must use `imaps://` for TLS) and `EMAIL_SMTP_URL` (`smtps://` or `smtp://`), along with credentials `EMAIL_USERNAME`, `EMAIL_PASSWORD`, and `EMAIL_FROM`.
3. Set `EMAIL_AUTHSERV_ID` to the hostname of your receiving MTA as reported in incoming `Authentication-Results` headers (e.g. `mail.example.com`). **Security invariant**: Only emails with a verified `dmarc=pass` matching this authserv-id are accepted. Unverified or failing emails are silently dropped with no bounce or response to prevent backscatter spam.
4. Optionally configure `EMAIL_POLL_SECONDS` (default: 60, minimum: 5) and `EMAIL_PROCESSED_FOLDER` (to move processed messages out of the inbox).
5. People link their email address by emailing `/link` or `!link` to the bot and opening the link returned in the response. List an email address in `EMAIL_ALLOWED_SENDERS` (comma-separated) to allow them to create a new account from that link; anyone else signs in to an existing one.

Email bodies are parsed with quote and signature stripping. Commands can be run with standard `!` or `/` syntax, or free text will route to the AI assistant. Responses are sent as clean multipart (plain text and HTML) replies preserving email threading headers (`In-Reply-To`, `References`).

---

### Option 1: Standalone Single Binary (Recommended)

The standalone binary is fully self-contained: **no Node.js, external web server, or container runtime required**. The React Web SPA is embedded directly into the executable.

#### 1. Download Prebuilt Binary
Download the prebuilt archive for your platform from the [GitHub Releases](https://github.com/evan1ee/matrix-bot/releases) page:
- **Linux (x86_64)**: `matrix-bot-linux-x86_64.tar.gz`
- **Linux (ARM64 / Raspberry Pi)**: `matrix-bot-linux-arm64.tar.gz`
- **macOS (Apple Silicon M1-M4)**: `matrix-bot-macos-arm64.tar.gz`
- **Windows (x64)**: `matrix-bot-windows-x86_64.zip`

#### 2. Run
```bash
# Linux / macOS
tar -xzf matrix-bot-linux-x86_64.tar.gz
chmod +x matrix-bot
./matrix-bot

# Windows (Command Prompt / PowerShell)
Expand-Archive matrix-bot-windows-x86_64.zip -DestinationPath .
.\matrix-bot.exe
```

Open **`http://localhost:8080`** in your browser to access the Web UI.

#### 3. (Optional) Run as a Linux Systemd Service
To keep the bot running 24/7 in the background:

```ini
# /etc/systemd/system/matrix-bot.service
[Unit]
Description=Matrix Bot Service
After=network.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/opt/matrix-bot
ExecStart=/opt/matrix-bot/matrix-bot
Restart=always
RestartSec=5
EnvironmentFile=/opt/matrix-bot/.env

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now matrix-bot
sudo systemctl status matrix-bot
```

---

### Option 2: Docker Compose (Recommended for Containers)

Docker Compose manages persistent data volumes and restarts automatically.
`docker compose up` starts **three** containers: `matrix-bot`, `postgres`
(the only application database), and `inference` (the local ONNX model
server for OCR, speech, text-to-speech and embeddings). Neither `postgres` nor `inference` is published
to the host — the bot reaches both over the compose network only.

```bash
# 1. Clone repo & setup .env
cp .env.example .env
# Set WEBAUTHN_ORIGIN=http://localhost:8080 (matching API_PORT)

# 2. Build and start in background
docker compose up -d

# 3. View streaming logs
docker compose logs -f

# 4. Stop service
docker compose down
```

Data is safely persisted in the named Docker volumes `matrix-bot-data` (Matrix crypto store) and `matrix-bot-postgres-data` (the PostgreSQL database) across container restarts. The inference server's downloaded OCR and speech-to-text models persist in `matrix-bot-inference-data`. The schema is created on first boot; there is no separate migration step.

Voice transcription defaults to the bundled `inference` service. To use a
hosted provider instead, set `STT_BASE_URL`, `STT_API_KEY` and `STT_MODEL` in
`.env` (for example OpenAI or Groq) — no compose changes needed.

---

### Option 3: Docker Run

Run directly as a single container without compose. There is no database or
inference server here, so `.env`'s `DATABASE_URL` must point at a PostgreSQL
server you run or manage yourself, and `OCR_BASE_URL` stays unset: the bot
still boots and every other feature works, and an image message just gets a
reply saying OCR is unavailable. Voice transcription is likewise unavailable
unless `STT_BASE_URL` points somewhere; pointing it at a hosted provider
instead (with `STT_API_KEY` and `STT_MODEL` set) gives voice transcription
with no inference server at all.

```bash
# 1. Create a persistent named volume for the Matrix crypto store
docker volume create matrix-bot-data

# 2. Run the container
# --stop-timeout 20: Docker sends SIGTERM, then SIGKILL after this many
# seconds. Must exceed app::SHUTDOWN_GRACE (15 s) or the process is killed
# mid-drain before its own graceful-shutdown bound gets to finish.
docker run -d \
  --name matrix-bot \
  --restart unless-stopped \
  --stop-timeout 20 \
  -p 8080:8080 \
  --env-file .env \
  -v matrix-bot-data:/app/data \
  matrix-bot:latest

# 3. Check logs
docker logs -f matrix-bot
```

---

### Option 4: Development from Source

For local development with hot-reloading frontend and active Rust compiler feedback:

```bash
# Terminal 1: Bot + REST API & SSE Server
cargo run

# Terminal 2: React SPA dev server with Vite hot reload
npm --prefix frontend run dev
```

In development, open **`http://localhost:5173`** (Vite proxies `/api` requests to `:8080`).

#### Building the Standalone Binary Locally
You can also compile your own standalone single binary anytime:
```bash
# 1. Build the frontend bundle into backend/web/dist
npm --prefix frontend ci
npm --prefix frontend run build

# 2. Compile the release binary (embeds the web bundle into the executable)
cargo build --release --bin matrix-bot

# Output binary: target/release/matrix-bot (or matrix-bot.exe)
```

---

## 🧑‍💻 Development

Everything below runs against a throwaway local PostgreSQL. **Coding agents never run the bot binary** and never need Matrix credentials, an AI key or a production database; see `AGENTS.md`.

**Toolchain**

| Tool | Version | Why |
|---|---|---|
| Rust | stable, with `rustfmt` and `clippy` | backend and inference crates |
| Docker | any recent | local PostgreSQL 18 for tests |
| `sqlx-cli` | 0.9 (postgres + rustls) | query cache and migrations: `cargo install sqlx-cli --version ^0.9 --no-default-features --features rustls,postgres --locked` |
| Node.js | 20+ | frontend build, OpenAPI client generation, Playwright |

Linux and CI containers also need `pkg-config libssl-dev` (and `cmake` for the inference crate). On Windows a cold `openssl-sys` build needs a full Perl on `PATH` (Strawberry, or `scoop install perl`); loading the real Kokoro model in an inference test needs sherpa's `onnxruntime.dll` in `target/debug/deps/` (see `inference/src/tts/kokoro.rs`). Sharing `CARGO_TARGET_DIR` across worktrees speeds up rebuilds.

**Configuration.** The bot reads `.env` from the current directory only. Start from `.env.example`; `ENVIRONMENT=development` relaxes production checks. Tests and builds need no `.env` (`SQLX_OFFLINE=true` reads the committed `.sqlx/` cache).

**Tests** — from the repository root. Database setup, environment variables and troubleshooting are in [`docs/database.md`](docs/database.md#test-databases):

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets                  # warning-free
cargo test  -p matrix-bot --all-targets                  # includes the architecture test
cargo test  -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo clippy -p matrix-bot-inference --all-targets -- -D warnings   # when inference/ or contracts/ change
cargo test  -p matrix-bot-inference --all-targets        # when inference/ or contracts/ change
npm --prefix frontend ci && npm --prefix frontend run build
npm --prefix frontend run test:e2e                       # Playwright; fake API, no backend needed
```

Run `npx --prefix frontend playwright install chromium` once per machine. After changing a `#[utoipa::path]` or `ToSchema`, regenerate both artifacts and commit them:

```bash
cargo run --bin generate-openapi        # -> backend/openapi.json
npm --prefix frontend run generate-api  # -> frontend/src/types/api.ts
```

**Forge and CI.** Development happens on a self-hosted Gitea (`git.mbm.one`, remote `gitea`, over SSH); GitHub (`origin`) is a mirror updated by hand. Branch from `master` with Conventional prefixes (`feat/…`, `fix/…`, `refactor/…`, `docs/…`), push with `git push -u gitea <branch>` and open the pull request from the link Gitea prints; `gh` only sees the mirror. CI runs on Gitea Actions (`.github/workflows/`) for pushes to `master` and pull requests into it, so a bare feature-branch push does not run it. Jobs run inside a container, so the PostgreSQL service answers to its label (`postgres`), not `127.0.0.1`; the job image must carry Node.js, and `ci.yml` installs `pkg-config`, `libssl-dev` and `cmake` itself. Do not "simplify" those away. Secrets live under Settings → Actions → Secrets on Gitea. Releases are annotated `v*` tags pushed to `gitea`.

---

## 🏗️ Layout

Cargo workspace with two members, plus a separate npm frontend.

```
backend/                      # crate `matrix-bot`: bot + embedded API, one binary
├── migrations/               # forward-only SQL, applied at boot by sqlx::migrate!
├── openapi.json              # generated; source of the frontend's types
├── tests/architecture.rs     # enforces layer/dependency rules
└── src/
    ├── main.rs, app.rs       # composition roots; app::run owns startup and graceful shutdown
    ├── config/               # environment config and boot-time validation
    ├── domain/<feature>/     # pure rules and invariants (no I/O)
    ├── application/<feature>/# services: use cases, transactions, events (AppServices)
    ├── interface/            # everything that talks to the outside world
    │   ├── http/             #   axum REST + SSE, DTOs, auth extractors
    │   ├── chat/             #   BotModule command handlers per feature (channel-neutral)
    │   ├── channels/         #   ChannelAdapter, registry, shared inbound pipeline, renderers
    │   │   ├── matrix/       #     client, sync loop, E2EE, verification
    │   │   ├── discord/      #     serenity gateway and slash commands
    │   │   ├── telegram/     #     teloxide polling or webhook
    │   │   └── email/        #     IMAP polling, SMTP replies
    │   ├── events/           #   outbox delivery sinks
    │   ├── jobs/             #   job handlers (reminder, briefing, portfolio, agent, knowledge, media)
    │   └── mcp/              #   MCP protocol adapters
    ├── jobs/                 # job runtime: workers, reaper, leader-elected scheduler
    ├── events/               # events runtime: outbox listener, SSE hub, delivery dispatcher
    ├── llm/                  # prompts, tool loop, tool registry (llm/tools/), agent engine (llm/agent/)
    ├── infrastructure/
    │   ├── postgres/         #   repositories, read models, account merge
    │   ├── inference/        #   llm, ocr, stt, tts, embedding clients and provider testers
    │   ├── crypto/           #   AES-256-GCM secret encryption
    │   ├── storage/          #   local and S3 object storage
    │   └── web/              #   outbound feeds, SSRF guard
    ├── db/                   # pool, migration runner, test pools
    └── utils/
inference/                    # crate `matrix-bot-inference`: Axum server hosting local ONNX
│                              # models (OCR, STT, TTS and embeddings), a separate binary and image
contracts/                    # JSON fixtures each crate asserts for the wire formats it speaks
frontend/                     # React 19 + Vite + Tailwind v4, built into backend/web/dist
data/                         # runtime Matrix crypto store (SQLite, gitignored)
docs/                         # architecture, database, api, llm; superpowers/ holds plans, specs, progress
```

---

## 📚 Documentation

| Document | Contents |
|---|---|
| [`AGENTS.md`](AGENTS.md) | Rules for coding agents: architecture, workflow, constraints, verification |
| [`docs/README.md`](docs/README.md) | Index of the documentation and the conventions it follows |
| [`docs/architecture.md`](docs/architecture.md) | Layers, runtimes, channels, accounts, settings, storage, knowledge base, configuration vault |
| [`docs/database.md`](docs/database.md) | Schema and SQLx conventions, migrations, test databases |
| [`inference/README.md`](inference/README.md) | The inference server: routes, models, provisioning |
| [`docs/api.md`](docs/api.md) | REST/SSE conventions, authentication, webhooks, OpenAPI workflow |
| [`docs/llm.md`](docs/llm.md) | LLM transport, tool loop, agent limits, MCP |
| [`docs/superpowers/`](docs/superpowers/) | Plans, designs, progress and hand-off state, runbooks |
| [`Design.md`](Design.md) | Frontend design system |
| [`docs/brainstorming.md`](docs/brainstorming.md) | Feature ideation and future roadmap backlog |

Near-term work: passkey autofill (conditional UI) on page load. Everything else is tracked in `docs/superpowers/HANDOFF.md`.
