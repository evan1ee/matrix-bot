# Product Roadmap

Product discovery written against the tree at `742259f` (branch `e2e-test`).
Findings are drawn from the backend crate, the SPA, and the e2e suite; every one
cites the code that causes it.

A styled version of this document is published at
<https://claude.ai/code/artifact/dc9cef3a-e719-4855-a60c-4bc8078528b7>.

Companion documents: `FEATURE_PROPOSALS.md` covers four candidate features in
implementation detail. This document is the layer above it — what users need,
what the product currently fails to deliver, and in what order to fix it.

---

## Thesis

The product's promise, from `README.md`, is that **every feature is reachable two
ways — as a chat command or through the web UI — and both land on the same data.**

The data half of that is true and well built. The reachability half is not.
**The web app can store anything and deliver nothing.** Every notification this
product sends travels through Matrix, so a user who signs in with a passkey and
never links a Matrix account owns a reminder app that cannot remind them.

Everything below is ordered around closing that gap first.

---

## 1. Who is actually using this

Derived from what the code makes possible, not from aspiration.

### The Matrix-native self-hoster — well served

Runs the binary on their own box, already lives in Element all day, wants life
admin without opening another app. They get the full product: todos, reminders,
a ledger, bills, holdings and settings, all as chat commands, with reminders
arriving as messages in a room they already read. This is the person the
codebase was built for and it shows.

### The household or shared room — half built

A couple or flatshare with the bot in a shared room: a joint grocery list, whose
turn it is to pay the internet bill. Chat supports it — `!todo room` is a genuine
room-wide view. The web UI has no concept of a shared list at all, so the moment
anyone opens the browser the shared view disappears. The feature exists on one
side of the product only.

### The web-first passkey user — poorly served

The auth system actively recruits this person: one button, no username field, no
password, no Matrix account required. They can sign up in ten seconds and then
receive nothing — no reminder, no bill warning, no due-date nudge — unless they
happen to have the tab open at the moment it fires. The product invites a user
it has no way to reach.

---

## 2. Findings

Ordered by user impact.

### 1. A reminder with no Matrix room is silently dropped

A reminder created in the web UI is stored with the literal string `web` where a
room ID belongs. When it fires, the scheduler cannot parse that as a room, so it
returns *delivered* and moves on. The only delivery is an SSE broadcast to open
browser tabs. Close the tab and the reminder fires into nothing — no retry, no
error, no trace. The same block appears in the todo view: without a Matrix room,
the UI refuses to let you attach a due reminder at all.

**Impact:** critical · **Frequency:** daily · **Effort:** M

- `backend/src/api/reminders.rs:128` — `room_id.unwrap_or_else(|| "web")`
- `backend/src/modules/reminder/scheduler.rs:58` — unparseable room returns `true`
- `frontend/src/modules/todo/index.tsx:361` — `noRoomsAvailable` blocks the reminder
- `frontend/public/` — favicon only; no manifest, no service worker, no push anywhere

### 2. Paying a bill never reaches the ledger

Marking a recurring bill paid advances the due date, increments the instalment
count and re-arms the reminder. It does not write a transaction. Rent,
subscriptions and loan instalments are usually the largest and most predictable
share of someone's spending, so the ledger is systematically wrong by exactly
the amount the product already knows about — and the only fix available to the
user is to type every bill in twice. Portfolio cash is a third disconnected pot:
money moved in or out of it never appears in finance either.

**Impact:** critical · **Frequency:** weekly · **Effort:** S

- `backend/src/modules/recurring_bills/repository.rs:152` — `mark_paid` touches one table
- `backend/src/api/recurring_bills.rs:265` — `pay_bill` emits events, writes no transaction
- `grep "transaction" modules/recurring_bills/` — zero hits

### 3. The ledger records the past but predicts nothing

Finance answers "what did I spend between these two dates, grouped by currency".
It never answers the question people actually open a money app for: *am I on
track, and what is left?* There are no budgets, no month-over-month comparison,
and no way to model recurring income — salary, the single most predictable
transaction anyone has, cannot be represented. Expense tracking without a target
is journaling, and journaling is what expense trackers get abandoned for.

**Impact:** critical · **Frequency:** daily · **Effort:** M

- `backend/src/api/finance.rs:287` — `summary` is a date range, grouped by currency
- `grep "budget"` — 6 hits, all describing reminder retry budgets
- `grep "income" modules/recurring_bills/` — zero hits

### 4. The bot promises an export the product does not have

Type `!export` and the bot replies that this action needs the web app. The web
app has no export button. There is no CSV writer anywhere in the codebase, and
the security event that would record such an export is defined but constructed
nowhere. For a self-hosted finance tool, data portability is both table stakes
and the trust argument for self-hosting in the first place — and right now the
product actively directs users to a door that isn't there.

`CLAUDE.md` already acknowledges this as reserved-but-unbuilt. The finding is
that the user-facing copy does not.

