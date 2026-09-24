# Product Roadmap

Written against `4412157` (branch `visualizing-life`), by reading the tree rather
than the previous roadmap. Every claim below cites the code that causes it, and
every "does not exist" is a grep that returned zero hits at that commit.

This document replaces the previous `ROADMAP.md` and `BRAINSTORMING.md`, which
had both drifted badly — they still described bill-to-ledger sync, price feeds,
bookmarks, habits, footprints and the AI module as unbuilt. Keeping one document
instead of three is deliberate: the drift was caused by three overlapping files
aging independently.

`FEATURE_PROPOSALS.md` survives as the implementation-detail layer. Its §1
(room-shared lists) is still unbuilt and still the hard one; see §5 below.

---

## Thesis

The product's promise is that **every feature is reachable two ways — as a chat
command or through the web UI — and both land on the same data.**

The data half is true and well built. The delivery half is still not.
**The web app can store anything and deliver nothing.** Every notification this
product sends travels through Matrix, so a user who signs in with a passkey and
never links a Matrix account owns a reminder app that cannot remind them.

That was the previous roadmap's finding #1 and it is unchanged. What *has*
changed is the cost of leaving it open: eight modules have shipped since, each
adding something that wants to reach the user — habit nudges, price alerts, bill
warnings, weekly retrospectives — and none of them can.

---

## What shipped since the last roadmap

Recorded so the next reader does not re-propose them.

| Feature | Where |
|---|---|
| Bookmarks + reader view | `modules/bookmarks/`, `metadata.rs` extracts OpenGraph |
| Habits + heatmap | `modules/habits/`, `frontend/.../HabitHeatmap.tsx` |
| Footprints + Leaflet map | `modules/footprints/`, `FootprintsMap.tsx` |
| Life grid / memento mori | `modules/life/`, `LifeHorizonMinimap.tsx` |
| AI chat, 34 tools, retrospective | `modules/ai/`, `tools/definitions*.rs` |
| Webhooks in + out | `modules/webhook/`, `api/webhooks.rs` |
| Portfolio price feed | `modules/portfolio/price_feed.rs` |
| Portfolio alerts / history / audit chain | `alerts.rs`, `history.rs`, `audit.rs` |
| **Bill payment posts to the ledger** | `recurring_bills/repository.rs:205` |
| Matrix reactions → complete / snooze / delete | `bot/reaction.rs`, `backend/migrations/0001_baseline.sql` (`bot_message_targets`) |
| Voice replies (`!say`, offline Kokoro TTS) | `modules/tts/`, `opus.rs` writes the Ogg voice bubble |

The last one closed the previous finding #2; the price feed closed #9.

---

## 1. A reminder with no Matrix room is still silently dropped

**Impact:** critical · **Frequency:** daily · **Effort:** M

A reminder created in the web UI is stored with the literal string `web` where a
room ID belongs. `deliver_to_matrix` cannot parse that as a room and returns
`true` — delivered — without sending anything. The docstring states the intent
outright: *"for those the SSE broadcast is the delivery, so there is nothing to
fail here."* Close the tab and the reminder fires into nothing.

- `backend/src/modules/reminder/scheduler.rs:52-60` — `web` sentinel returns `true`
- `backend/src/api/reminders.rs` — `room_id.unwrap_or_else(|| "web")`
- `frontend/public/` — favicon only: no manifest, no service worker
- `grep -ri "vapid\|serviceworker\|web-push"` — **zero hits**

### The fix is an abstraction, not a feature

Four separate places reach directly for the Matrix `Client` to tell a user
something: `bot/notify.rs`, `reminder/scheduler.rs`, `portfolio/alerts.rs`,
`recurring_bills/due_reminder.rs`. Adding push as a fifth special case in each
is how this stays broken.

Introduce a `Notification` trait with two implementations — Matrix DM and Web
Push — resolved per owner from their linked identities and push subscriptions.
Store subscriptions in `web_push_subscriptions` keyed by `owner_id`, the same
key every other table uses. The trait is the deliverable; push is the second
impl, and any future channel is the third.

Ship the PWA manifest in the same change, and with it **`share_target`**: share
a URL from mobile Safari straight into `bookmarks`, which already fetches
OpenGraph metadata and extracts reader content. Small once the manifest exists.

---

## 2. Work that is already written and merely not scheduled

**Impact:** high · **Frequency:** weekly · **Effort:** S

