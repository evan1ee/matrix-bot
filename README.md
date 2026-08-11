# Matrix Bot (Rust) 🦀

A modular Matrix bot and REST API in one binary, with a React SPA front end.
Built on [`matrix-sdk`](https://github.com/matrix-org/matrix-rust-sdk),
[`axum`](https://github.com/tokio-rs/axum), [`sqlx`](https://github.com/launchbadge/sqlx)
and [`webauthn-rs`](https://github.com/kanidm/webauthn-rs).

Every feature is reachable two ways — as a chat command, or through the web UI —
and both land on the same data.

---

## 🌟 Features

- 🤖 **Core** — greetings (`!hello`), latency (`!ping`), dynamic help (`!help`).
- 📋 **To-Dos** — priorities, due dates, lead-time reminders, per-user and
  room-wide views, pagination.
- ⏰ **Reminders** — one-off and recurring, persisted through restarts, with
  delivery retries and downtime catch-up.
- 💰 **Finance** — expense/income ledger with categories and summaries. Amounts
  are integer minor units throughout, so no total ever touches a float.
- 📈 **Portfolio** — holdings with weighted-average cost, realised P&L, and a
  cash balance that gates every buy. Prices are entered manually; there is no
  price feed in this codebase.
- 🧾 **Recurring bills** — schedules, instalments, auto-pay, advance reminders.
- ⚙️ **Settings** — per-user timezone and default currency, read by every other
  module so a date renders the same wherever it appears.
- 🔐 **Accounts** — passkey sign-in, linked Matrix identities, recovery.

---

## 🔐 Authentication

Sign-in is **passkeys only**. There are no passwords, no bearer tokens and no
JWTs anywhere in the system.

**Local accounts own the data.** A Matrix account is something you *attach* to a
local account, not an identity in its own right. Every todo, reminder,
transaction, bill and holding belongs to an account; the same person signing in
on the web and messaging the bot from Matrix is one owner with one dataset.

- **Sign in** — one button. Credentials are discoverable, so the browser offers
  the passkey and the credential itself says which account it is. There is no
  username field.
- **Sessions** — an HttpOnly cookie backed by a server-side row, so signing out
  a device takes effect on its next request. Only the hash of the cookie is
  stored.
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

## 📋 Command reference

| Module | Commands |
|---|---|
| Core | `!hello / !hi [name]`, `!ping`, `!help` |
| To-Do | `!todo add [[high\|low]] <task>`, `!todo [list\|room] [page]`, `!todo done <id>`, `!todo priority <id> <low\|medium\|high>`, `!todo due <id> <time> [remind <lead>]`, `!todo edit <id> <new text>`, `!todo clear [room]` |
| Reminder | `!remind [daily\|weekly] <time> <message>`, `!remind list`, `!remind cancel <id>` |
| Finance | `!expense add <amount> [CUR] <note> [-c <category>]`, `!income add <amount> [CUR] <note> [-c <category>]`, `!expense list [page]`, `!expense summary`, `!expense delete <id>`, `!category [add\|list\|remove] <name>` |
| Portfolio | `!portfolio [list]`, `!portfolio summary`, `!portfolio buy <symbol> <qty> @ <price> [CUR] [note]`, `!portfolio sell <symbol> <qty> @ <price>`, `!portfolio price <symbol> <price>`, `!portfolio note <symbol> [text]`, `!portfolio remove <symbol>`, `!portfolio cash [deposit\|withdraw] <amount> [CUR]` |
| Bills | `!bill add <name> <amount> <currency> <category> [frequency]`, `!bill list`, `!bill pay <id>`, `!bill remind <id> <minutes\|off>`, `!bill delete <id>` |
| Settings | `!settings`, `!settings timezone <IANA zone>`, `!settings currency <CODE>` |
| Account | `!link <code>`, `!recover` |

`!export`, `!unlink`, `!passkey`, `!sessions` and `!recoverycodes` are
deliberately refused on Matrix and point at the web app — see **step-up** above.
(`!export` reserves the name; financial export is not built yet, and when it is
it belongs behind step-up on the web side.)

---

## 🌐 REST API

All endpoints live under `/api/v1`. Everything except health and the four
passkey ceremony endpoints requires the session cookie.

| Area | Endpoints |
|---|---|
| Health | `GET /health` |
| Sign-in | `POST /auth/passkey/{register,login}/{start,finish}`, `POST /auth/logout`, `GET /auth/me`, `GET /auth/media` |
| Account | `GET/PUT/DELETE /account/passkeys[/{id}]`, `GET /account/matrix`, `POST /account/matrix/{link,unlink,confirm-merge}`, `GET /account/matrix/pending-merge`, `GET /account/sessions`, `POST /account/sessions/revoke-others`, `POST /account/recovery-codes`, `GET /account/recovery-status`, `POST /account/recovery/cancel`, `GET /account/security-events` |
| Data | `/todos`, `/reminders`, `/finance/{transactions,categories,summary}`, `/portfolio/{holdings,cash,summary}`, `/recurring-bills`, `/settings`, `/rooms` |
| Live | `GET /events` — SSE, scoped to the caller's own rows |

`GET /api-docs/openapi.json` serves the generated document; the frontend's
TypeScript types are generated from it.

The SSE stream authenticates by cookie like everything else. Messages are
unnamed and carry `{"type": "TodoCreated", "payload": {…}}`, so the browser
reads them from `EventSource.onmessage` and discriminates on `type`.

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
cargo check                      # must be warning-free
cargo test                       # 377 tests
npm --prefix frontend run build
```

After changing a `#[utoipa::path]` or `ToSchema`, regenerate both artifacts:

```bash
cargo run --bin generate-openapi        # -> backend/openapi.json
npm --prefix frontend run generate-api  # -> frontend/src/types/api.ts
```

---

## 🏗️ Layout

Cargo workspace with one member, plus a separate npm frontend.

```
backend/                      # crate `matrix-bot`: bot + embedded API, one binary
├── openapi.json              # generated; source of the frontend's types
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
    │   ├── auth/             #   passkey ceremonies, cookie extractor, step-up
    │   ├── account.rs        #   passkeys, links, sessions, recovery, audit
    │   └── …                 #   todos, reminders, finance, portfolio, bills, settings, sse
    ├── bot/                  # Matrix client, dispatcher, verification
    │   ├── linking.rs        #   !link, handled before the sender is resolved
    │   └── notify.rs         #   security DMs to linked identities
    ├── db/                   # pool + the single migration path
    ├── modules/              # BotModule plugins: core, todo, reminder, finance,
    │                         # portfolio, recurring_bills, settings, account
    └── utils/
frontend/                     # React 19 + Vite + Tailwind v4, built into backend/web/dist
data/                         # runtime SQLite + Matrix crypto store (gitignored)
docs/                         # testing-auth-locally.md
```

`AGENTS.md` holds the development conventions; `CLAUDE.md` explains *why* the
load-bearing parts are shaped the way they are, and is worth reading before
changing auth, ownership or migrations.

---

## 🚀 Roadmap

- **PostgreSQL** — the repository layer is already abstract; `DATABASE_URL` is
  the switch.
- **Live prices** for the portfolio module. Today every price is entered by hand,
  and market value is `Option`, not zero, until one is.
- **Conditional-UI sign-in** — passkey autofill on page load, rather than a
  button.