**Impact:** high · **Frequency:** rare · **Effort:** S

- `backend/src/modules/account/mod.rs` — `WEB_ONLY` lists `!export`
- `grep "csv"` — zero hits across backend and frontend
- `SecurityEventKind::FinancialDataExported` — declared, never constructed

### 5. Filters only filter the page you are looking at

Both the todo and finance views filter in the browser, over the rows already
fetched — but the API paginates. Switch to "Completed" and you see the completed
items *on this page*, not the completed items. The mismatch is invisible at ten
rows and misleading at five hundred, which is exactly the todo ceiling the
backend enforces. There is no text search anywhere in the product, so a
transaction from four months ago can only be reached by paging to it.

**Impact:** high · **Frequency:** daily · **Effort:** M

- `frontend/src/modules/finance/index.tsx:384` — client-side `tx.direction` filter
- `frontend/src/modules/todo/index.tsx:456` — client-side completed filter
- `backend/src/modules/todo/handler.rs:15` — `MAX_ROOM_TASKS = 500`

### 6. Sign-up is one button, and then the user is on their own

The passkey ceremony is the best-executed thing in the product — discoverable
credentials, no username, nothing to leak. Then the app drops the new user into
empty views with no prompt to link Matrix, set a timezone, or save recovery
codes. Recovery codes in particular are only minted if the user goes looking for
them, which means the average account has no recovery path at all. The `!link`
flow is genuinely well designed and almost nobody will discover it unaided.

**Impact:** high · **Frequency:** once · **Effort:** S

- `POST /account/recovery-codes` — on demand only, never prompted
- `frontend/src/App.tsx` — no first-run state between sign-in and the dashboard

### 7. Shared lists work in chat and vanish on the web

The backend carries room-scoped todos, and they are more genuinely *shared* than
`FEATURE_PROPOSALS.md` currently records. `list_by_room` selects on `room_id`
with **no owner filter**, and `!todo clear room` deletes every completed row in
the room regardless of who created it. So room-wide reads and deletes already
cross the ownership boundary `CLAUDE.md` describes as absolute.

The web UI is strictly per-owner and shows no room scope anywhere. A household
that builds a shared grocery list in chat cannot see it in the browser, which
reads as data loss rather than a missing view — and one member can already clear
another's completed items with no web-side record that it happened.

This is a decision, not just a gap. Either the shared list is a product or it
isn't, and right now it is half of one with an undocumented ownership exception.

**Impact:** medium · **Frequency:** weekly · **Effort:** M

- `backend/src/modules/todo/repository.rs:116` — room read, no `owner_id` predicate
- `backend/src/modules/todo/repository.rs:316` — room clear, no `owner_id` predicate
- `frontend/src/modules/todo/index.tsx` — owner-scoped only, no room view
- `FEATURE_PROPOSALS.md` §1 — proposes shared lists as new work; partly already shipped

### 8. Anyone who can reach the port can create an account

Registration requires an invite secret only if one is configured, and nothing
forces that in production. `Config::validate` already refuses to boot with
localhost WebAuthn defaults — it applies the same care to the origin but not to
who may sign up. For a personal instance exposed to the internet, open
registration is almost never what the operator intended, and there is no admin
view that would show them it happened.

**Impact:** medium · **Frequency:** once · **Effort:** XS

- `backend/src/api/auth/webauthn.rs:74` — `check_invite` returns `Ok` when unset
- `backend/src/config.rs` — `validate` guards WebAuthn defaults, not registration

### 9. The most sophisticated module has the least sustainable input

Portfolio is the strongest engineering in the codebase: weighted-average cost,
realised P&L in 128-bit arithmetic, a cash balance that gates every buy inside a
single call so no concurrent purchase can race it. All of it hangs off a price
the user has to type in by hand, and market value stays undefined until they do.
The engine deserves a feed; without one, the module is a spreadsheet with a
command syntax, and it will be the first thing people stop updating.

**Impact:** medium · **Frequency:** weekly · **Effort:** M

- `backend/src/modules/portfolio/` — market value and P&L are `Option` until priced
- `CLAUDE.md` — "there is no live price feed anywhere in this codebase"

### 10. The room picker asks a question the user answers once

Setting a reminder or a due date in the web UI means choosing a destination room
each time, from a list most users will have exactly one entry in. It is a
per-item decision that belongs in settings, alongside timezone and default
currency, where the same pattern already works well.

**Impact:** low · **Frequency:** daily · **Effort:** XS

- `frontend/src/modules/reminder/index.tsx:273`, `todo/index.tsx:208` — per-item pickers
- `backend/src/modules/settings/` — the pattern a default would follow

---

## 3. Prioritisation

Rank is user impact weighted by how often the need recurs, divided by build
cost. The top three are all cheap relative to what they unblock.