`ai/retrospective.rs:60` generates a full weekly retrospective — spending versus
prior week, top category, completed tasks, habit consistency, upcoming bills. It
is reachable only by pulling: `handle_retrospective_command` and
`GET /ai/retrospective`. Nothing pushes it. 555 lines of finished analysis that
a user has to remember to ask for.

The reminder scheduler already runs a 5s tick with the `Client`, `Database` and
broadcaster in hand (`spawn_background_workers`). Hanging a Sunday 20:00 send off
it is a small change against a large piece of existing work.

The same loop gives you the **daily digest** for free — today's due todos,
uncompleted habits, bills due within three days, the life-grid progress line.
Every one of those queries exists in a repository already; the digest is
composition, not new capability.

---

## 3. Matrix is still used as a text box, not a platform

**Impact:** high · **Frequency:** daily · **Effort:** S–M

`bot/handler.rs::register_handlers` registers exactly two handlers: auto-join on
invite, and room messages. Fourteen modules of chat commands and no way to do
anything without typing.

- `grep -ri "m\.location\|LocationMessageEvent"` — **zero hits**

**Reactions (✅ ⏰ 🗑️) — shipped.** `bot/reaction.rs` now captures the
`event_id` when the bot sends a todo or reminder, handles
`OriginalSyncReactionEvent`, and maps the key to complete / snooze / delete.
What remains of this finding is the rest of the platform surface below.

**Voice — partly shipped.** `!say` sends a real `m.audio` voice message
(`modules/tts/`), and inbound voice notes are transcribed (`modules/voice/`).
The bot can now be spoken to and answer out loud; it still cannot be driven
without a typed command to start it.

**`m.location` → footprints.** This was speculative when first proposed; it is
not now. `modules/footprints/` has models, a repository and a Leaflet map all
waiting for input that currently must be typed. Drop a native pin in the DM,
reverse-geocode through Nominatim, and the row appears on the map.

---

## 4. Finance records the past and predicts nothing

**Impact:** critical · **Frequency:** daily · **Effort:** M

Bill payments now post to the ledger, so the ledger is finally *accurate*. It is
still not *forward-looking*.

- `grep -ri "budget"` — 6 hits, every one a reminder **retry** budget
- `grep -ri "income" modules/recurring_bills/` — **zero hits**

Two gaps, one theme. There are no budgets, so there is no target to be on track
against. And `recurring_bills` has no concept of income — salary, the single
most predictable transaction anyone has, cannot be modelled at all, even though
`Direction::Income` already exists on the transaction side
(`finance/models.rs:11`). The recurring engine is right there; it just assumes
everything recurring is a cost.

Both are small schema changes: a `direction` column on `recurring_bills`, and a
`budgets` table with per-category caps plus 80% / 100% alerts routed through the
new notification trait from §1. Together they turn the ledger from journaling
into a question the product can answer.

### Multi-currency is a wall across three modules

There is no rate source anywhere (`grep -ri "exchange_rate\|fx_rate"` — zero
hits), and every module handles that correctly by refusing to guess: `finance`
never sums across currencies, `portfolio` rejects a second currency for a held
symbol (`BuyOutcome::CurrencyMismatch`), and the retrospective keeps currencies
strictly apart.

Correct, and also the reason the product cannot state a single net-worth number
across its three pots — ledger, cash balances, holdings. `price_feed.rs` already
establishes that calling an external feed on a schedule is acceptable here. One
daily fetch into an `fx_rates` table, with conversion applied at the presentation
layer only and the stored minor units left untouched, unlocks it.

---

## 5. Filters lie, and the fix is smaller than previously recorded

**Impact:** high · **Frequency:** daily · **Effort:** XS

Both the todo and finance views filter client-side, over the rows already
fetched, while the API paginates. Switch to "Completed" and you see the completed
items *on this page*, not the completed items.

The previous roadmap called this M-sized and blamed the API. That was wrong.
**The API already supports both filters server-side** — the frontend simply does
not pass them:

- `backend/src/api/todos.rs:20-27` — `TodoQuery.completed`, threaded to
  `list_all_user` **and** `count_all_user` so the total matches the filter
- `backend/src/modules/todo/repository.rs:132-139` — `completed_clause` in SQL
- `frontend/src/modules/todo/index.tsx:455` — client-side `.filter()` instead
- `backend/src/api/finance.rs:21-26` — `LedgerQuery.direction`
- `backend/src/modules/finance/repository.rs:207-210` — `list(.., direction, ..)`
- `frontend/src/modules/finance/index.tsx:385` — client-side `.filter()` instead

