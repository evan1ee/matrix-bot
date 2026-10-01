# Sub-project 2, Plan A: Channel framework, identity, linking, Matrix adapter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Matrix runs as the first `ChannelAdapter` behind a `ChannelRegistry`; every inbound message flows through one channel-neutral pipeline into the shared `BotModule` registry; platform identities live in `platform_identities`; account linking is chat-initiated and finished in the browser with a passkey; the agent (`llm/`) no longer knows Matrix exists. Discord, Telegram and Email (plans B–D) then only add adapters.

**Architecture:**
- **Domain** `domain/channels/` — `Channel`, `PlatformUserId`, `PlatformIdentity`, `ConversationAddress` (storage and destination encodings), `ProvisioningPolicy`, the pure link decision.
- **Persistence** — migrations `0006` (DDL), `0007` (backfill from `matrix_links`, drop it), `0008` (drop `matrix_link_challenges`); repositories `identities.rs`, `link_tokens.rs`.
- **Application** `application/identity/` — `IdentityService` (resolve under the channel's provisioning policy, list, unlink) and `LinkService` (start, inspect, claim, claim with signup).
- **Interface** `interface/channels/` — `IncomingMessage`/`OutgoingMessage`/`Content`/`Reply`, the `ChannelAdapter` trait, `ChannelRegistry`, the shared inbound pipeline, renderers; `matrix/` becomes `MatrixAdapter`.
- **Agent** — `llm::progress::ProgressSink` replaces `StageReporter`; tool targets come back in `AiTurnResult`; the outage alert goes through the registry.

**Tech Stack:** Rust, axum, sqlx 0.9 (`query!` macros), PostgreSQL 18, tokio + `tokio_util::sync::CancellationToken`, matrix-sdk 0.18, webauthn-rs (existing), React 19 + Vite (link page), Playwright.

**Spec:** `docs/superpowers/specs/2026-10-01-omnichannel-design.md` — read it first; this plan argues from it. Also read `AGENTS.md` and `docs/superpowers/HANDOFF.md` §5, §6, §8.

**Worktree:** `D:\01_Data\code\matrix-hello-bot-outbox`, branch `feat/omnichannel` (stacked on `refactor/outbox`). Never work in the main checkout.

## Global Constraints

- **Layers** (`AGENTS.md` §3.1 plus this sub-project): `domain/channels` is pure. `application/` and `llm/` import no `matrix_sdk`, no `interface::channels::matrix`, and no future platform crate (`serenity`, `teloxide`, `lettre`, `async_imap`, `mail_parser`). `interface/chat/` imports no `matrix_sdk`. `interface/channels/**` runs no SQL. Only `app.rs` constructs adapters and the `ChannelRegistry`.
- **Frozen Matrix behaviour.** Every chat command and reply on Matrix keeps its wording and HTML, except `!link` (new flow, spec D1). E2EE, `ensure_self_verified` on every boot, verification, invites, reactions, attachments and the stage indicator behave exactly as before.
- **OpenAPI.** `backend/openapi.json` is byte-identical after Tasks 1–9. Task 10 changes it intentionally (spec §7.3) and regenerates the frontend client. `GET /api/v1/auth/me` keeps its `linked_mxids` field (computed from Matrix identities).
- **SQL.** Only `sqlx::query!` / `query_as!` / `query_scalar!`; never format a value into SQL text. Regenerate `.sqlx/` whenever a query changes and commit it. Migrations are forward-only, LF line endings (`.gitattributes` already pins `backend/migrations/*`), never edited once committed. A data-rewriting migration starts with a guard naming offending rows and is separate from DDL.
- **Merge bookkeeping.** Every `account_id`-keyed table is in exactly one of `ACCOUNT_ID_FOLLOWS` / `ACCOUNT_ID_DISCARDED` / `ACCOUNT_ID_UNTOUCHED` (`domain/accounts/mod.rs`); the schema-catalog test enforces it.
- **Security.** Link tokens: 32 random bytes, base64url, stored as SHA-256 only, 10-minute TTL, single use, row-locked on claim. `/api/v1/link/claim` requires `VerifiedUser` and is listed in `SENSITIVE`. Never log a token.
- **File size.** No file in `backend/src/` exceeds 500 production lines (`#[cfg(test)]` excluded). `interface/chat/account.rs`, `interface/channels/matrix/reaction.rs` and `verification.rs` are already large — split along a real seam if a change pushes one over.
- **Error handling.** No `unwrap()`/`expect()` on runtime paths. Every long-lived task gets a child `CancellationToken` and joins `app::run`'s shutdown.
- **Hard safety rules** (HANDOFF §6): never read `.env` or `data/` of the main checkout, never run the bot binary (only `cargo run --bin generate-openapi`), never contact a homeserver or external service from tests, never push/merge/PR, never bare `git stash`.
- **Commits.** Conventional Commits, one logical change each, ending with the committing model's trailer, e.g. `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

### The Gate (run at the end of every task; every line must pass)

```bash
cd D:/01_Data/code/matrix-hello-bot-outbox
export TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
export DATABASE_URL=$TEST_DATABASE_URL
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo test  -p matrix-bot --all-targets
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # Tasks 1–9 only; Task 10+ commits the regenerated file
git diff --check
```

From Task 10 on, also: `npm --prefix frontend run build` and `npm --prefix frontend run test:e2e`.

If the test container is not running: `docker start pg18-test`, or `docker run -d --name pg18-test --shm-size=1g -p 55433:5432 -e POSTGRES_PASSWORD=bot postgres:18`. After a query change run `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` before the `--check` line.

> **Pre-existing dirt.** `backend/openapi.json` was modified in the working tree before this plan began. Before Task 1, run `cargo run --bin generate-openapi` and `git diff backend/openapi.json`: if the regenerated file equals `HEAD`, the dirt was stale — `git checkout -- backend/openapi.json`; otherwise stop and ask the user.

## Plan clarifications (supersede the spec where they differ; record in HANDOFF as rulings)

- **A1 — migration numbers.** `0005` is taken (`0005_drop_webhook_outbox.sql`). The spec's `0005`/`0006` become `0006_platform_identities.sql` (DDL), `0007_platform_identities_backfill.sql` (guarded copy + drop `matrix_links`) and `0008_drop_matrix_link_challenges.sql` (after the old flow is deleted in Task 9). Spec §7.1 already uses these numbers.
- **A2 — Matrix-specific lookups stay, re-pointed.** Room authorization (`domain/rooms`), Matrix login (`/auth/matrix/login`), notifications and `/auth/me` legitimately need "the MXIDs of this account". `find_by_mxid`/`linked_mxids` move to `repositories::identities` as `find_account(conn, &PlatformIdentity)` and `user_ids(conn, account, Channel)`; `AccountService::{find_by_mxid, linked_mxids}` are deleted and callers use `IdentityService::{find_account, user_ids}` with `Channel::Matrix`.
- **A3 — merge on link keeps today's rule.** An identity already mapped to another account can be moved only when that account `is_unclaimed()` (auto-provisioned); otherwise `AlreadyLinked`. The "previously unlinked from an account with data" merge prompt (`security_events.matrix_unlinked` lookup in `linking::claim`) is kept, generalised to `identity_unlinked` with detail `<platform>:<user_id>`; existing `matrix_unlinked` rows are still honoured for Matrix.
- **A4 — the AI outage alert.** Target is `settings.dev_room_id` parsed as a `ConversationAddress`; when unset, `ChannelRegistry::find_conversation_by_name(Channel::Matrix, "dev")` (a default-`None` adapter method the Matrix adapter implements with today's joined-room scan). Behaviour on Matrix is unchanged.
- **A5 — no `Arc` cycle.** Adapters own the inbound pipeline; the pipeline needs the registry for out-of-band sends. The pipeline holds `Weak<ChannelRegistry>` (`ChannelRegistry::new_cyclic`-style construction in `app.rs`: build the registry with `Arc::new_cyclic`, hand the `Weak` to each adapter's pipeline).
- **A6 — `Reply` from `String`.** `impl From<String> for Reply` (HTML, `Visibility::Normal`, plain = `strip_html`) so the ~20 modules change their return type mechanically; modules returning private data use `Reply::private(html)`.

---

## File map

| File | Responsibility | Task |
|---|---|---|
| `backend/src/domain/channels/mod.rs` | `Channel`, `PlatformUserId`, `PlatformIdentity`, `ConversationAddress`, `ProvisioningPolicy` | 1 |
| `backend/src/domain/channels/link.rs` | `LinkDecision`, `decide_link`, token TTL constant | 8 |
| `backend/migrations/0006_platform_identities.sql` | DDL | 2 |
| `backend/migrations/0007_platform_identities_backfill.sql` | guard, copy, drop `matrix_links` | 2 |
| `backend/migrations/0008_drop_matrix_link_challenges.sql` | drop old challenge table | 9 |
| `backend/src/infrastructure/postgres/repositories/identities.rs` | identity queries | 2 |
| `backend/src/infrastructure/postgres/repositories/link_tokens.rs` | token queries | 8 |
| `backend/src/application/identity/mod.rs` | `IdentityService` | 3 |
| `backend/src/application/identity/link.rs` | `LinkService` | 8 |
| `backend/src/interface/channels/message.rs` | `IncomingMessage`, `OutgoingMessage`, `Target`, `Content`, `Attachment`, `Action`, `MessageRef`, `Visibility`, `Reply` | 4 |
| `backend/src/interface/channels/adapter.rs` | `ChannelAdapter`, `ChannelError`, `SentRef` | 4 |
| `backend/src/interface/channels/registry.rs` | `ChannelRegistry` | 4 |
| `backend/src/interface/channels/render/{mod,html}.rs` | HTML-subset sanitizer, `strip_html` re-export | 4 |
| `backend/src/interface/channels/fake.rs` (`#[cfg(test)]`) | `FakeAdapter` recording sends | 4 |
| `backend/src/llm/progress.rs` | `ProgressSink` trait, `NoProgress` | 5 |
| `backend/src/interface/chat/mod.rs` | neutral `CommandContext`, `BotModule -> Option<Reply>` | 6 |
| `backend/src/interface/channels/inbound.rs` | the shared inbound pipeline | 7 |
| `backend/src/interface/channels/matrix/adapter.rs` | `MatrixAdapter: ChannelAdapter` | 7 |
| `backend/src/interface/channels/matrix/inbound.rs` | Matrix event → `IncomingMessage` (replaces `command_flow.rs`) | 7 |
| `backend/src/interface/channels/matrix/outbound.rs` | `OutgoingMessage` → Matrix sends | 7 |
| `backend/src/interface/jobs/mod.rs` | `Outbound` replaces `Delivery` | 8 |
| `backend/src/interface/events/channel_sink.rs` | `ChannelSink` replaces `MatrixRoomSink` | 8 |
| `backend/src/interface/chat/link.rs` | `link` command handler (start) | 9 |
| `backend/src/interface/http/identities.rs`, `interface/http/link.rs` | new routes | 10 |
| `frontend/src/modules/link/*`, `frontend/src/components/SecurityPanel.tsx` | link page, identity list | 11 |
| `backend/tests/architecture.rs` | new boundary tests | 12 |

Task order matters: each task leaves the whole workspace compiling and the Gate green.

---

### Task 1: Domain channel types

**Files:**
- Create: `backend/src/domain/channels/mod.rs`
- Modify: `backend/src/domain/mod.rs` (add `pub mod channels;`)

**Interfaces:**
- Produces:
  - `pub enum Channel { Matrix, Discord, Telegram, Email }` with `as_str(&self) -> &'static str`, `FromStr` (`"matrix"|"discord"|"telegram"|"email"`), `Display`, `Copy, Eq, Hash, Debug, Serialize, Deserialize` (serde lowercase).
  - `pub struct PlatformUserId(String)` with `PlatformUserId::parse(channel: Channel, raw: &str) -> Result<Self, ChannelsError>` and `as_str()`.
  - `pub struct PlatformIdentity { channel, user_id, display_name: Option<String> }` with `new(channel, user_id: PlatformUserId, display_name: Option<String>)`, `matrix(mxid: &str) -> Result<Self, ChannelsError>`, getters `channel()`, `user_id()`, `display_name()`.
  - `pub enum ConversationAddress { Web, Matrix(String), Discord(u64), Telegram(i64), Email(String) }` with `channel(&self) -> Option<Channel>` (`Web` → `None`), `parse_storage(&str) -> Result<Self, ChannelsError>`, `to_storage(&self) -> String`, `parse_destination(&str) -> Option<Self>`, `to_destination(&self) -> Option<String>` (`Web` → `None`).
  - `pub enum ProvisioningPolicy { ProvisionUnknown, RequireLink }`, `pub fn provisioning_policy(channel: Channel) -> ProvisioningPolicy`.
  - `#[derive(thiserror::Error)] pub enum ChannelsError { InvalidUserId { channel: Channel, reason: &'static str }, InvalidAddress(String) }`.

- [ ] **Step 1: Write the failing tests** (in `domain/channels/mod.rs`, `#[cfg(test)] mod tests`)

```rust
#[test]
fn storage_form_round_trips_and_keeps_bare_matrix_ids() {
    for raw in ["web", "!abc:mbm.one", "discord:123456789012345678", "telegram:-1001234567890", "email:evan@example.com"] {
        let addr = ConversationAddress::parse_storage(raw).unwrap();
        assert_eq!(addr.to_storage(), raw);
    }
    assert_eq!(
        ConversationAddress::parse_storage("!abc:mbm.one").unwrap(),
        ConversationAddress::Matrix("!abc:mbm.one".into())
    );
}

#[test]
fn destination_form_always_carries_the_channel_prefix() {
    let m = ConversationAddress::Matrix("!abc:mbm.one".into());
    assert_eq!(m.to_destination().as_deref(), Some("matrix:!abc:mbm.one"));
    assert_eq!(ConversationAddress::parse_destination("matrix:!abc:mbm.one"), Some(m));
    assert_eq!(ConversationAddress::Web.to_destination(), None);
    assert_eq!(ConversationAddress::parse_destination("telegram:42"), Some(ConversationAddress::Telegram(42)));
}

#[test]
fn malformed_addresses_are_rejected() {
    for raw in ["", "matrix:", "discord:abc", "telegram:", "email:not-an-address", "slack:1", "abc"] {
        assert!(ConversationAddress::parse_storage(raw).is_err(), "{raw} accepted");
    }
}

#[test]
fn user_ids_are_validated_per_channel() {
    assert!(PlatformUserId::parse(Channel::Matrix, "@evan:mbm.one").is_ok());
    assert!(PlatformUserId::parse(Channel::Matrix, "evan").is_err());
    assert!(PlatformUserId::parse(Channel::Discord, "123456789012345678").is_ok());
    assert!(PlatformUserId::parse(Channel::Discord, "-1").is_err());
    assert!(PlatformUserId::parse(Channel::Telegram, "987654321").is_ok());
    assert!(PlatformUserId::parse(Channel::Telegram, "x").is_err());
    assert_eq!(PlatformUserId::parse(Channel::Email, " Evan@Example.COM ").unwrap().as_str(), "evan@example.com");
    assert!(PlatformUserId::parse(Channel::Email, "evan@").is_err());
    assert!(PlatformUserId::parse(Channel::Email, &format!("{}@x.io", "a".repeat(250))).is_err());
}

#[test]
fn only_matrix_provisions_unknown_identities() {
    assert_eq!(provisioning_policy(Channel::Matrix), ProvisioningPolicy::ProvisionUnknown);
    for c in [Channel::Discord, Channel::Telegram, Channel::Email] {
        assert_eq!(provisioning_policy(c), ProvisioningPolicy::RequireLink);
    }
}

#[test]
fn channel_names_round_trip() {
    for c in [Channel::Matrix, Channel::Discord, Channel::Telegram, Channel::Email] {
        assert_eq!(c.as_str().parse::<Channel>().unwrap(), c);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p matrix-bot domain::channels::`
Expected: compile error — module `channels` not found.

- [ ] **Step 3: Implement**

Rules (write them as code; no other behaviour):
- Matrix user id: starts with `@`, contains `:`, length ≤ 255. Matrix room id (address): starts with `!`, contains `:`.
- Discord id: decimal `u64`, non-zero. Telegram user id: decimal `i64` > 0; Telegram chat id (address): any non-zero `i64` (groups are negative).
- Email: trim, lowercase, exactly one `@`, non-empty local part and a domain containing `.`, total ≤ 254.
- Storage form: `web` → `Web`; leading `!` → `Matrix`; otherwise split once on `:` into a known non-Matrix channel prefix and a validated id. `matrix:` prefix is **not** accepted in storage form (bare ids are canonical, spec R-O1).
- Destination form: `<channel>:<id>` for every channel including Matrix; `Web` has none.
- Module docs explain the two encodings and why Matrix stays bare in storage.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p matrix-bot domain::channels::` → PASS. Then the Gate.

- [ ] **Step 5: Commit**

```bash
git add backend/src/domain/channels backend/src/domain/mod.rs
git commit -m "feat(domain): channel, platform identity and conversation address types"
```

---

### Task 2: `platform_identities` schema and repository; `matrix_links` retired

**Files:**
- Create: `backend/migrations/0006_platform_identities.sql`, `backend/migrations/0007_platform_identities_backfill.sql`, `backend/src/infrastructure/postgres/repositories/identities.rs`
- Modify: `backend/src/infrastructure/postgres/repositories/mod.rs`, `repositories/accounts/core.rs` (`find_by_mxid`, `linked_mxids`, `resolve_or_provision` move out), `repositories/accounts/linking.rs` (inserts/deletes target the new table), `backend/src/infrastructure/postgres/merge.rs`, `backend/src/domain/accounts/mod.rs` (`ACCOUNT_ID_FOLLOWS`: `matrix_links` → `platform_identities`), every caller listed by `grep -rn "core::find_by_mxid\|core::linked_mxids\|core::resolve_or_provision" backend/src`.
- Test: in `identities.rs`; migration test in `backend/src/db/migrate.rs` tests.

**Interfaces:**
- Consumes: Task 1 types.
- Produces (`repositories::identities`):

```rust
pub async fn find_account(conn: impl sqlx::PgExecutor<'_>, identity: &PlatformIdentity) -> Result<Option<AccountId>, sqlx::Error>;
pub async fn user_ids(conn: impl sqlx::PgExecutor<'_>, account: &AccountId, channel: Channel) -> Result<Vec<String>, sqlx::Error>;
pub async fn list(conn: impl sqlx::PgExecutor<'_>, account: &AccountId) -> Result<Vec<LinkedIdentity>, sqlx::Error>;
/// Inserts; Ok(false) if (platform, platform_user_id) already exists.
pub async fn bind(conn: impl sqlx::PgExecutor<'_>, account: &AccountId, identity: &PlatformIdentity) -> Result<bool, sqlx::Error>;
pub async fn rebind(conn: impl sqlx::PgExecutor<'_>, to: &AccountId, identity: &PlatformIdentity) -> Result<bool, sqlx::Error>;
pub async fn unbind(conn: impl sqlx::PgExecutor<'_>, account: &AccountId, channel: Channel, user_id: &str) -> Result<bool, sqlx::Error>;
pub async fn touch_display_name(conn: impl sqlx::PgExecutor<'_>, identity: &PlatformIdentity) -> Result<(), sqlx::Error>;
/// Today's `core::resolve_or_provision`, generalised: find, else create an unclaimed account + bind, race-safe.
pub async fn resolve_or_provision(pool: &PgPool, identity: &PlatformIdentity) -> Result<AccountId, sqlx::Error>;
```

`LinkedIdentity { channel: Channel, user_id: String, display_name: Option<String>, linked_at: DateTime<Utc> }` lives in `domain/channels/mod.rs` (add it there with a `pub fn new`).

- [ ] **Step 1: Write the migrations**

`0006_platform_identities.sql`:

```sql
-- Sub-project 2: one identity table for every channel (spec §7.1).
CREATE TABLE platform_identities (
    id               BIGSERIAL PRIMARY KEY,
    account_id       UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    platform         TEXT NOT NULL CHECK (platform IN ('matrix', 'discord', 'telegram', 'email')),
    platform_user_id TEXT NOT NULL,
    display_name     TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform, platform_user_id)
);
CREATE INDEX idx_platform_identities_account ON platform_identities(account_id);

CREATE TABLE link_tokens (
    token_hash       TEXT PRIMARY KEY,
    platform         TEXT NOT NULL CHECK (platform IN ('matrix', 'discord', 'telegram', 'email')),
    platform_user_id TEXT NOT NULL,
    display_name     TEXT,
    conversation     TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,
    consumed_at      TIMESTAMPTZ
);
CREATE INDEX idx_link_tokens_identity ON link_tokens(platform, platform_user_id) WHERE consumed_at IS NULL;
```

`0007_platform_identities_backfill.sql`:

```sql
-- Guard: refuse to migrate links whose account no longer exists (names the rows).
DO $$
DECLARE orphans TEXT;
BEGIN
    SELECT string_agg(l.mxid, ', ') INTO orphans
    FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id
    WHERE a.id IS NULL;
    IF orphans IS NOT NULL THEN
        RAISE EXCEPTION 'matrix_links rows reference missing accounts: %', orphans;
    END IF;
END $$;

INSERT INTO platform_identities (account_id, platform, platform_user_id, created_at, updated_at)
SELECT account_id, 'matrix', mxid, COALESCE(linked_at, now()), COALESCE(linked_at, now())
FROM matrix_links;

DROP TABLE matrix_links;
```

(`matrix_links.account_id` is `UUID` with an FK since `0002`, so the guard is belt-and-braces for a hand-edited database; keep it — AGENTS §6.)

- [ ] **Step 2: Write the failing repository tests** (`identities.rs`)

```rust
#[tokio::test]
async fn a_platform_user_id_belongs_to_at_most_one_account() {
    let p = pool().await;
    let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
    let b = core::create(&p, "b", STATUS_ACTIVE).await.unwrap();
    let id = tg("987654321");
    assert!(bind(&p, &a, &id).await.unwrap());
    assert!(!bind(&p, &b, &id).await.unwrap(), "second bind must not steal the identity");
    assert_eq!(find_account(&p, &id).await.unwrap(), Some(a));
}

#[tokio::test]
async fn every_channel_resolves_to_the_same_account_once_bound() {
    let p = pool().await;
    let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
    for id in [mx("@evan:mbm.one"), dc("123456789012345678"), tg("987654321"), em("evan@example.com")] {
        bind(&p, &a, &id).await.unwrap();
        assert_eq!(find_account(&p, &id).await.unwrap(), Some(a.clone()));
    }
    assert_eq!(list(&p, &a).await.unwrap().len(), 4);
    assert_eq!(user_ids(&p, &a, Channel::Matrix).await.unwrap(), vec!["@evan:mbm.one"]);
}

#[tokio::test]
async fn identities_are_isolated_per_account() {
    let p = pool().await;
    let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
    let b = core::create(&p, "b", STATUS_ACTIVE).await.unwrap();
    bind(&p, &a, &tg("1")).await.unwrap();
    assert!(list(&p, &b).await.unwrap().is_empty());
    assert!(!unbind(&p, &b, Channel::Telegram, "1").await.unwrap(), "b cannot unbind a's identity");
}

#[tokio::test]
async fn resolve_or_provision_is_idempotent_and_creates_an_unclaimed_account() {
    let p = pool().await;
    let id = mx("@new:mbm.one");
    let first = resolve_or_provision(&p, &id).await.unwrap();
    let second = resolve_or_provision(&p, &id).await.unwrap();
    assert_eq!(first, second);
    assert!(core::find(&p, &first).await.unwrap().unwrap().is_unclaimed());
}
```

with helpers `mx/dc/tg/em(&str) -> PlatformIdentity` and `pool()` as in `accounts/core.rs` tests.

Add to `db/migrate.rs` tests a backfill test: run migrations up to `0005` on a fresh database (use the existing per-version helper if there is one; otherwise apply `0001`–`0005` with `sqlx::migrate!().run` against a `Migrator` filtered by version), insert an account and a `matrix_links` row, run the rest, assert one `platform_identities` row with `platform = 'matrix'` and that `matrix_links` no longer exists.

- [ ] **Step 3: Run to verify they fail** — `cargo test -p matrix-bot repositories::identities::` → compile error.

- [ ] **Step 4: Implement**
  - `identities.rs` with the functions above (private `IdentityRow` converted at the edge; `platform` parsed with `Channel::from_str`, a bad value is `sqlx::Error::Decode`).
  - Delete `core::find_by_mxid`, `core::linked_mxids`, `core::resolve_or_provision`; update their callers to `identities::find_account(&PlatformIdentity::matrix(mxid)?)`, `identities::user_ids(.., Channel::Matrix)`, `identities::resolve_or_provision(.., &PlatformIdentity::matrix(mxid)?)`. In tests use `.unwrap()`; at runtime a malformed MXID from the homeserver is logged and the event dropped (it cannot happen for a real sender, but no `expect`).
  - `linking.rs`: `INSERT INTO matrix_links` / `DELETE FROM matrix_links` → `identities::bind` / `identities::unbind` with `Channel::Matrix`. The flow itself is replaced in Task 9; here it only changes table.
  - `merge.rs` moves every `ACCOUNT_ID_FOLLOWS` table generically (`move_rows`), so the list change below is the whole move; delete the `"matrix_links" => "mxid"` arm of `row_key` (`platform_identities` is keyed by `id`, the default arm). The table allow-list in `assert_allowed_table` derives from the lists, so it follows automatically.
  - `ACCOUNT_ID_FOLLOWS`: replace `"matrix_links"` with `"platform_identities"`. `link_tokens` has no `account_id` column, so it is in no list.
  - Regenerate `.sqlx/`.

- [ ] **Step 5: Run the Gate.** The schema-catalog test and every existing Matrix/account test must pass unchanged in meaning.

- [ ] **Step 6: Commit**

```bash
git add backend/migrations/0006_platform_identities.sql backend/migrations/0007_platform_identities_backfill.sql backend/src backend/.sqlx
git commit -m "feat(identity): platform_identities replaces matrix_links"
```

---

### Task 3: `IdentityService`

**Files:**
- Create: `backend/src/application/identity/mod.rs`
- Modify: `backend/src/application/mod.rs` (`AppServices.identity: IdentityService`, built in `AppServices::new`), `backend/src/application/accounts/mod.rs` (delete `resolve_or_provision`, `find_by_mxid`, `linked_mxids`, `unlink`), every caller (`grep -rn "accounts\.\(resolve_or_provision\|find_by_mxid\|linked_mxids\|unlink\)" backend/src`), `application/rooms/mod.rs` (`linked_mxids` delegates to `identity.user_ids(.., Channel::Matrix)`).

**Interfaces:**
- Consumes: Task 2 repository.
- Produces:

```rust
#[derive(Clone)]
pub struct IdentityService { pool: PgPool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution { Account(AccountId), Unlinked }

impl IdentityService {
    pub fn new(pool: PgPool) -> Self;
    /// Applies `provisioning_policy(identity.channel())`; refreshes a changed display name.
    pub async fn resolve(&self, identity: &PlatformIdentity) -> Result<Resolution, AppError>;
    pub async fn find_account(&self, identity: &PlatformIdentity) -> Result<Option<AccountId>, AppError>;
    pub async fn user_ids(&self, account: &AccountId, channel: Channel) -> Result<Vec<String>, AppError>;
    pub async fn list(&self, account: &AccountId) -> Result<Vec<LinkedIdentity>, AppError>;
    /// Records `identity_unlinked` (detail "<platform>:<user_id>") in the same transaction.
    /// Refuses (Conflict) removing the last identity of an account that has no passkey.
    pub async fn unlink(&self, account: &AccountId, channel: Channel, user_id: &str) -> Result<bool, AppError>;
}
```

Add `SecurityEventKind::IdentityLinked` and `IdentityUnlinked` (`snake_case`: `identity_linked`, `identity_unlinked`). Keep `MatrixLinked`/`MatrixUnlinked` so historical rows still deserialize; new rows use the new kinds. The audit DTO (`interface/http/account/types.rs`) exposes `kind` as a `String`, so OpenAPI is unaffected; add labels for the two new kinds wherever the frontend maps kinds to text (`grep -rn "matrix_unlinked" frontend/src`).

- [ ] **Step 1: Write the failing tests** (`application/identity/mod.rs`)

```rust
#[tokio::test]
async fn an_unknown_matrix_user_is_provisioned() {
    let s = service().await;
    let r = s.resolve(&mx("@new:mbm.one")).await.unwrap();
    assert!(matches!(r, Resolution::Account(_)));
}

#[tokio::test]
async fn an_unknown_telegram_user_is_not_provisioned() {
    let s = service().await;
    let before = s.count_accounts_for_test().await;
    assert_eq!(s.resolve(&tg("987654321")).await.unwrap(), Resolution::Unlinked);
    assert_eq!(s.count_accounts_for_test().await, before, "no account may be created");
}

#[tokio::test]
async fn a_bound_identity_on_any_channel_resolves_to_its_account() {
    let (s, pool) = service_with_pool().await;
    let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
    for id in [dc("123456789012345678"), tg("987654321"), em("evan@example.com")] {
        identities::bind(&pool, &a, &id).await.unwrap();
        assert_eq!(s.resolve(&id).await.unwrap(), Resolution::Account(a.clone()));
    }
}

#[tokio::test]
async fn unlinking_the_last_way_in_to_a_passkeyless_account_is_refused() {
    let (s, pool) = service_with_pool().await;
    let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
    identities::bind(&pool, &a, &tg("1")).await.unwrap();
    assert!(matches!(s.unlink(&a, Channel::Telegram, "1").await, Err(AppError::Conflict(_))));
}

#[tokio::test]
async fn unlinking_records_a_security_event() {
    // bind two identities, unlink one, assert one identity_unlinked (or matrix_unlinked, see above) row for `a`.
}
```

Write the last test out fully against `repositories::accounts::security_events::list_for_account` (whatever the existing read function is named — `grep -n "pub async fn" backend/src/infrastructure/postgres/repositories/accounts/security_events.rs`). `count_accounts_for_test` is a `#[cfg(test)]` helper calling `core::count`.

- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement** the service; delete the four `AccountService` methods; migrate callers. `interface/http/auth/handlers.rs` (`/auth/me`) fills `linked_mxids` from `identity.user_ids(.., Channel::Matrix)` — field name and type unchanged.
- [ ] **Step 4: Run the Gate.**
- [ ] **Step 5: Commit** — `feat(identity): IdentityService resolves every channel under its provisioning policy`

---

### Task 4: Neutral message types, adapter trait, registry, renderers

**Files:**
- Create: `backend/src/interface/channels/{message,adapter,registry}.rs`, `render/mod.rs`, `render/html.rs`, `fake.rs` (`#[cfg(test)]`)
- Modify: `backend/src/interface/channels/mod.rs` (module docs per spec §3.2; `pub mod` lines; keep `pub mod matrix;`)

**Interfaces:**
- Consumes: Task 1.
- Produces:

```rust
// message.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageRef(pub String);               // opaque, adapter-minted

#[derive(Clone, Debug)]
pub struct IncomingMessage {
    pub channel: Channel,
    pub conversation: ConversationAddress,
    pub sender: PlatformIdentity,
    pub message_id: MessageRef,
    pub text: String,
    pub reply_to: Option<MessageRef>,
    pub quoted: Option<crate::domain::chat::QuotedMessage>,
    pub is_direct: bool,
    pub is_mentioned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visibility { #[default] Normal, Private }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentKind { Audio, Image, File }

#[derive(Clone, Debug)]
pub struct Attachment { pub kind: AttachmentKind, pub mime: String, pub filename: String, pub bytes: bytes::Bytes, pub duration_ms: Option<u64> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action { pub label: String, pub command: String }

#[derive(Clone, Debug, Default)]
pub struct Content { pub plain: String, pub html: Option<String>, pub attachments: Vec<Attachment>, pub actions: Vec<Action> }

impl Content {
    /// plain = strip_html(html)
    pub fn html(html: impl Into<String>) -> Self;
    pub fn text(plain: impl Into<String>) -> Self;
}

#[derive(Clone, Debug)]
pub struct Reply { pub content: Content, pub visibility: Visibility }
impl Reply {
    pub fn html(html: impl Into<String>) -> Self;     // Normal
    pub fn private(html: impl Into<String>) -> Self;  // Private
    pub fn with_attachment(self, a: Attachment) -> Self;
    pub fn with_actions(self, actions: Vec<Action>) -> Self;
    pub fn html_body(&self) -> &str;                   // html or plain; for tests and Matrix rendering
}
impl From<String> for Reply { /* Reply::html */ }

#[derive(Clone, Debug)]
pub enum Target { Conversation(ConversationAddress), Direct(PlatformIdentity) }

#[derive(Clone, Debug)]
pub struct OutgoingMessage { pub target: Target, pub content: Content, pub reply_to: Option<MessageRef>, pub visibility: Visibility }
impl OutgoingMessage {
    pub fn to(target: Target, reply: Reply) -> Self;
    pub fn channel(&self) -> Option<Channel>;          // from the target; None for Web
}

// adapter.rs
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("{0} is not connected")] NotConnected(Channel),
    #[error("not configured: {0:?}")] NotConfigured(Option<Channel>),
    #[error("rejected: {0}")] Rejected(String),
    #[error("transient: {0}")] Transient(String),
}

#[derive(Clone, Debug)]
pub struct SentRef { pub message: Option<MessageRef> }

#[async_trait::async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn channel(&self) -> Channel;
    fn is_connected(&self) -> bool;
    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()>;
    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>;
    /// Outbox pre-check (webhook alerts); default allows.
    async fn authorize_delivery(&self, _owner: &AccountId, _to: &ConversationAddress) -> bool { true }
    /// A conversation found by display name (AI outage alert fallback, A4); default none.
    async fn find_conversation_by_name(&self, _name: &str) -> Option<ConversationAddress> { None }
}

// registry.rs
pub struct ChannelRegistry { adapters: HashMap<Channel, Arc<dyn ChannelAdapter>> }
impl ChannelRegistry {
    pub fn new(adapters: Vec<Arc<dyn ChannelAdapter>>) -> anyhow::Result<Self>;   // error on duplicate channel
    pub fn get(&self, channel: Channel) -> Option<&Arc<dyn ChannelAdapter>>;
    pub fn is_connected(&self, channel: Channel) -> bool;
    pub async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>;  // NotConfigured(None) for Web
    pub async fn find_conversation_by_name(&self, channel: Channel, name: &str) -> Option<ConversationAddress>;
    /// Spawns every adapter's run under a child token; returns the join handles for app.rs supervision.
    pub fn spawn_all(&self, cancel: &CancellationToken) -> Vec<(Channel, tokio::task::JoinHandle<anyhow::Result<()>>)>;
}

// render/html.rs
/// Keeps only b i u code pre br p ul ol li a(href http/https/mailto) blockquote; drops other tags, keeps their text; escapes stray text.
pub fn sanitize_subset(html: &str) -> String;
```

`fake.rs` (`#[cfg(test)] pub(crate)`): `FakeAdapter { channel, connected: AtomicBool, sent: Mutex<Vec<OutgoingMessage>> }` implementing `ChannelAdapter` (`run` waits for `cancel`).

- [ ] **Step 1: Write the failing tests**

```rust
// registry.rs tests
#[tokio::test]
async fn send_routes_by_the_target_channel() {
    let m = Arc::new(FakeAdapter::connected(Channel::Matrix));
    let t = Arc::new(FakeAdapter::connected(Channel::Telegram));
    let reg = ChannelRegistry::new(vec![m.clone(), t.clone()]).unwrap();
    reg.send(OutgoingMessage::to(Target::Conversation(ConversationAddress::Telegram(42)), Reply::html("<b>hi</b>"))).await.unwrap();
    assert_eq!(t.sent().len(), 1);
    assert!(m.sent().is_empty());
}

#[tokio::test]
async fn an_unconfigured_channel_or_web_is_not_configured() {
    let reg = ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Matrix))]).unwrap();
    let to_tg = OutgoingMessage::to(Target::Conversation(ConversationAddress::Telegram(1)), Reply::html("x"));
    assert!(matches!(reg.send(to_tg).await, Err(ChannelError::NotConfigured(Some(Channel::Telegram)))));
    let to_web = OutgoingMessage::to(Target::Conversation(ConversationAddress::Web), Reply::html("x"));
    assert!(matches!(reg.send(to_web).await, Err(ChannelError::NotConfigured(None))));
}

#[test]
fn two_adapters_for_one_channel_are_refused() {
    let a = Arc::new(FakeAdapter::connected(Channel::Matrix));
    let b = Arc::new(FakeAdapter::connected(Channel::Matrix));
    assert!(ChannelRegistry::new(vec![a, b]).is_err());
}

#[tokio::test]
async fn spawned_adapters_stop_when_the_token_is_cancelled() {
    let reg = ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Matrix))]).unwrap();
    let cancel = CancellationToken::new();
    let handles = reg.spawn_all(&cancel);
    cancel.cancel();
    for (_, h) in handles { tokio::time::timeout(Duration::from_secs(1), h).await.unwrap().unwrap().unwrap(); }
}

// message.rs tests
#[test]
fn reply_from_html_derives_its_plain_text() {
    let r = Reply::from("<b>Done</b> &amp; dusted".to_string());
    assert_eq!(r.content.plain, "Done & dusted");
    assert_eq!(r.visibility, Visibility::Normal);
    assert_eq!(Reply::private("<i>x</i>").visibility, Visibility::Private);
}

// render/html.rs tests
#[test]
fn the_subset_keeps_formatting_and_drops_everything_else() {
    assert_eq!(sanitize_subset("<b>a</b><script>x</script><span>c</span>"), "<b>a</b>xc");
    assert_eq!(sanitize_subset("<a href=\"javascript:alert(1)\">x</a>"), "x");
    assert_eq!(sanitize_subset("<a href=\"https://e.com\">x</a>"), "<a href=\"https://e.com\">x</a>");
}
```

- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement.** `sanitize_subset` is a small hand-written tag scanner (the inputs are bot-generated HTML whose user values are already `html_escape`d; `ammonia` is not a dependency — do not add one for this). Reuse `crate::utils::strip_html`.
- [ ] **Step 4: Run the Gate.**
- [ ] **Step 5: Commit** — `feat(channels): channel-neutral messages, adapter trait and registry`

---

### Task 5: The agent stops depending on Matrix

**Files:**
- Create: `backend/src/llm/progress.rs`
- Modify: `llm/mod.rs`, `llm/context.rs` (`stage_reporter` → `progress: Option<&'a dyn ProgressSink>`), `llm/stream.rs`, `llm/turn.rs`, `llm/search.rs` (import `crate::domain::chat::QuotedMessage`), `llm/alert.rs` → **moved** to `backend/src/interface/chat/llm_alert.rs`, `interface/channels/matrix/stage.rs` (`impl ProgressSink for StageReporter`), `interface/chat/{llm,ocr/mod,voice/mod}.rs`, `interface/channels/matrix/attachments.rs`, `interface/channels/matrix/command_flow.rs` (records tool targets from the result).

**Interfaces:**
- Produces:

```rust
// llm/progress.rs
#[async_trait::async_trait]
pub trait ProgressSink: Send + Sync {
    async fn report_stage(&self, text: &str);
    async fn report_tool_call(&self, tool_name: &str);
    /// Streaming text so far is appended; the sink throttles.
    async fn push_delta(&self, delta: &str);
    async fn flush(&self);
    /// Adapter message id of the progress message, if any (Matrix: the stage event id).
    fn message_ref(&self) -> Option<String> { None }
}
pub struct NoProgress;   // impl: all no-ops

// llm/context.rs
pub struct AiTurnResult {
    pub conversation_id: String,
    pub message: String,
    pub model_used: String,
    pub tool_calls_executed: Vec<String>,
    /// (tool name, tool result) pairs, in execution order, for an adapter that links replies to records.
    pub tool_results: Vec<(String, String)>,
}
```

Matrix's `StageReporter` keeps its own `StreamThrottler`; its `ProgressSink::push_delta` locks an inner `Mutex<StreamThrottler>` (the code `stream.rs` builds today moves into the Matrix impl). `llm/stream.rs` takes `Option<&dyn ProgressSink>` and calls `push_delta`/`flush`.

`record_ai_tool_target` moves out of `llm/turn.rs`: Matrix's command flow, after the turn, calls it for each `tool_results` entry with the stage event id it already owns — same arguments as today, so `bot_message_targets` rows are identical. Where the turn result does not reach the Matrix flow (the result is consumed inside `interface/chat/llm.rs`), thread `tool_results` out through the module's `Reply`: add `pub tool_results: Vec<(String, String)>` to `Reply` (default empty) — the Matrix adapter records targets from it in Task 7; for this task, record them in `interface/chat/llm.rs` right where the turn returns, using `ctx.progress.and_then(|p| p.message_ref())` as the event id (identical to today's `reporter.event_id()`).

The outage alert (`interface/chat/llm_alert.rs`) keeps `AI_ERROR_ALERTED`, `clear_ai_error_state`, `is_ai_error_alerted`, the plain/HTML wording, and still uses the Matrix client in this task (it is now interface code, so that is legal); Task 7 switches it to the registry (A4).

- [ ] **Step 1: Write the failing tests**

```rust
// llm/progress.rs
#[tokio::test]
async fn no_progress_accepts_everything_silently() {
    let p = NoProgress;
    p.report_stage("x").await; p.report_tool_call("todo_add").await; p.push_delta("a").await; p.flush().await;
    assert_eq!(p.message_ref(), None);
}

// llm/turn.rs (existing test module): a turn driven by a recording sink
#[tokio::test]
async fn a_turn_reports_tool_stages_through_any_progress_sink() {
    // Use the existing fake LLM transport used by llm/turn.rs tests (grep "fn fake_llm" / mockito in llm/);
    // script one tool call then a final answer; pass a RecordingSink; assert it saw report_tool_call(<tool>)
    // and report_stage("🧠 正在组织回复..."), and that AiTurnResult.tool_results has one entry.
}
```

Write the second test concretely against the existing LLM test harness in `llm/` (find it with `grep -rn "#\[tokio::test\]" backend/src/llm/turn.rs backend/src/llm/tools/mod.rs`); if `run_turn` has no transport seam, test `execute_stream_or_complete` with a `RecordingSink` against the existing `LlmClient` test double instead, and cover the turn-level reporting in Task 7's pipeline test.

Architecture test (add now, in `backend/tests/architecture.rs`):

```rust
#[test]
fn the_agent_knows_no_channel() {
    let hits = production_violations("llm", &["matrix_sdk", "interface::channels", "serenity", "teloxide", "lettre", "async_imap", "mail_parser"]);
    assert!(hits.is_empty(), "llm/ must not know any channel:\n{}", hits.join("\n"));
}
```

- [ ] **Step 2: Run** `cargo test -p matrix-bot --test architecture the_agent_knows_no_channel` → FAIL listing `alert.rs`, `context.rs`, `stream.rs`, `search.rs`, `turn.rs`.
- [ ] **Step 3: Implement** as described.
- [ ] **Step 4: Run the Gate** (the architecture test now passes; every existing LLM and Matrix test passes).
- [ ] **Step 5: Commit** — `refactor(llm): progress sink replaces the Matrix stage reporter`

---

### Task 6: Channel-neutral `CommandContext` and `Reply`

**Files:**
- Modify: `backend/src/interface/chat/mod.rs`, every `BotModule` under `backend/src/interface/chat/**`, `interface/channels/matrix/{command_flow,dispatcher,attachments,reaction}.rs`, `interface/chat/tts/mod.rs`, `interface/chat/account.rs`.

**Interfaces:**
- Consumes: Tasks 1, 4, 5.
- Produces:

```rust
#[derive(Clone)]
pub struct ParsedCommand {
    pub conversation: ConversationAddress,
    pub sender: PlatformIdentity,
    pub command: String,
    pub raw_args: String,
    pub full_body: String,
    pub is_mentioned: bool,
    pub quoted: Option<QuotedMessage>,
}
impl ParsedCommand { pub fn for_owner(self, owner_id: AccountId, is_direct: bool) -> CommandContext; }

#[derive(Clone)]
pub struct CommandContext {
    pub conversation: ConversationAddress,
    pub owner_id: AccountId,
    pub sender: PlatformIdentity,
    pub command: String,
    pub raw_args: String,
    pub full_body: String,
    pub is_direct: bool,
    pub is_mentioned: bool,
    pub quoted: Option<QuotedMessage>,
    pub progress: Option<Arc<dyn ProgressSink>>,
    /// Out-of-band sends (account announcements, outage alert). `None` in unit tests.
    pub outbound: Option<Weak<ChannelRegistry>>,
}
impl CommandContext {
    pub fn channel(&self) -> Channel { self.sender.channel() }
    /// Storage-form conversation, for the existing `room_id` columns.
    pub fn room_key(&self) -> String { self.conversation.to_storage() }
    /// The sender's MXID when the command came over Matrix.
    pub fn sender_mxid(&self) -> Option<&str>;
    /// Display name for "who typed this" (never an ownership key).
    pub fn sender_name(&self) -> &str;
}

#[async_trait]
pub trait BotModule: Send + Sync {
    fn name(&self) -> &str;
    fn commands(&self) -> Vec<CommandHelp>;
    async fn handle_command(&self, ctx: &CommandContext, services: &AppServices, db: &Database) -> Option<Reply>;
}

pub fn error_reply(err: AppError) -> Reply;   // same HTML as today
```

Rules for the mechanical sweep:
- `ctx.room_id` → `ctx.room_key()` (identical value for Matrix: the bare room id).
- `ctx.sender_mxid` → `ctx.sender_mxid().unwrap_or_default()` only in the two account commands that act on a Matrix account; anywhere else it was provenance, use `ctx.sender.user_id().as_str()`.
- `ctx.sender_name` → `ctx.sender_name()`.
- `ctx.stage_reporter` → `ctx.progress.as_deref()`.
- `Some(html)` → `Some(Reply::from(html))`; replies carrying private data use `Reply::private(html)`: every reply of `FinanceModule`, `PortfolioModule`, `JournalModule`, `LifeModule`, `SettingsModule`, `AccountModule`, and `!ai` replies in a non-direct conversation are **Normal** (unchanged Matrix behaviour; Matrix renders both identically).
- `TtsModule` no longer holds a `matrix_sdk::Client`: it returns `Reply::html(caption).with_attachment(Attachment { kind: Audio, mime: "audio/ogg", filename, bytes, duration_ms })`; the Matrix adapter uploads it as an MSC3245 voice message with today's `AttachmentInfo::Voice` (move that code from `tts/mod.rs` into `interface/channels/matrix/outbound.rs`; until Task 7 creates that file, put it in `interface/channels/matrix/voice_upload.rs` and call it from `command_flow.rs` when a reply carries an audio attachment).
- `AccountModule` and `LlmModule` keep their `Arc<RwLock<Option<Client>>>` handles **in this task** (for `announce_account` and the outage alert). Task 7, which introduces the registry, removes both handles and sends through `ctx.outbound`.

- [ ] **Step 1: Update the existing dispatch tests first** (`interface/chat/mod.rs` tests): build contexts with `ConversationAddress::Matrix("!room:a".into())` and `PlatformIdentity::matrix("@tester:x")`; compare `reply.html_body()`; they fail to compile until Step 3.
- [ ] **Step 2: Add a new test**

```rust
#[test]
fn a_matrix_context_exposes_the_same_room_key_and_mxid_as_before() {
    let ctx = ctx_for("todo");
    assert_eq!(ctx.room_key(), "!room:a");
    assert_eq!(ctx.sender_mxid(), Some("@tester:x"));
    assert_eq!(ctx.channel(), Channel::Matrix);
}

#[test]
fn error_replies_keep_their_wording() {
    assert_eq!(error_reply(AppError::Validation("<b>too long</b>".into())).html_body(), "⚠️ &lt;b&gt;too long&lt;/b&gt;");
}
```

- [ ] **Step 3: Implement the sweep.** `cargo check -p matrix-bot --all-targets` drives it.
- [ ] **Step 4: Architecture test** (add):

```rust
#[test]
fn chat_modules_know_no_platform_sdk() {
    let hits = production_violations("interface/chat", &["matrix_sdk", "serenity", "teloxide", "lettre", "async_imap"]);
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}
```

`account.rs` and `llm.rs` still import `matrix_sdk` until Task 7, so add this test with `#[ignore = "enabled in Task 7"]`; Task 7 removes the attribute.

- [ ] **Step 5: Run the Gate.**
- [ ] **Step 6: Commit** — `refactor(chat): channel-neutral command context and replies`

---

### Task 7: Inbound pipeline and `MatrixAdapter`; registry in the composition root

**Files:**
- Create: `backend/src/interface/channels/inbound.rs`, `interface/channels/matrix/{adapter,inbound,outbound}.rs`
- Delete: `interface/channels/matrix/command_flow.rs` (logic moves to `matrix/inbound.rs` + `channels/inbound.rs`), `interface/channels/matrix/delivery.rs` (Task 8 removes its last user — keep it until then if needed), `interface/chat/llm_alert.rs`'s Matrix client use
- Modify: `interface/channels/matrix/{mod,handler,handles,notify,voice_upload}.rs`, `interface/chat/{account,llm}.rs`, `backend/src/app.rs`

**Interfaces:**
- Consumes: Tasks 3–6.
- Produces:

```rust
// channels/inbound.rs
pub struct Inbound {
    services: AppServices,
    db: Database,
    modules: Arc<ModuleRegistry>,
    registry: Weak<ChannelRegistry>,
}
pub enum InboundText { Command { command: String, raw_args: String, full_body: String }, FreeText(String) }
pub enum InboundOutcome { Replied(Reply), Ignored }
impl Inbound {
    pub fn new(services: AppServices, db: Database, modules: Arc<ModuleRegistry>, registry: Weak<ChannelRegistry>) -> Self;
    /// Spec §5 steps 2–4. The adapter parsed `text` (step 1) and sends the reply (step 5) so it can attach
    /// its own progress UI; `progress` is that adapter's sink.
    pub async fn handle(&self, msg: &IncomingMessage, text: InboundText, progress: Option<Arc<dyn ProgressSink>>) -> InboundOutcome;
}
pub const UNLINKED_REPLY: &str = "This account is not linked yet. Send <code>/link</code> in a direct message to connect it to your workspace.";

// channels/matrix/adapter.rs
pub struct MatrixAdapter { /* config, services, db, handles: BotHandles, inbound: OnceLock<Arc<Inbound>> */ }
impl MatrixAdapter {
    pub fn new(config: Config, services: AppServices, db: Database, handles: BotHandles) -> Self;
    pub fn attach_inbound(&self, inbound: Arc<Inbound>);   // called once from app.rs inside Arc::new_cyclic
}
// impl ChannelAdapter for MatrixAdapter:
//   channel = Matrix; is_connected = handles.matrix.current().is_some();
//   run = today's run_bot_loop (login, ensure_self_verified, register handlers, sync) until cancel;
//   send = matrix/outbound.rs; authorize_delivery = rooms::is_room_authorized_for_account (today's MatrixRoomSink check);
//   find_conversation_by_name = today's joined-room scan from llm/alert.rs.
```

Pipeline behaviour (`Inbound::handle`), exactly:
1. `text` is `Command` with `command.eq_ignore_ascii_case("link")` → Task 9 hook; **in this task** keep today's `!link <code>` behaviour by calling the existing `interface::channels::matrix::linking::handle_link_command` when `msg.channel == Matrix` (it is deleted in Task 9). Return before resolving.
2. `services.identity.resolve(&msg.sender)`. `Unlinked` has no account, so nothing is dispatched: `start` and `help` get `UNLINKED_REPLY` followed by the line "Commands available once linked: todo, expense, portfolio, ask."; every other command and free text in a direct conversation gets `UNLINKED_REPLY`; free text in a shared conversation is `Ignored`. A resolution error → log, `Ignored` (today's behaviour on Matrix).
3. Build `CommandContext` (`FreeText` in a direct conversation or with a mention becomes command `ai` with the text as `raw_args`/`full_body` — move today's `CommandParser::parse_or_dm_prompt` rule here so every channel shares it; Matrix's `dispatcher.rs` keeps only Matrix syntax: `!`, mentions, localpart stripping, quote extraction).
4. `modules.dispatch` wrapped in `catch_unwind` (today's panic reply text unchanged); `None` → `Ignored`.

`matrix/inbound.rs` does today's `command_flow::handle_text` work up to the parse (quote resolution, typing guard, stage indicator), converts to `IncomingMessage` (`message_id = MessageRef(event_id)`, `conversation = Matrix(room_id)`, `sender = PlatformIdentity::matrix(sender)` with display name = localpart, `is_direct` as computed today), calls `Inbound::handle`, then renders the reply exactly as today (stage indicator `finish_with_reply`, `send_html`, voice upload for audio attachments, `try_record_from_reply`, `link_turn_targets`, tool-target recording from `Reply.tool_results`) and the same error/`None` handling.

`app.rs`:

```rust
let modules = Arc::new(crate::interface::channels::matrix::build_modules(&config, &handles)); // the registration list from MatrixBot::new_full, unchanged order
let matrix = Arc::new(MatrixAdapter::new(config.clone(), services.clone(), db.clone(), handles.clone()));
let channels = Arc::new_cyclic(|weak| {
    matrix.attach_inbound(Arc::new(Inbound::new(services.clone(), db.clone(), modules.clone(), weak.clone())));
    ChannelRegistry::new(vec![matrix.clone() as Arc<dyn ChannelAdapter>]).expect("one adapter per channel")
});
```

`ChannelRegistry::new` returns `Result`; inside `new_cyclic` an `expect` is acceptable only because the vector is built two lines above with one adapter per channel — instead, check uniqueness before `new_cyclic` with a `ChannelRegistry::check_unique(&[Channel])` helper and keep `new` infallible inside the closure (no `expect` on a runtime path). The supervised `matrix` task in `app::run` becomes the registry's spawned Matrix handle; keep the existing supervision tests green (rename the variable, not the semantics).

`ModuleRegistry` construction moves from `MatrixBot::new_full` to `interface/channels/matrix/modules.rs::build_modules` (TTS and LLM modules no longer need the client). `AccountModule`'s `announce_account` sends `OutgoingMessage { target: Direct(matrix identity) }` per linked MXID through `ctx.outbound`; the outage alert (`llm_alert.rs`) sends through `ctx.outbound` with the A4 target rule.

- [ ] **Step 1: Write the failing pipeline tests** (`channels/inbound.rs`, real test DB, `FakeAdapter`)

```rust
#[tokio::test]
async fn an_unlinked_telegram_user_reaches_no_module() {
    let (inbound, _reg) = pipeline_with(StubModule::claiming("todo")).await;
    let msg = incoming(Channel::Telegram, ConversationAddress::Telegram(42), tg("987654321"), true);
    let out = inbound.handle(&msg, cmd("todo", "add milk"), None).await;
    assert_reply_contains(out, "not linked yet");
}

#[tokio::test]
async fn a_linked_telegram_user_reaches_the_same_module_as_matrix() {
    let (inbound, pool) = pipeline_with(StubModule::claiming("todo")).await;
    let a = create_account(&pool).await;
    identities::bind(&pool, &a, &tg("987654321")).await.unwrap();
    let out = inbound.handle(&incoming(Channel::Telegram, ConversationAddress::Telegram(42), tg("987654321"), true), cmd("todo", "list"), None).await;
    assert_reply_contains(out, "handled by todo");
}

#[tokio::test]
async fn free_text_in_a_direct_conversation_goes_to_the_ai_command() {
    let (inbound, pool) = pipeline_with(StubModule::claiming("ai")).await;
    let a = create_account(&pool).await;
    identities::bind(&pool, &a, &tg("1")).await.unwrap();
    let out = inbound.handle(&incoming(Channel::Telegram, ConversationAddress::Telegram(1), tg("1"), true), InboundText::FreeText("hello".into()), None).await;
    assert_reply_contains(out, "handled by ai");
}

#[tokio::test]
async fn free_text_in_a_group_without_a_mention_is_ignored() { /* is_direct=false, is_mentioned=false → Ignored */ }

#[tokio::test]
async fn a_matrix_stranger_is_provisioned_and_dispatched() { /* mx("@new:x"), cmd("todo","list") → handled */ }

#[tokio::test]
async fn a_panicking_module_gets_the_fixed_apology() { /* PanickingModule → reply equals today's err_html */ }
```

and a Matrix conversion test in `matrix/inbound.rs` (pure): build an `OriginalSyncRoomMessageEvent` from JSON (`serde_json::from_value` of a fixture under `contracts/channels/matrix_text_event.json`) and assert the produced `IncomingMessage` fields.

- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement.** Keep `handler.rs` registering the same three handlers; the text branch calls `matrix::inbound::handle_text(&ev, &room, &adapter_inbound, ...)`.
- [ ] **Step 4: Remove the `#[ignore]` from `chat_modules_know_no_platform_sdk`.** Add:

```rust
#[test]
fn only_the_composition_root_builds_adapters_and_the_registry() {
    for needle in ["MatrixAdapter::new(", "ChannelRegistry::new("] {
        let hits: Vec<_> = rust_files(&src("")).into_iter()
            .filter(|f| !relative_to_src(f).starts_with("app.rs") && !relative_to_src(f).starts_with("interface/channels/registry.rs"))
            .flat_map(|f| production_hits(&std::fs::read_to_string(&f).unwrap(), &[needle]).into_iter().map(move |(n, l)| format!("{}:{n}: {l}", relative_to_src(&f))))
            .collect();
        assert!(hits.is_empty(), "{needle} outside app.rs:\n{}", hits.join("\n"));
    }
}

#[test]
fn channel_adapters_run_no_sql() {
    let hits = production_violations("interface/channels", &["sqlx::", "query!", "query_as!", "query_scalar!", "QueryBuilder"]);
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}
```

If `channel_adapters_run_no_sql` finds the pre-existing SQL in `matrix/reaction_targets.rs` or `matrix/invite.rs`, move those queries into `repositories::reaction_targets` / `IdentityService` (they belong there under AGENTS §6) as part of this task.

- [ ] **Step 5: Run the Gate.** All Matrix tests (dispatch order, TTS-before-catch-all, quotes, reactions, attachments) pass.
- [ ] **Step 6: Commit** — `refactor(matrix): MatrixAdapter behind the channel registry and shared inbound pipeline`

---

### Task 8: Outbound through the registry (jobs and outbox)

**Files:**
- Modify: `backend/src/interface/jobs/{mod,fake,reminders,briefing,portfolio}.rs`, `backend/src/domain/events.rs` (`Destination`), `backend/src/application/webhooks/mod.rs` (where it builds destinations), `backend/src/app.rs`
- Create: `backend/src/interface/events/channel_sink.rs`
- Delete: `backend/src/interface/events/matrix_room.rs`, `backend/src/interface/channels/matrix/delivery.rs`

**Interfaces:**
- Produces:

```rust
// interface/jobs/mod.rs
#[async_trait]
pub trait Outbound: Send + Sync {
    fn is_connected(&self, channel: Channel) -> bool;
    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>;
}
impl Outbound for ChannelRegistry { /* delegate */ }
pub fn handlers(services: &AppServices, outbound: Arc<dyn Outbound>) -> Vec<Arc<dyn JobHandler>>;
/// Renamed from MATRIX_RETRY; same 30 s.
pub const CHANNEL_RETRY: Duration = Duration::from_secs(30);

// domain/events.rs
pub enum Destination { Conversation(ConversationAddress) }   // as_key = to_destination(); parse = parse_destination()
```

Handler rules:
- Today's `send_to_room(room_id, …)`: parse the job's stored room string with `ConversationAddress::parse_storage`. `Err` or `Web` → log at `warn` and treat as delivered (today's `Ok(None)` semantics). Otherwise send `Target::Conversation(addr)`; `NotConnected`/`NotConfigured(Some(_))` → defer with `CHANNEL_RETRY` (today's behaviour when Matrix is down); `Transient` → retryable job error; `Rejected` → permanent job error.
- Today's `is_connected()` checks become `is_connected(addr.channel())`.
- Today's `send_dm(mxid, …)` (portfolio alerts): for each of the owner's identities, prefer the channel of the alert's originating conversation when the alert carries one, else Matrix: `Target::Direct(identity)`; frozen wording.
- `ChannelSink` (`DeliverySink`): parse `Destination`, load the alert payload as `MatrixRoomSink` does today, call `adapter.authorize_delivery(owner, &addr)` (Matrix: today's room-authorization check — frozen error text), send. Same transaction id dedupe for Matrix: the Matrix adapter's `send` accepts an optional idempotency key — add `pub idempotency_key: Option<String>` to `OutgoingMessage` (default `None`); Matrix maps it to the transaction id (`outbox-<delivery id>`); other adapters ignore it until they need it.
- Existing `outbox_deliveries` rows with `matrix:<room>` keys parse unchanged (test it).

- [ ] **Step 1: Write the failing tests**

```rust
// domain/events.rs
#[test]
fn legacy_matrix_destination_keys_still_parse() {
    let d = Destination::parse("matrix:!abc:mbm.one").unwrap();
    assert_eq!(d, Destination::Conversation(ConversationAddress::Matrix("!abc:mbm.one".into())));
    assert_eq!(d.as_key(), "matrix:!abc:mbm.one");
}

// interface/jobs/reminders.rs (existing tests use fake::FakeDelivery → rename FakeOutbound)
#[tokio::test]
async fn a_reminder_set_on_telegram_fires_on_telegram() {
    // create a reminder whose room_id is "telegram:42"; run the fire handler with FakeOutbound;
    // assert one send with Target::Conversation(Telegram(42)).
}

#[tokio::test]
async fn a_web_reminder_is_delivered_nowhere_and_succeeds() { /* room "web" → no send, job Ok */ }

#[tokio::test]
async fn a_disconnected_channel_defers_the_job() { /* FakeOutbound disconnected → JobError::Retry(CHANNEL_RETRY) as today */ }
```

Adapt every existing jobs test from `FakeDelivery` to `FakeOutbound` with the same assertions on content.

- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement**; wire `app.rs`: `jobs::Registry::new(handlers(&services, channels.clone() as Arc<dyn Outbound>))`, `DeliverySink = ChannelSink::new(channels.clone(), services.clone())`. Add `ConversationAddress`/`Destination` serde to any job payload that stores a destination only if needed (payloads keep their current string fields — no payload format change).
- [ ] **Step 4: Run the Gate** (OpenAPI unchanged: `Destination` is not an API type — verify).
- [ ] **Step 5: Commit** — `refactor(delivery): jobs and outbox deliver through the channel registry`

---

### Task 9: Chat-initiated linking (`LinkService`) — old flow deleted

**Files:**
- Create: `backend/src/domain/channels/link.rs`, `backend/src/infrastructure/postgres/repositories/link_tokens.rs`, `backend/src/application/identity/link.rs`, `backend/src/interface/chat/link.rs`, `backend/migrations/0008_drop_matrix_link_challenges.sql`
- Delete: `backend/src/interface/channels/matrix/linking.rs`, `repositories/accounts/linking.rs` (its `issue`/`consume`/`claim`), `AccountService::{issue_link_challenge, claim_link}`, `PendingMerges` (`handles.rs`, `ApiState.pending_merges`, `interface/http/auth/types.rs`), the `"matrix_link_challenges"` entry of `ACCOUNT_ID_DISCARDED`
- Modify: `domain/events.rs` (new `SystemEvent::IdentityLinked`), `interface/channels/inbound.rs` (step 1 calls the new handler), `application/mod.rs` (`AppServices.link: LinkService`), `config.rs` (`WEB_URL` already exists; add `pub link_signup_allowlist: HashMap<Channel, Vec<String>>` read from `DISCORD_ALLOWED_USERS`, `TELEGRAM_ALLOWED_USERS`, `EMAIL_ALLOWED_SENDERS` — comma-separated; empty when unset)

**OpenAPI exception.** The handlers behind `/api/v1/account/matrix/{link,pending-merge,confirm-merge}` depend on the deleted code, so this task deletes those three handlers and their routes and `SENSITIVE` entries. That changes OpenAPI; regeneration is deferred to Task 10, so **skip the Gate's OpenAPI line for Task 9 only** and say "OpenAPI regenerated in the next commit" in the commit body. Task 10 must follow immediately. `/account/matrix` (list) and `/account/matrix/unlink` stay until Task 10, re-pointed at `IdentityService` in Task 3.

**Interfaces:**
- Produces:

```rust
// domain/channels/link.rs
pub const LINK_TOKEN_TTL: chrono::Duration = chrono::Duration::minutes(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDecision { Bind, AlreadyYours, Merge { from: AccountId }, OwnedElsewhere }

/// `current` = account the identity maps to now (if any) and whether it is unclaimed.
pub fn decide_link(viewer: &AccountId, current: Option<(&AccountId, bool)>) -> LinkDecision;

// repositories/link_tokens.rs
pub struct LinkTokenRow { pub identity: PlatformIdentity, pub conversation: String, pub expires_at: DateTime<Utc>, pub consumed_at: Option<DateTime<Utc>> }
pub async fn insert(conn: &mut PgConnection, token_hash: &str, identity: &PlatformIdentity, conversation: &str, now: DateTime<Utc>) -> Result<(), sqlx::Error>;
pub async fn consume_live_for_identity(conn: &mut PgConnection, identity: &PlatformIdentity, now: DateTime<Utc>) -> Result<u64, sqlx::Error>;
pub async fn find(conn: impl PgExecutor<'_>, token_hash: &str) -> Result<Option<LinkTokenRow>, sqlx::Error>;
pub async fn lock(conn: &mut PgConnection, token_hash: &str) -> Result<Option<LinkTokenRow>, sqlx::Error>;   // SELECT … FOR UPDATE
pub async fn consume(conn: &mut PgConnection, token_hash: &str, now: DateTime<Utc>) -> Result<(), sqlx::Error>;

// application/identity/link.rs
pub struct LinkStart { pub token: String, pub url: String }
pub enum LinkPreview { Bind { identity: LinkedIdentity }, AlreadyYours, NeedsMerge { identity: LinkedIdentity, preview: MergePreview }, OwnedElsewhere, SignupAllowed { identity: LinkedIdentity }, SignInRequired { identity: LinkedIdentity }, Expired }
pub enum LinkOutcome { Linked { account: AccountId }, AlreadyYours, Merged { account: AccountId, from: AccountId }, OwnedElsewhere, Expired }

#[derive(Clone)]
pub struct LinkService { pool: PgPool, web_url: String, signup_allowlist: Arc<HashMap<Channel, Vec<String>>> }
impl LinkService {
    pub fn new(pool: PgPool, web_url: String, signup_allowlist: HashMap<Channel, Vec<String>>) -> Self;
    pub async fn start(&self, identity: &PlatformIdentity, conversation: &ConversationAddress, is_direct: bool) -> Result<LinkStart, AppError>;   // Validation if !is_direct
    pub async fn inspect(&self, token: &str, viewer: Option<&AccountId>) -> Result<LinkPreview, AppError>;
    pub async fn claim(&self, token: &str, account: &AccountId) -> Result<LinkOutcome, AppError>;
    pub fn signup_allowed(&self, identity: &PlatformIdentity) -> bool;
    /// Used by the signup finish handler inside the passkey-registration transaction.
    pub async fn claim_for_new_account(&self, conn: &mut PgConnection, token: &str, account: &AccountId) -> Result<LinkOutcome, AppError>;
}

// domain/events.rs
SystemEvent::IdentityLinked { owner_id: AccountId, channel: Channel, user_id: String, conversation: String }
// owner_id() → owner_id; the claim publishes it with destination Destination::Conversation(parse_storage(conversation))
// (skipped when the conversation is Web or unparseable). ChannelSink renders it as
// "✓ <Channel> account linked. Your workspace is now available here." (HTML-escaped channel name).
```

`claim` transaction: `lock` the token → missing/consumed/expired → `Expired` (and no write); `decide_link(account, current)`: `Bind` → `identities::bind`; `AlreadyYours` → nothing; `Merge { from }` → `merge_within(tx, from → account)` then `identities::rebind`; `OwnedElsewhere` → nothing. Then `consume`, write the security event, publish `IdentityLinked` (not for `OwnedElsewhere`/`Expired`), commit. A3's "previously unlinked with data" check runs in `inspect` and turns `Bind` into `NeedsMerge` exactly as `linking::claim` does today.

Chat `link` handler (`interface/chat/link.rs`, called by the pipeline before resolution): not direct → "That would be visible to everyone here. Send <code>/link</code> to me in a direct message." (no token minted); else `start` → "Open this link within 10 minutes to connect this <Channel> account to your workspace:<br/><a href=\"{url}\">{url}</a>". The URL is `{web_url}/link?token={token}`. The token is never logged.

- [ ] **Step 1: Write the failing tests**

Domain (`link.rs`):

```rust
#[test]
fn link_decisions() {
    let me = AccountId::from_seed("me"); let other = AccountId::from_seed("other");
    assert_eq!(decide_link(&me, None), LinkDecision::Bind);
    assert_eq!(decide_link(&me, Some((&me, false))), LinkDecision::AlreadyYours);
    assert_eq!(decide_link(&me, Some((&other, true))), LinkDecision::Merge { from: other.clone() });
    assert_eq!(decide_link(&me, Some((&other, false))), LinkDecision::OwnedElsewhere);
}
```

Service (`application/identity/link.rs`, test DB):

```rust
#[tokio::test] async fn start_refuses_a_shared_conversation_and_mints_nothing() { /* Err(Validation); link_tokens empty */ }
#[tokio::test] async fn a_token_is_stored_only_as_its_hash() { /* start; row token_hash == sha256(token); no row equals token */ }
#[tokio::test] async fn starting_again_consumes_the_previous_token() { /* t1, t2 = start twice; inspect(t1) == Expired */ }
#[tokio::test] async fn claim_binds_a_new_telegram_identity_and_publishes_identity_linked() {
    /* start(tg) → claim(token, a) == Linked{a}; identity.resolve(tg) == Account(a);
       one outbox event IdentityLinked with destination "telegram:42" */ }
#[tokio::test] async fn a_token_works_once() { /* claim twice → second Expired */ }
#[tokio::test] async fn an_expired_token_binds_nothing() { /* insert row with expires_at in the past via repository; claim → Expired */ }
#[tokio::test] async fn an_auto_provisioned_matrix_account_is_merged_into_the_claimer() {
    /* resolve(mx) provisions B with a todo; start(mx); claim(token, A) == Merged{A, B}; the todo now belongs to A */ }
#[tokio::test] async fn an_identity_owned_by_a_real_account_is_not_stolen() { /* bind tg to active C; claim by A → OwnedElsewhere; still C */ }
#[tokio::test] async fn a_failed_claim_leaves_the_token_usable() {
    /* force the publish to fail as the outbox atomicity tests do (see commit 2d18d1b, grep "force" in application/outbox tests);
       claim → Err; inspect(token) is not Expired; identity unbound */ }
#[tokio::test] async fn signup_is_allowed_only_for_allowlisted_identities() { /* allowlist {Telegram: ["1"]}: tg("1") true, tg("2") false, mx(...) false */ }
```

Chat (`interface/chat/link.rs`): a group-conversation `link` reply contains "direct message" and `link_tokens` is empty; a direct one contains `/link?token=`.

- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement**; migration `0008_drop_matrix_link_challenges.sql`: `DROP TABLE matrix_link_challenges;`. Delete the old code paths listed above and their tests (their invariants are re-proven by the new tests). Regenerate `.sqlx/`.
- [ ] **Step 4: Run the Gate** (skip the OpenAPI line; see above).
- [ ] **Step 5: Commit** — `feat(identity)!: chat-initiated account linking for every channel`, body: "Replaces the web-minted `!link <code>` flow. OpenAPI and the web UI follow in the next commits."

---

### Task 10: Identity and link HTTP API (intentional OpenAPI change)

**Files:**
- Create: `backend/src/interface/http/identities.rs`, `backend/src/interface/http/link.rs`
- Delete: `backend/src/interface/http/account/links.rs` (and its routes)
- Modify: `interface/http/routes.rs`, `interface/http/openapi.rs`, `interface/http/mod.rs` (`SENSITIVE`), `interface/http/account/audit.rs` (if it read `pending_merges`), `interface/http/rate_limit.rs` (a limiter for signup), `interface/http/auth/webauthn.rs` (reuse registration ceremony for signup), `backend/openapi.json`, `frontend/src/api/*` (regenerated), `frontend/e2e/fake-api.ts`

**Interfaces:**
- Routes (DTOs in each file, `From<&domain>` conversions, `ApiError` everywhere):

| Method | Path | Extractor | Request | Response |
|---|---|---|---|---|
| GET | `/api/v1/account/identities` | `AuthUser` | — | `IdentityListResponse { identities: Vec<IdentityDto { channel: String, user_id: String, display_name: Option<String>, linked_at: DateTime<Utc> }> }` |
| POST | `/api/v1/account/identities/unlink` | `VerifiedUser` | `UnlinkIdentityRequest { channel: String, user_id: String }` | `UnlinkIdentityResponse { unlinked: bool }` |
| POST | `/api/v1/link/inspect` | `Option<AuthUser>` | `LinkTokenRequest { token: String }` | `LinkPreviewDto { state: "bind"|"already_yours"|"needs_merge"|"owned_elsewhere"|"signup_allowed"|"sign_in_required"|"expired", channel: Option<String>, user_id: Option<String>, merge_item_count: Option<i64> }` |
| POST | `/api/v1/link/claim` | `VerifiedUser` | `LinkTokenRequest` | `LinkClaimResponse { outcome: "linked"|"already_yours"|"merged"|"owned_elsewhere"|"expired" }` |
| POST | `/api/v1/link/signup/start` | none, rate-limited | `LinkTokenRequest` | same shape as `/auth/passkey/register/start` |
| POST | `/api/v1/link/signup/finish` | none, rate-limited | `{ token, credential }` | same as `/auth/passkey/register/finish` (sets the session cookie) |

`signup/start` refuses (`403`) unless `inspect(token, None)` is `SignupAllowed`. `signup/finish` creates the account, stores the credential and calls `LinkService::claim_for_new_account` in **one** transaction (follow how `/auth/passkey/register/finish` creates accounts today; extract the shared part into the passkeys application service rather than duplicating it). All string fields capped at 500 chars. The `token` never appears in logs or error messages.

- Add `("POST", "/api/v1/link/claim")` and `("POST", "/api/v1/account/identities/unlink")` to `SENSITIVE`; remove `("POST", "/api/v1/account/matrix/unlink")` (Task 9 already removed the link and confirm-merge entries).

- [ ] **Step 1: Write the failing HTTP tests** (follow `interface/http/account/mod.rs` tests' `app_with_session` style):
  - `GET /account/identities` lists Matrix and Telegram identities of the caller only.
  - `POST /link/claim` with a stale session → 403 (the `SENSITIVE` loop covers it; add the entry first and watch it fail).
  - `POST /link/claim` with a fresh session binds and returns `linked`.
  - `POST /link/inspect` without a session for a non-allowlisted Telegram identity → `sign_in_required`; allowlisted → `signup_allowed`.
  - `POST /link/signup/start` for a non-allowlisted identity → 403.
  - A missing `Origin` on a cookie-authenticated `POST /link/claim` is refused (existing CSRF layer; assert it applies).
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement**; then `cargo run --bin generate-openapi`, `npm --prefix frontend run generate-api`; update `frontend/e2e/fake-api.ts` routes (identities list, link inspect/claim) so existing e2e specs keep passing; the Security panel switch happens in Task 11 — in this task only make the frontend compile against the regenerated client (the panel's Matrix-link section calls the new list endpoint and shows Matrix entries as before).
- [ ] **Step 4: Run the Gate** including `npm --prefix frontend run build` and `npm --prefix frontend run test:e2e`; commit the regenerated `backend/openapi.json`.
- [ ] **Step 5: Commit** — `feat(api)!: identity and link endpoints replace /account/matrix/*`

---

### Task 11: Web link page and identity list

**Files:**
- Create: `frontend/src/modules/link/LinkPage.tsx`, `frontend/e2e/link.spec.ts`
- Modify: `frontend/src/App.tsx` (route `/link`, reachable signed-out), `frontend/src/components/SecurityPanel.tsx` (list every channel's identities with an Unlink button that runs the existing step-up flow), `frontend/e2e/fake-api.ts`

Behaviour (follow `Design.md`: monochrome, Geist, sentence case, `space-y-6` root, visible labels/focus, `Escape`/`Enter`):
- Read `token` from the query string; `POST /link/inspect`; render by `state`:
  - `bind` → "Connect <Channel> account <user_id> to your workspace?" [Connect] → `POST /link/claim` (step-up re-auth on 403 using the existing helper in `core/apiClient.ts`).
  - `needs_merge` → the existing merge-confirmation copy with `merge_item_count`, [Merge and connect].
  - `sign_in_required` → [Sign in with a passkey] (existing login flow, then re-inspect).
  - `signup_allowed` → [Create an account with a passkey] (`/link/signup/start|finish`) and a secondary [I already have an account].
  - `already_yours` / `owned_elsewhere` / `expired` → explanatory text; expired says "Send /link to the bot again."
  - After success: "Connected. You can close this tab." (the bot confirms in chat via the outbox).
- Remove the token from the address bar after reading it (`history.replaceState`).

- [ ] **Step 1: Write the Playwright spec** (`link.spec.ts`) covering: bind happy path; expired token; signup-allowed path renders the create button; needs-merge shows the item count. Run `npm --prefix frontend run test:e2e -- link.spec.ts` → FAIL.
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Run** the frontend build and full e2e suite → PASS; then the Gate.
- [ ] **Step 4: Commit** — `feat(web): link landing page and multi-channel identity list`

---

### Task 12: Boundary tests, docs, hand-off

**Files:**
- Modify: `backend/tests/architecture.rs`, `docs/architecture.md`, `docs/database.md`, `docs/api.md`, `docs/llm.md`, `SPEC.md` §9 (replace §9.1's contract and §9.5's ceremony with pointers to the spec; mark Matrix as on `ChannelAdapter`), `docs/superpowers/specs/2026-10-01-omnichannel-design.md` (migration numbers per A1), `docs/superpowers/runbooks/postgres-foundation-cutover.md` (rehearse `0007` on a production copy; warn that `!link <code>` is gone and users re-link with `/link`), `docs/superpowers/HANDOFF.md`, `AGENTS.md` (§3.1 layer table rows for `domain/channels` and `interface/channels`; §3.4 `!link` note now says "`link` is handled by the inbound pipeline before identity resolution"; §5.2 points at the adapter).

- [ ] **Step 1: Add the remaining architecture tests**

```rust
#[test]
fn application_knows_no_channel_sdk() {
    let hits = production_violations("application", &["matrix_sdk", "serenity", "teloxide", "lettre", "async_imap", "mail_parser", "interface::channels"]);
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}

#[test]
fn channel_domain_is_pure() {
    let hits = violations("domain/channels", &["sqlx", "axum", "matrix_sdk", "reqwest", "utoipa", "tokio"]);
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}
```

(`domain_depends_on_nothing_outside_itself` may already cover the second; keep whichever is stricter, do not duplicate.)

- [ ] **Step 2: Run** `cargo test -p matrix-bot --test architecture` → PASS.
- [ ] **Step 3: Docs.** HANDOFF: sub-project 2 row → "In progress: plan A done"; add a §4 table for plan A tasks with commits; rulings `R-O1`, `R-O2` (spec) and `A1`–`A6` (this plan) in the `Ruling: <decision> — <why> — <cost if wrong>` form; §7 next actions → "write plan B (Discord) against the code A produced". `docs/database.md`: `platform_identities`, `link_tokens`, the backfill migration. `docs/api.md`: the link/identity endpoints and the Telegram webhook path reserved for plan C. `docs/llm.md`: `ProgressSink`, channel-free agent.
- [ ] **Step 4: Final verification** — the full Gate plus `cargo build -p matrix-bot --release`, `npm --prefix frontend run build`, `npm --prefix frontend run test:e2e`, `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1` (with `PG_TEST_URL` pointed at the throwaway server). Record actual results in HANDOFF.
- [ ] **Step 5: Commit** — `docs: omnichannel plan A hand-off, architecture and runbook updates`

---

## Self-review against the spec

| Spec | Covered by |
|---|---|
| §2 D1 chat-initiated linking | Task 9, 10, 11 |
| §2 D2 unlinked identities limited | Task 3 (policy), Task 7 (pipeline) |
| §2 D3 reply-to-origin | Task 8 |
| §2 D4 allowlisted signup | Task 9 (`signup_allowed`), Task 10 (signup routes), Task 11 |
| §2 D5 shared module registry | Task 6, 7 |
| §2 D6–D8 Discord/Email specifics | plans B–D |
| §3.1 domain types, both encodings | Task 1 |
| §3.2 message types, no metadata map | Task 4 |
| §3.3 renderers | Task 4 (HTML subset); per-channel renderers in B–D |
| §4 adapter + registry, replaces Delivery and MatrixRoomSink | Task 4, 7, 8 |
| §5 inbound pipeline, neutral `CommandContext`, `Reply` | Task 6, 7 |
| §6 agent boundary | Task 5, 6, 7 |
| §7.1 schema, backfill, merge lists | Task 2, 9 |
| §7.2 services | Task 3, 9 |
| §7.3 HTTP and web | Task 10, 11 |
| §8.1 Matrix adapter | Task 7 |
| §9 outbound routing, `Destination::Conversation` | Task 8 |
| §10 errors | Task 4 (`ChannelError`), Task 8 (job mapping) |
| §11 invariants 1–6 | Tasks 3, 7, 9, 10 (tests named there); 7–10 in plans B–D |
| §12 tests (architecture, identity, normalization for Matrix, outbound for Matrix) | Tasks 2–8, 12 |