| Rank | Work | Impact | Frequency | Effort | Unblocks |
|-----:|------|--------|-----------|--------|----------|
| 1 | Bill payment writes a transaction | Critical | Weekly | S | A ledger that is actually correct |
| 2 | Web push notifications + installable app | Critical | Daily | M | The entire web-first segment |
| 3 | CSV export behind step-up | High | Rare | S | Trust; keeps an existing promise |
| 4 | Require an invite secret in production | Medium | Once | XS | Safe public deployment |
| 5 | Budgets per category, month to date | Critical | Daily | M | The reason to open the app daily |
| 6 | Recurring income & cash-flow forecast | High | Weekly | M | "What is left" as a real answer |
| 7 | Server-side search and filtering | High | Daily | M | Usability past ~100 rows |
| 8 | First-run checklist | High | Once | S | Activation; recovery coverage |
| 9 | Shared room views on the web | Medium | Weekly | M | The household segment |
| 10 | Portfolio price feed | Medium | Weekly | M | Portfolio staying current |

**The ordering argument.** Push notifications are ranked below the bill fix only
because the bill fix is a few hours of work against a systematic data error.
Everything else in this roadmap is worth less than it looks until push ships — a
budget you are not warned about, a bill reminder that cannot arrive, and a
forecast nobody sees are all features that store value rather than deliver it.

---

## 4. Roadmap

Sized for a small team, in the layered style the project already follows: never
trade a working product for unfinished complexity.

### P0 — Now (~3 weeks): keep the promises already made

Nothing here is a new feature. Each item makes an existing one true.

- **Bill payment posts to the ledger.** `mark_paid` writes a transaction
  carrying the bill's category and currency, with a link back to the bill so it
  can be traced or reversed. The same treatment for portfolio cash movements.
- **Web push and an installable app.** A VAPID subscription table, a service
  worker, and a `deliver_to_web` sibling to `deliver_to_matrix` so the scheduler
  has a second real channel instead of a sentinel it silently discards. This
  deletes the `noRoomsAvailable` block outright.
- **CSV export behind `VerifiedUser`.** The step-up boundary and the audit event
  are already built and waiting for it; `!export` starts telling the truth.
- **Refuse to boot production without an invite secret**, in the same
  `Config::validate` that already refuses localhost WebAuthn defaults.

### P1 — Next (~5 weeks): close the money loop

Turn a record of the past into a question the app can answer every morning.

- **Budgets per category.** A monthly target, month-to-date progress on the
  dashboard, a push when a category crosses 80% and again at 100%, and a
  `!budget` command for parity. This is the single change most likely to make
  the app a daily habit.
- **Recurring income.** A direction flag on the recurring-bills model reuses the
  schedule, instalment and reminder machinery almost entirely, and unlocks a
  forecast: committed outgoings against expected income, for the rest of the month.
- **Search and filtering on the server.** Move the direction, status and text
  filters behind the pagination boundary so a filter means what it says at any
  data size.

### P2 — Then (~3 weeks): finish what is half built

Two features currently exist on one side of the product only; pick a side for each.

- **First-run checklist.** Four steps after the first sign-in: link Matrix, set
  timezone and currency, enable notifications, save recovery codes. Dismissible,
  never blocking, and it fixes the accounts-with-no-recovery-path problem at the
  source.
- **Shared lists on the web** — a room-scoped todo view matching `!todo room`,
  and a documented ownership exception for room-wide reads and deletes. If
  shared lists are not a product direction, remove the room scope from chat
  instead; leaving it visible in one client and absent in the other is the worst
  of both. See `FEATURE_PROPOSALS.md` §1 for the implementation shape.
- **A default reminder destination in settings**, retiring the per-item room pickers.

### P3 — Later (scoped after P1): decide what portfolio is

A deliberate call, not a backlog item — the module's cost is already paid and
its value depends entirely on this.

- **Either a pluggable price source** — one trait, one optional provider, prices
  refreshed on a schedule alongside the reminder tick — **or an explicit
  repositioning** as a manual position tracker, with the UI saying so and no
  further investment.
- **Revisit after budgets ship.** If usage shows portfolio prices going stale
  within a month of entry, that is the answer, and the cheap option is the right one.

---

## 5. What to stop

| Verdict | Surface |
|---------|---------|
| **Cut** | `!hello` and the greeting handler. It was the first command in a "hello bot" and it has outlived that; it dilutes `!help` for a product that now runs someone's money. |
| **Cut** | Per-item room pickers, once a default destination exists in settings. |
| **Decide** | Room-scoped todos. Finish them on the web or remove them from chat — the current split reads to a household as data that disappeared. |
| **Freeze** | Cross-signing and self-verification. Correct, hard-won, and worth every line — but it is infrastructure with no user-visible feature attached. Treat it as a fixed cost that is now paid, and resist further investment unless something breaks. |
| **Keep** | The account model. One owner, several access paths, ownership carried rather than derived. It is the reason a single fix to bill payment lands correctly for both front doors, and it is what makes every item in P0 and P1 cheap. |