Two frontend changes, no backend work. Do this first; it is nearly free.

### Search does not exist at all

`grep -ri "global_search\|commandpalette"` — zero hits. Fourteen modules, no
`Cmd+K`, no `!search`. A transaction from four months ago is reachable only by
paging to it. PostgreSQL full-text search (`tsvector`/`tsquery` with a GIN
index) over todos, transactions, bookmarks, footprints and life notes is a
single column and one endpoint, and it is the feature that stops the module
count from becoming a navigation cost.

---

## 6. The bot still promises an export the product does not have

**Impact:** high · **Frequency:** rare · **Effort:** S

Unchanged from the previous roadmap, and now a year older.

- `modules/account/mod.rs` — `WEB_ONLY` still lists `!export`
- `accounts/security_events.rs:33` — `FinancialDataExported` is **declared,
  tested, and constructed nowhere**
- `grep -ri "csv"` — zero hits across backend and frontend

`docs/ARCHITECTURE.md` documents this as reserved-but-unbuilt, which is honest. The
user-facing copy is not: `!export` directs people to a web button that does not
exist. Behind `VerifiedUser`, per the reservation's original intent.

---

## 7. Registration is open by default

**Impact:** medium · **Frequency:** once · **Effort:** XS

`check_invite` returns `Ok` when no invite secret is configured
(`api/auth/webauthn.rs:70-74`), and nothing forces one in production.
`Config::validate` already refuses to boot on localhost WebAuthn defaults — it
applies that care to the origin but not to who may sign up. For a personal
instance on the open internet, open registration is almost never intended, and
there is no admin view that would reveal it happened.

---

## 8. New modules, ranked by fit

Not by novelty. Each is judged on how much existing machinery it reuses.

**Journal / notes — build first.** There are life-day notes, bookmark reader
content and AI conversations, but nowhere to simply write. It is also the best
available corpus for the AI module, which today reasons only over structured
tool output and cannot answer anything about what the user actually thought.

**Warranty & expiry guardian.** Passports, insurance, domains, registrations.
Structurally it is `recurring_bills` with a different due-date policy and no
amount; the scheduler is reusable as-is.

**Focus / Pomodoro.** Feeds `habits`, and more interestingly feeds the
`time_allocation` grid now landing on `visualizing-life` (§9).

**Household expense splitter — defer.** Room-scoped todos *already* cross the
ownership boundary that `AGENTS.md` describes as absolute: `list_by_room` selects
on `room_id` with no `owner_id` predicate, and `!todo clear room` deletes every
completed row in the room regardless of author. Splitting would add a second and
deeper exception to a model that has one undocumented one already. Settle the
shared-data story — `FEATURE_PROPOSALS.md` §1 — before adding to it.

---

## 9. On the branch this was written from

`visualizing-life` adds a 24-slot `time_allocation` per day plus deep-work and
average-sleep aggregates (`backend/migrations/`, `life/models.rs`,
`LifeTimeGrid.tsx`). Two consequences worth deciding on deliberately:

**It is the first quantitative time series in the life module.** Mood is
categorical; hours are not. Correlating allocation against mood and habit
completion is analysis the AI module has no data to perform today —
`get_life_profile` returns aggregates only. A `get_time_allocation` tool would
let it answer "your low-mood days average 5.4h sleep," which is the kind of claim
the whole life module exists to make possible.

**It overlaps with `habits`.** A "Deep work" habit checkbox and a deep-work hour
block are the same fact recorded twice, in two tables, with two streak notions.
Decide whether time allocation *derives* habit completion before both grow
independently — this is much cheaper to settle now than after either has users.

---

## Recommended order

| # | Item | Effort | Why here |
|---|---|:---:|---|
| 1 | Server-side filters (§5) | XS | Backend already supports it; two frontend lines |
| 2 | Notification trait + Web Push + PWA (§1) | M | Unblocks every module; nothing else competes |
| 3 | ~~Matrix reactions (§3)~~ — shipped | S | Largest UX return per line in this document |
| 4 | Schedule the retrospective + digest (§2) | S | 555 lines already written and unreachable |
| 5 | Budgets + recurring income (§4) | M | Turns the ledger from record into forecast |
| 6 | Export (§6) | S | Closes an active falsehood in the UI |
| 7 | `m.location` → footprints (§3) | S | Map is built and waiting for input |
| 8 | PostgreSQL full-text search (§5) | M | Caps the navigation cost of 14 modules |

§7 (registration) is XS and independent — take it whenever it is convenient.
