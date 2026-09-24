# Matrix Bot (Rust) 🦀

A modular Matrix bot, embedded REST API, and real-time SSE engine in one binary, with a React SPA frontend and native iOS client support (including a native SwiftUI tab in Element X iOS).
Built on [`matrix-sdk`](https://github.com/matrix-org/matrix-rust-sdk),
[`axum`](https://github.com/tokio-rs/axum), [`sqlx`](https://github.com/launchbadge/sqlx),
and [`webauthn-rs`](https://github.com/kanidm/webauthn-rs).

Every feature is reachable through multiple surfaces — as Matrix chat commands, via the Web UI, or from a native iOS app — and all land on the exact same data.

---

## 🌟 Features

- 🤖 **Core** — greetings (`!hello`), latency ping (`!ping`), dynamic module-driven help (`!help`).
- 📋 **To-Dos** — priorities, due dates, lead-time reminder scheduling, personal & room-wide views, task editing, pagination, and quoted message context capture (replying `!todo add` uses quoted text).
- ⏰ **Reminders** — one-off and recurring (daily, weekly, monthly, yearly), persisted across process restarts with downtime catch-up, exponential retry backoff, and quoted message context capture (replying `!remind <time>` uses quoted text).
- 💰 **Finance** — expense and income ledger with customizable categories and monthly summaries. Amounts are stored strictly as integer minor units throughout (zero floating-point inaccuracies).
- 📈 **Portfolio** — asset holdings with weighted-average cost basis, realized & unrealized P&L, live ticker price fetching, custom price alerts, cash balance gating, manual position recording, and a tamper-evident SHA-256 audit hash chain.
- 🧾 **Recurring bills** — payment schedules, recurring frequencies, advance reminder notifications, auto-pay marking, and payment history.
- 🔖 **Bookmarks** — URL saving with automated OpenGraph/metadata scraping, reading time estimation, tag filtering, domain statistics, favorite stars, and archive management.
- 🎯 **Habits** — daily and weekly habit tracking, streak counters, completion heatmaps, and streak leaderboards.
- ⏳ **Life Grid & Daily Journal** — Memento Mori lifespan visualization (interactive weeks & decades grid), daily reflection logs (mood, weather, notes), cross-module daily activity timelines, and lifespan statistics.
- 📍 **Footprints** — travel & location check-in logs with country/city pins, ratings, visited places timeline, and journey statistics.
- 🔔 **Webhooks** — inbound alert gateway with severity cards, Telegram Bot API drop-in compatibility (`/bot:token/sendMessage`), and a durable retry delivery queue. See [`docs/webhooks.md`](docs/webhooks.md).
- 🧠 **AI Assistant & Live Web Search** — multi-turn contextual AI threads, live web search & fact checking (powered by Tavily Search API with DuckDuckGo fallback), parallel tool execution, context-aware quoted replies (replying `@bot fact check` or `true?` verifies claims with sourced web snippets), per-user LLM configuration (xAI Grok, OpenAI, Anthropic Claude, Google Gemini), active model switching, and weekly executive retrospectives.
- 🔊 **Voice replies** — `!say <text>` speaks into the room as a native Matrix voice bubble (Ogg Opus with waveform), synthesized entirely offline by sherpa-onnx Kokoro (English + Mandarin, 103 voices, per-language lexicons and Chinese text normalization). Every user picks their own voice; no cloud TTS API and no second ML runtime.
- ⚙️ **Settings** — per-user timezone (IANA), default currency code, dev room, conversation history turns, and `!say` voice selection, respected by all modules for consistent date/time rendering.
- 🗄️ **SQLite or PostgreSQL** — one `DATABASE_URL` picks the database; no application code is dialect-specific. SQLite is the default and needs no server, which is what keeps the standalone single binary a single binary. Point it at PostgreSQL for a shared or managed deployment and the schema is created there on boot. The **entire test suite passes on both**, and CI runs both on every pull request. See [`docs/POSTGRES_MIGRATION.md`](docs/POSTGRES_MIGRATION.md).
- 🔐 **Accounts & Security** — passkey (WebAuthn) sign-in, zero-friction Matrix session exchange (`POST /api/v1/auth/matrix/login`), linked Matrix identities, step-up security assertions, delayed Matrix recovery, and an immutable security audit log.

---

## 🏛️ System Architecture

The system unifies two distinct interaction models — fast, deterministic chat commands and intelligent LLM-driven natural language — on top of a shared domain layer, real-time SSE streaming, and persistent SQLite **or PostgreSQL** storage:

```
                                      Matrix
                                        │
                      ┌─────────────────┴─────────────────┐
                      │                                   │
                  Command                         Natural Language
             (!todo, !finance)                  (@bot ..., direct DM)
                      │                                   │
                      │                                  LLM
                      │                             (xAI / OpenAI)
                      │                                   │
                      │                              Tool Calling ────► Live Web Search
                      │                            (add_todo, etc.)     (Tavily / DDG)
                      │                                   │
                      └─────────────────┬─────────────────┘
                                        │
                                        ▼
                                   Domain Layer
                     (Todo, Finance, Portfolio, Habits, etc.)
                                        │
                      ┌─────────────────┴─────────────────┐
                      ▼                                   ▼
             SQLite Persistence                    SSE Broadcaster
             (SQLx WAL Engine)                   (Real-Time Web/iOS)
```

### Core Architecture Highlights

1. **Dual Ingress (Matrix & REST/SSE)**:
   - **Matrix Chat**: Inbound events arrive over Matrix rooms and are classified as either explicit commands or natural language queries.
   - **REST API & SSE**: An embedded Axum web server provides REST endpoints (`/api/v1/*`) and real-time SSE event streaming (`/api/v1/events`) for the React Web UI and native iOS client (Element X).

2. **Deterministic Commands vs. Natural Language**:
   - **Commands (`!cmd`)**: Parsed immediately by `CommandParser` in [`backend/src/bot/dispatcher.rs`](backend/src/bot/dispatcher.rs). They bypass LLM processing entirely, providing deterministic execution with zero token cost and sub-millisecond response.
   - **Natural Language**: Routed to [`backend/src/modules/ai/`](backend/src/modules/ai/). The LLM selects tools defined in the function schema, which are dispatched through domain-grouped handlers in [`backend/src/modules/ai/tools/`](backend/src/modules/ai/tools/) (`todo.rs`, `finance.rs`, `reminder.rs`, `portfolio/`, `bills.rs`, `habits.rs`, `life.rs`, `bookmarks.rs`, `webhooks.rs`, `settings.rs`). Tool schemas are colocated directly within their domain tool files.

3. **Unified Domain Layer**:
   - Both command handlers and AI tool executors delegate to the **exact same domain repositories** (`TodoRepository`, `FinanceRepository`, `ReminderRepository`, `PortfolioRepository`, etc.). Room resolution and caching live in [`backend/src/db/rooms.rs`](backend/src/db/rooms.rs), preventing domain and tool layers from coupling to API modules. Business logic, validations, and ownership isolation are never duplicated.

4. **Persistence & Real-Time Sync**:
   - All state modifications persist through runtime SQLx query binding to **either SQLite or PostgreSQL**, chosen at boot from `DATABASE_URL`. SQLite is the default and uses write-ahead logging (WAL); no code is dialect-specific, because every query goes through the macros in [`backend/src/db/executor.rs`](backend/src/db/executor.rs). Schemas are modularized into domain DDL files in [`backend/src/db/schema_*.rs`](backend/src/db/schema.rs).
   - Any state change emits a strongly typed `SystemEvent` through the `EventBroadcaster`, delivered instantly to active web and mobile clients via SSE.

---

## 🔐 Authentication & Multi-Client Architecture

The backend supports multiple client surfaces with strict ownership isolation:

- **Web UI (Browser)**: **Passkeys only** (WebAuthn / FIDO2). No passwords or JWTs. Authenticated via HttpOnly, SameSite cookies backed by server-side session records.
- **Native iOS & Element X**: **1-Step Matrix Session Exchange** (`POST /api/v1/auth/matrix/login`). The native client exchanges its existing homeserver access token for an opaque backend session token, transmitted via standard `Authorization: Bearer <session_token>` headers.
- **Matrix Chat**: Bot commands are authenticated via cryptographic Matrix room event signatures, resolved to account owners through linked identities.

**Matrix account is the primary identity.** Every todo, reminder,
transaction, bill, habit, bookmark, and holding belongs to an account. Whether messaging the bot in Matrix, browsing on the web, or using the native iOS app, you are one owner with one unified dataset.

- **Matrix-Driven Passkey Enrollment (`!passkey add` or `!web`)** — send `!passkey add` (or `!web`) to the bot in a direct message. The bot generates a single-use 15-minute enrollment link that opens the web UI and registers a WebAuthn passkey directly to your existing Matrix account without creating orphan accounts.
- **Multiple Passkeys** — attach multiple passkeys (e.g. MacBook Touch ID, Windows Hello, iPhone Face ID, YubiKey) to a single account. Additional passkeys can be added anytime from chat via `!passkey add` or via the **+ Add a passkey** button in the Web UI Security Panel.
- **Chat Management & Revocation (`!passkeys` & `!unlink`)**:
  - `!passkeys` (or `!passkey list`) — lists all registered passkeys with index numbers, labels, and creation dates.
  - `!unlink passkey <n>` — deletes the specified passkey directly from chat. Because your Matrix identity is the primary identity, removing the last passkey is safely permitted without locking you out of your account.
  - `!unlink sessions` — revokes all active web sessions across all devices directly from chat.
- **Web Sign-in** — one button. Credentials are discoverable, so the browser offers
  the passkey and the credential itself identifies the account. There is no
  username or password field.
- **Native iOS Sign-in** — zero extra login forms. The iOS app presents its active Matrix session token directly to the backend. The backend verifies it against the Matrix homeserver (`/_matrix/client/v3/account/whoami`) and yields a backend session token immediately.
- **Dual Token Extraction & CSRF Protection** — API requests accept either an `Authorization: Bearer <token>` header or an HttpOnly session cookie. Requests using Bearer authentication bypass browser Origin CSRF checks, ensuring native mobile requests are never blocked by web CORS/Origin constraints.
- **Sessions** — backed by server-side database rows with SHA-256 token hashing; revoking a session immediately takes effect on its next request.
- **Linking** — the web UI shows a one-time code and you send it to the bot in a
  DM (`!link A3F9-K2QX`). Proof of ownership is the message sender, so no Matrix
  password is ever typed into the web app. Sending the code in a shared room
  cancels it.
- **Merging** — linking a Matrix account that already has history reports what
  it holds and asks before moving anything.
- **Step-up** — sensitive actions in the web UI (managing passkeys, unlinking identities, signing out other devices, generating recovery codes) require a recent passkey assertion within the last 5 minutes.
- **Recovery** — ten single-use codes, plus `!recover` from a linked Matrix
  account. The latter is announced immediately, waits 24 hours, can be cancelled
  from any signed-in device, and then grants a session that may register a
  passkey and nothing else for 72 hours.

Every sensitive action is written to a per-account audit log and DM'd to each
linked Matrix identity.

See [`docs/testing-auth-locally.md`](docs/testing-auth-locally.md) to exercise
all of this on `localhost` with no hardware key.

---

## 📱 iOS & Element X Integration

The backend is built to power native iOS apps, specifically designed to plug directly into **[Element X iOS](https://github.com/element-hq/element-x-ios)** as an additional first-class native tab (e.g. **"Assistant"** or **"Workspace"**).

### Architectural Overview

```
┌─────────────────────────────────────────────────────────────┐
│                       Element X iOS                         │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────────┐  │
│  │  Chats Tab    │ │  People Tab   │ │ ⭐️ Assistant Tab  │  │
│  │ (Matrix SDK)  │ │ (Matrix SDK)  │ │ (Native SwiftUI)  │  │
│  └───────┬───────┘ └───────┬───────┘ └─────────┬─────────┘  │
└──────────┼─────────────────┼───────────────────┼────────────┘
           │ Matrix Sync     │ Matrix Sync       │ REST API / SSE
           │ (E2EE, APNs)    │ (E2EE, APNs)      │ Bearer Auth
           ▼                 ▼                   ▼
┌────────────────────────────────────┐   ┌────────────────────┐
│      Matrix Homeserver             │   │ matrix-bot Backend │
│   (Synapse / Conduit / Dendrite)   │◄──┤ (Axum + SQLite)    │
└────────────────────────────────────┘   └────────────────────┘
```

1. **Zero-Friction Authentication Handshake**:
   - Element X is already logged into Matrix with an `access_token`, `matrix_user_id`, and `homeserver_url`.
   - The native iOS tab calls `POST /api/v1/auth/matrix/login` passing these credentials.
   - The backend validates the token directly against the homeserver via `GET /_matrix/client/v3/account/whoami`.
   - If the Matrix user ID matches an existing account (or auto-provisions a new linked account), the backend returns a session token:
     ```json
     {
       "session_token": "mb_sess_...",
       "account_id": "acc_...",
       "matrix_user_id": "@alice:example.org"
     }
     ```
   - The token is securely saved in the iOS Keychain.

2. **Native iOS Networking**:
   - Every subsequent request includes the HTTP header:
     ```http
     Authorization: Bearer mb_sess_...
     ```
   - Standard REST endpoints (`/api/v1/todos`, `/api/v1/finance/...`, `/api/v1/portfolio/...`, etc.) return strongly typed JSON models.
   - Native real-time streaming connects to `GET /api/v1/events` via `URLSession.bytes(for:)` for live SSE updates.

3. **Zero-Infra Push Notifications via Matrix**:
   - **No custom APNs server or certificates required.**
   - All reminders, bill alerts, and portfolio notifications triggered by the bot's background workers are sent directly as Matrix messages to the user's 1:1 Bot DM room.
   - Element X iOS already runs background sync and push notifications for incoming room events, delivering native lock-screen push notifications automatically!

👉 **Complete Step-by-Step iOS Integration Guide**:
See [**`docs/ELEMENT_X_IOS_INTEGRATION.md`**](docs/ELEMENT_X_IOS_INTEGRATION.md) for full Swift networking code (`BotClient.swift`), SwiftUI views (`AssistantTabView.swift`), Keychain management, and instructions on registering the tab inside Element X's `HomeScreen.swift` and `AppCoordinator.swift`.

---

## 📋 Command reference

Every feature module exposes intuitive chat commands formatted in HTML for rich Matrix clients (Element, Element X, FluffyChat, Cinny):

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
| **Voice (TTS)** | `!say <text>` | Speaks the text into the room as a native Matrix voice message, synthesized offline by Kokoro; voice chosen per user |
| **Settings** | `!settings`<br>`!settings timezone <IANA zone>`<br>`!settings currency <CODE>`<br>`!settings voice <0-1023\|none>` | Configure per-user timezone, default currency, and the `!say` speaker voice, applied across all modules |
| **Account** | `!link <code>`<br>`!recover` | Link Matrix account to web/native identity, initiate delayed recovery |

`!export`, `!unlink`, `!passkey`, `!sessions` and `!recoverycodes` are
deliberately refused on Matrix and point at the web app or native app — see **step-up** security assertions.

---

## 🌐 REST API

All API endpoints are mounted under `/api/v1`. Authentication is accepted via either:
1. **HttpOnly Session Cookie**: Standard for the React Web SPA with strict CSRF Origin validation.
2. **Bearer Token (`Authorization: Bearer <session_token>`)**: Standard for native iOS clients (Element X iOS), exempt from web browser Origin checks.

| Category | Endpoint(s) | Description |
|---|---|---|
| **Health** | `GET /health` | Liveness probe and uptime check |
| **Auth** | `POST /auth/passkey/{register,login}/{start,finish}`<br>`POST /auth/matrix/login`<br>`POST /auth/logout`<br>`GET /auth/me`<br>`GET /auth/media`<br>`POST /auth/recovery/redeem` | WebAuthn passkey ceremonies, 1-step Matrix session token exchange for iOS, logout, profile info, authenticated media proxy, and single-use recovery code redemption |
| **Account** | `GET/PUT/DELETE /account/passkeys[/:id]`<br>`GET /account/matrix`<br>`POST /account/matrix/{link,unlink,confirm-merge}`<br>`GET /account/matrix/pending-merge`<br>`GET /account/sessions`<br>`POST /account/sessions/revoke-others`<br>`DELETE /account/sessions/:id`<br>`POST /account/recovery-codes`<br>`GET /account/recovery-status`<br>`POST /account/recovery/cancel`<br>`GET /account/security-events` | Manage passkeys, link/unlink Matrix MXIDs, review pending merges, inspect and revoke active sessions, generate 10 recovery codes, delayed recovery controls, and immutable security audit logs |
| **AI Config** | `GET/PUT /account/ai-config` | Per-user LLM provider API keys and model parameters |
| **Webhooks Mgmt** | `GET/POST/PATCH/DELETE /account/webhooks[/:id]`<br>`POST /account/webhooks/:id/{rotate,test}`<br>`GET /account/webhooks/:id/logs` | Create and manage inbound webhook tokens, test delivery, and inspect execution logs |
| **To-Dos** | `GET/POST /todos`<br>`PATCH/DELETE /todos/:id` | List with filters and pagination, create, update status/priority/due date, and delete |
| **Reminders** | `GET/POST /reminders`<br>`PATCH/DELETE /reminders/:id` | Manage one-off and recurring scheduled reminders |
| **Finance** | `GET/POST /finance/transactions`<br>`PUT/DELETE /finance/transactions/:id`<br>`GET /finance/summary`<br>`GET/POST/DELETE /finance/categories[/:id]` | Ledger transactions, monthly category breakdowns, and custom category management |
| **Portfolio** | `GET/POST /portfolio/holdings`<br>`POST /portfolio/holdings/record`<br>`DELETE /portfolio/holdings/:id`<br>`POST /portfolio/holdings/:id/sell`<br>`PUT /portfolio/holdings/:id/{price,symbol,note}`<br>`POST /portfolio/{refresh-prices,holdings/:id/refresh-price}`<br>`GET /portfolio/{quote,search,summary,history}`<br>`GET /portfolio/activity`<br>`GET /portfolio/activity/verify`<br>`GET /portfolio/cash`<br>`POST /portfolio/cash/{deposit,withdraw}`<br>`GET/POST/DELETE /portfolio/alerts[/:id]` | Positions, cash management, automated market price refreshes, live quotes, price alerts, historical valuations, and tamper-evident SHA-256 cryptographic audit chain verification |
| **Recurring Bills** | `GET/POST /recurring-bills`<br>`GET/PUT/DELETE /recurring-bills/:id`<br>`POST /recurring-bills/:id/pay` | Recurring subscription/bill tracker with advance reminders and payment recording |
| **Bookmarks** | `GET/POST /bookmarks`<br>`GET /bookmarks/stats`<br>`GET/PATCH/DELETE /bookmarks/:id`<br>`POST /bookmarks/:id/{toggle-favorite,toggle-archive}` | Read-it-later bookmarking, metadata scraper, domain statistics, favorite & archive filters |
| **Habits** | `GET/POST /habits`<br>`GET /habits/{stats,heatmap}`<br>`PUT/DELETE /habits/:id`<br>`POST /habits/:id/toggle` | Habit tracker, daily checkoff toggling, streak counters, and completion heatmap |
| **Life & Journal** | `GET/POST/PUT /life/profile`<br>`GET /life/{year/:year,horizon}`<br>`POST /life/days`<br>`GET/DELETE /life/days/:date`<br>`PUT /life/days/:date/time`<br>`GET /life/days/:date/activity` | Memento Mori lifespan timeline, day entries with mood and reflection, time allocation tracker, and cross-module daily activity aggregations |
| **Footprints** | `GET/POST /footprints`<br>`GET /footprints/{stats,geocode}`<br>`GET/PUT/DELETE /footprints/:id` | Travel check-ins, location geocoding proxy, visit timeline, and journey statistics |
| **AI Assistant** | `POST /ai/chat`<br>`POST /ai/verify`<br>`GET/DELETE /ai/conversations[/:id]`<br>`GET /ai/conversations/:id/messages`<br>`GET /ai/retrospective` | Multi-turn contextual AI chat, LLM connectivity testing, conversation management, and weekly executive retrospectives |
| **Inbound Gateway** | `POST /webhooks/:token`<br>`GET/POST /bot:token/sendMessage` | Inbound generic webhook receiver and drop-in Telegram Bot API message receiver |
| **Settings & Rooms** | `GET/PUT /settings`<br>`GET /rooms`<br>`POST /rooms/sync` | User preferences (timezone, default currency, dev room) and accessible Matrix room synchronization |
| **Real-Time Live** | `GET /events` | Server-Sent Events (SSE) stream scoped strictly to authenticated owner's mutations |

`GET /api-docs/openapi.json` serves the auto-generated OpenAPI 3.1 specification; the frontend's TypeScript types and iOS API client models are generated from it.

The SSE stream authenticates via cookie or `Bearer` token. Events carry `{ "type": "TodoCreated", "payload": { ... } }`, allowing real-time reactive UI updates without manual polling.

---

## 🛠️ Setup & Running

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

# Database — SQLite (default, no server) or PostgreSQL; the scheme decides
DATABASE_URL=sqlite://data/bot_data.db?mode=rwc
# DATABASE_URL=postgres://bot:bot@localhost:5432/matrix_bot

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

# AI Assistant & Live Web Search (Optional)
AI_ENDPOINT=https://api.x.ai/v1
AI_API_KEY=xai-...
AI_MODEL=grok-4.6
# AI_PROXY=http://127.0.0.1:7897        # Optional proxy for restricted networks
TAVILY_API_KEY=tvly-...                # Live Web Search (fallback to DuckDuckGo if unset)

# Text-to-Speech / !say voice replies (Optional — off by default; the Kokoro model is 311 MB)
TTS_ENABLED=false
TTS_MODEL_DIR=data/tts_models/kokoro-multi-lang-v1_1
TTS_SPEAKER_ID=0                       # House default voice; each user can override it in Settings
```

> [!WARNING]
> **`WEBAUTHN_RP_ID` is baked into passkey credentials upon registration.**
> Changing it invalidates existing passkeys. If deploying to production (e.g. `bot.example.com`), set `WEBAUTHN_RP_ID=bot.example.com` and `WEBAUTHN_ORIGIN=https://bot.example.com` before creating passkeys.

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
`docker compose up` now starts **two** containers: `matrix-bot` and
`inference` (the local ONNX model server behind `POST /ocr`). The inference
service is not published to the host — the bot reaches it over the compose
network only.

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

Data is safely persisted in the named Docker volume `matrix-bot-data` across container restarts. The inference server's downloaded OCR and speech-to-text models persist in `matrix-bot-inference-data`.

Voice transcription defaults to the bundled `inference` service. To use a
hosted provider instead, set `STT_BASE_URL`, `STT_API_KEY` and `STT_MODEL` in
`.env` (for example OpenAI or Groq) — no compose changes needed.

---

#### Running against PostgreSQL

SQLite is the default and needs nothing extra. To run against PostgreSQL
instead, the Compose file ships a `postgres` profile:

```bash
DATABASE_URL=postgres://bot:bot@postgres:5432/matrix_bot docker compose --profile postgres up -d
```

The profile keeps the database container out of the default `docker compose up`,
so the SQLite path stays a single service. The schema is created on first boot;
there is no separate migration step.

> [!NOTE]
> This is a *new* database, not a migration of an existing one. Nothing here
> copies rows from a SQLite file into PostgreSQL — if you have existing data,
> move it yourself before switching.

---

### Option 3: Docker Run

Run directly as a single container without compose. There is no inference
server here, so `OCR_BASE_URL` stays unset: the bot still boots and every
other feature works, and an image message just gets a reply saying OCR is
unavailable. Voice transcription is likewise unavailable unless `STT_BASE_URL`
points somewhere; pointing it at a hosted provider instead (with
`STT_API_KEY` and `STT_MODEL` set) gives voice transcription with no
inference server at all.

```bash
# 1. Create a persistent named volume for SQLite and Matrix crypto store
docker volume create matrix-bot-data

# 2. Run the container
docker run -d \
  --name matrix-bot \
  --restart unless-stopped \
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

### Test

```bash
cargo check                            # must be warning-free
cargo test                             # 494+ tests passing
npm --prefix frontend run build        # TypeScript & production bundle
npx --prefix frontend playwright test  # 169 E2E specs
```

After changing a `#[utoipa::path]` or `ToSchema`, regenerate both artifacts:

```bash
cargo run --bin generate-openapi        # -> backend/openapi.json
npm --prefix frontend run generate-api  # -> frontend/src/types/api.ts
```

---

## 🏗️ Layout

Cargo workspace with two members, plus a separate npm frontend and iOS integration guides.

```
backend/                      # crate `matrix-bot`: bot + embedded API, one binary
├── openapi.json              # generated; source of the frontend's & iOS types
└── src/
    ├── main.rs               # boots Config, DB, API server, and the Matrix sync loop
    ├── config.rs             # environment config and boot-time validation
    ├── accounts/             # the ownership layer
    │   ├── repository.rs     #   accounts + matrix_links, MXID -> account resolution
    │   ├── credentials.rs    #   stored passkeys
    │   ├── session.rs        #   server-side sessions, step-up clock
    │   ├── linking.rs        #   one-time link codes, claim and unlink
    │   ├── merge.rs          #   absorbing one account into another
    │   ├── recovery.rs       #   recovery codes and delayed Matrix recovery
    │   └── security_events.rs#   per-account audit log
    ├── api/                  # axum REST layer
    │   ├── auth/             #   passkey ceremonies, 1-step Matrix login, cookie/bearer extractor
    │   ├── account.rs        #   passkeys, links, sessions, recovery, audit
    │   └── …                 #   todos, reminders, finance, portfolio, bills, bookmarks, habits, life, ai, sse
    ├── bot/                  # Matrix client, dispatcher, verification
    │   ├── linking.rs        #   !link, handled before the sender is resolved
    │   └── notify.rs         #   security DMs to linked identities
    ├── db/                   # pool + the single migration path
    ├── modules/              # BotModule plugins: core, todo, reminder, finance, portfolio,
    │                         # recurring_bills, bookmarks, habits, life, journal, footprints,
    │                         # webhook, ai, settings, account, briefing, mcp, rooms,
    │                         # ocr (image text), voice (speech→text), tts (text→speech)
    ├── services/             # HTTP clients to sibling services
    │   ├── ocr/               #   client to the inference crate's POST /ocr
    │   └── stt/               #   OpenAI-compatible client to POST /v1/audio/transcriptions
    └── utils/
inference/                    # crate `matrix-bot-inference`: Axum server hosting local ONNX
│                              # models (OCR, STT now; TTS later), a separate binary and image
contracts/                    # JSON fixtures each crate asserts for the wire formats it speaks
frontend/                     # React 19 + Vite + Tailwind v4, built into backend/web/dist
data/                         # runtime SQLite + Matrix crypto store (gitignored)
docs/                         # documentation, roadmap, architecture reviews, guides
```

`AGENTS.md` holds the development conventions; `CLAUDE.md` explains *why* the
load-bearing parts are shaped the way they are, and is worth reading before
changing auth, ownership or migrations.

---

## 🚀 Roadmap

- ~~**PostgreSQL**~~ — shipped. `DATABASE_URL` is the switch, and the whole
  suite passes on both databases; see
  [`docs/POSTGRES_MIGRATION.md`](docs/POSTGRES_MIGRATION.md).
- **Conditional-UI sign-in** — passkey autofill on page load, rather than a
  button.
- **Offline Sync for Native iOS** — local SwiftData caching in Element X iOS for offline task and log creation.
