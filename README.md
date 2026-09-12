# Matrix Bot (Rust) 🦀

A modular Matrix bot, embedded REST API, and real-time SSE engine in one binary, with a React SPA frontend and native iOS client support (including a native SwiftUI tab in Element X iOS).
Built on [`matrix-sdk`](https://github.com/matrix-org/matrix-rust-sdk),
[`axum`](https://github.com/tokio-rs/axum), [`sqlx`](https://github.com/launchbadge/sqlx),
and [`webauthn-rs`](https://github.com/kanidm/webauthn-rs).

Every feature is reachable through multiple surfaces — as Matrix chat commands, via the Web UI, or from a native iOS app — and all land on the exact same data.

---

## 🌟 Features

- 🤖 **Core** — greetings (`!hello`), latency ping (`!ping`), dynamic module-driven help (`!help`).
- 📋 **To-Dos** — priorities, due dates, lead-time reminder scheduling, per-user and room-wide views, task editing, and pagination.
- ⏰ **Reminders** — one-off and recurring (daily, weekly, monthly, yearly), persisted across process restarts with downtime catch-up and exponential retry backoff.
- 💰 **Finance** — expense and income ledger with customizable categories and monthly summaries. Amounts are stored strictly as integer minor units throughout (zero floating-point inaccuracies).
- 📈 **Portfolio** — asset holdings with weighted-average cost basis, realized & unrealized P&L, live ticker price fetching, custom price alerts, cash balance gating, manual position recording, and a tamper-evident SHA-256 audit hash chain.
- 🧾 **Recurring bills** — payment schedules, recurring frequencies, advance reminder notifications, auto-pay marking, and payment history.
- 🔖 **Bookmarks** — URL saving with automated OpenGraph/metadata scraping, reading time estimation, tag filtering, domain statistics, favorite stars, and archive management.
- 🎯 **Habits** — daily and weekly habit tracking, streak counters, completion heatmaps, and streak leaderboards.
- ⏳ **Life Grid & Daily Journal** — Memento Mori lifespan visualization (interactive weeks & decades grid), daily reflection logs (mood, weather, notes), cross-module daily activity timelines, and lifespan statistics.
- 📍 **Footprints** — travel & location check-in logs with country/city pins, ratings, visited places timeline, and journey statistics.
- 🔔 **Webhooks** — inbound alert gateway with severity cards, Telegram Bot API drop-in compatibility (`/bot:token/sendMessage`), and a durable retry delivery queue. See [`docs/webhooks.md`](docs/webhooks.md).
- 🧠 **AI Assistant** — contextual conversation threads, per-user LLM configuration (OpenAI, Anthropic Claude, Google Gemini), active model switching, and weekly executive retrospectives across spending and productivity.
- ⚙️ **Settings** — per-user timezone (IANA), default currency code, dev room, and conversation history turns, respected by all modules for consistent date/time rendering.
- 🔐 **Accounts & Security** — passkey (WebAuthn) sign-in, zero-friction Matrix session exchange (`POST /api/v1/auth/matrix/login`), linked Matrix identities, step-up security assertions, delayed Matrix recovery, and an immutable security audit log.

---

## 🔐 Authentication & Multi-Client Architecture

The backend supports multiple client surfaces with strict ownership isolation:

- **Web UI (Browser)**: **Passkeys only** (WebAuthn / FIDO2). No passwords or JWTs. Authenticated via HttpOnly, SameSite cookies backed by server-side session records.
- **Native iOS & Element X**: **1-Step Matrix Session Exchange** (`POST /api/v1/auth/matrix/login`). The native client exchanges its existing homeserver access token for an opaque backend session token, transmitted via standard `Authorization: Bearer <session_token>` headers.
- **Matrix Chat**: Bot commands are authenticated via cryptographic Matrix room event signatures, resolved to account owners through linked identities.

**Local accounts own the data.** A Matrix account is something you *attach* to a
local account, not an identity in its own right. Every todo, reminder,
transaction, bill, habit, bookmark, and holding belongs to an account; the same person signing in
on the web, using the native iOS app, and messaging the bot in Matrix is one owner with one unified dataset.

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
- **Step-up** — linking, unlinking, managing passkeys, signing out other devices
  and generating recovery codes all need a passkey assertion from the last five
  minutes. Matrix commands cannot reach any of them, because Matrix has no way
  to prompt for a passkey.
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
See [**`ELEMENT_X_IOS_INTEGRATION.md`**](ELEMENT_X_IOS_INTEGRATION.md) for full Swift networking code (`BotClient.swift`), SwiftUI views (`AssistantTabView.swift`), Keychain management, and instructions on registering the tab inside Element X's `HomeScreen.swift` and `AppCoordinator.swift`.

---

## 📋 Command reference

Every feature module exposes intuitive chat commands formatted in HTML for rich Matrix clients (Element, Element X, FluffyChat, Cinny):

| Module | Commands | Description |
|---|---|---|
| **Core** | `!hello / !hi [name]`, `!ping`, `!help` | Greetings, ping/latency diagnostic, dynamic module command help |
| **To-Do** | `!todo add [[high\|low]] <task>`<br>`!todo [list\|room] [page]`<br>`!todo done <id>`<br>`!todo priority <id> <low\|medium\|high>`<br>`!todo due <id> <time> [remind <lead>]`<br>`!todo edit <id> <new text>`<br>`!todo clear [room]` | Task management with priorities, due dates, advance reminders, room-wide or personal scoping, and pagination |
| **Reminder** | `!remind [daily\|weekly] <time> <message>`<br>`!remind list`<br>`!remind cancel <id>` | One-off and recurring reminders, scheduled in user timezone with automatic restart recovery |
| **Finance** | `!expense add <amount> [CUR] <note> [-c <cat>]`<br>`!income add <amount> [CUR] <note> [-c <cat>]`<br>`!expense list [page]`<br>`!expense summary`<br>`!expense delete <id>`<br>`!category [add\|list\|remove] <name>` | Double-entry-style ledger, category breakdowns, monthly summaries, integer minor units (no float rounding errors) |
| **Portfolio** | `!portfolio [list]`<br>`!portfolio summary`<br>`!portfolio buy <sym> <qty> @ <price> [CUR]`<br>`!portfolio record <sym> <qty> @ <price> [CUR]`<br>`!portfolio sell <sym> <qty> @ <price>`<br>`!portfolio price <sym> <price>`<br>`!portfolio refresh`<br>`!portfolio note <sym> [text]`<br>`!portfolio remove <sym>`<br>`!portfolio cash [deposit\|withdraw] <amt> [CUR]` | Holdings tracking with cash balance gating, market value, realized P&L, manual price recording or automated live quotes (`!portfolio refresh`), and notes |
| **Recurring Bills** | `!bill add <name> <amt> <cur> <cat> [frequency]`<br>`!bill list`<br>`!bill pay <id>`<br>`!bill remind <id> <minutes\|off>`<br>`!bill delete <id>` | Fixed schedules, instalment tracking, advance due date reminders, and payment history |
| **Bookmarks** | `!save <url> [tag1,tag2]`<br>`!bookmarks [tag]`<br>`!bookmark fav <id>`<br>`!bookmark archive <id>`<br>`!bookmark rm <id>` | Save web links with OpenGraph title & description extraction, estimated reading time, tagging, favorites, and archiving |
| **Habits** | `!habit [list]`<br>`!habit add <name>`<br>`!habit done <name\|id>`<br>`!habit undo <name\|id>`<br>`!habit streak`<br>`!habit delete <id>` | Daily habit tracking, one-tap checkoffs, streak counters, and streaks leaderboard |
| **Life Grid** | `!life stats`<br>`!life today`<br>`!life set-birth <YYYY-MM-DD> [lifespan]`<br>`!life log <mood> [note]` | Memento Mori lifespan perspective (% elapsed, weeks left), daily status, and mood reflections |
| **Daily Log** | `!journal [date]` (or `!daily`)<br>`!journal log <mood> [\| weather] [\| note]`<br>`!journal activities [date]`<br>`!journal clear [date]` | Daily retrospective logging, mood tracking, weather notes, and cross-module daily activity summaries |
| **Footprints** | `!footprint list`<br>`!footprint add <name> [\| date] [\| rating] [\| note]`<br>`!footprint delete <id>` | Visited places log, travel timeline, ratings (1-5 stars), and journey statistics |
| **Webhooks** | `!webhook list`<br>`!webhook create <name> [room]`<br>`!webhook test <id>`<br>`!webhook revoke <id>` | Inbound webhook endpoints for external services, custom alert cards, and Telegram Bot API emulation |
| **AI Assistant** | `!ai <prompt>`<br>`!ai reset`<br>`!ai model [name]`<br>`!ai config`<br>`!ai retro` | Conversational LLM assistant, room context memory, model selection, and weekly executive retrospectives |
| **Settings** | `!settings`<br>`!settings timezone <IANA zone>`<br>`!settings currency <CODE>` | Configure per-user timezone and default currency, applied across all modules |
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

## 🛠️ Setup

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (edition 2021)
- Node 20+ for the frontend
- A Matrix account for the bot on any homeserver

### Configure

```bash
cp .env.example .env
```

```env
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USERNAME=your_bot_username
MATRIX_PASSWORD=your_bot_password
MATRIX_RECOVERY_KEY=optional_e2ee_recovery_key
DATABASE_URL=sqlite://data/bot_data.db?mode=rwc
API_HOST=0.0.0.0
API_PORT=8080
API_REGISTRATION_SECRET=optional_secret_gating_registration

# Passkeys. RP ID is a bare hostname and ignores ports; ORIGIN carries the port
# and must match where the browser loads the page.
WEBAUTHN_RP_ID=localhost
WEBAUTHN_ORIGIN=http://localhost:5173
```

⚠️ **`WEBAUTHN_RP_ID` is baked into every passkey at creation.** Changing it
invalidates every credential ever registered, so pick the production hostname
before anyone registers. The app refuses to boot in production while these are
still the localhost defaults.

### Run

```bash
cargo run                        # bot + API on :8080
npm --prefix frontend run dev    # SPA on :5173, proxying /api
```

Open <http://localhost:5173> — the port `WEBAUTHN_ORIGIN` names.

### Test

```bash
cargo check                            # must be warning-free
cargo test                             # 473 tests passing
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

Cargo workspace with one member, plus a separate npm frontend and iOS integration guides.

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
    │                         # webhook, ai, settings, account
    └── utils/
frontend/                     # React 19 + Vite + Tailwind v4, built into backend/web/dist
ELEMENT_X_IOS_INTEGRATION.md  # Comprehensive guide, Swift client, and SwiftUI views for iOS
data/                         # runtime SQLite + Matrix crypto store (gitignored)
docs/                         # testing-auth-locally.md, webhooks.md
```

`AGENTS.md` holds the development conventions; `CLAUDE.md` explains *why* the
load-bearing parts are shaped the way they are, and is worth reading before
changing auth, ownership or migrations.

---

## 🚀 Roadmap

- **PostgreSQL** — the repository layer is already abstract; `DATABASE_URL` is
  the switch.
- **Conditional-UI sign-in** — passkey autofill on page load, rather than a
  button.
- **Offline Sync for Native iOS** — local SwiftData caching in Element X iOS for offline task and log creation.
