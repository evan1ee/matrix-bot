# Account UUID (`0002_account_uuid`) Implementation Plan

> **Date:** 2026-09-29  
> **Status:** Proposed Plan  
> **Parent Spec:** [`SPEC.md §4.1`](../../../SPEC.md), [`docs/superpowers/specs/2026-09-24-postgres-foundation-design.md §4.2, §6 step 7`](../specs/2026-09-24-postgres-foundation-design.md)  
> **Context:** Sub-project 1, Step 7 of the PostgreSQL Foundation refactor.

---

## 1. Goal

Migrate `accounts.id`, all 23 `owner_id` columns, and all 11 `account_id` columns from `TEXT` (`acct_<uuid>`) to native PostgreSQL `UUID` with foreign key constraints `REFERENCES accounts(id) ON DELETE CASCADE`.

In Rust:
- Replace `domain::accounts::AccountId(String)` with `domain::accounts::AccountId(uuid::Uuid)`.
- Ensure `AccountId` implements `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Serialize`, `Deserialize`, `Display`, `FromStr`, `TryFrom<&str>`.
- Wire format serialization remains identical (`acct_<32 hex chars>`), ensuring **zero API drift** on `backend/openapi.json` and complete compatibility with `frontend/`.
- Portfolio audit log hashing continues to fold the canonical `acct_<32 hex chars>` string into every entry digest, keeping historical audit chains valid.

---

## 2. Table & Column Inventory

### 2.1 The Root Table
- `accounts`: `id` (`TEXT PRIMARY KEY` $\to$ `UUID PRIMARY KEY`)

### 2.2 The 23 Owner-Keyed Tables (`OWNED_TABLES` + `OWNER_ID_MERGED_SEPARATELY`)
All `owner_id` columns become `UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE` (or `UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE`):
1. `user_settings` (`owner_id` PK)
2. `webhooks` (`owner_id`)
3. `webhook_outbox` (`owner_id`)
4. `ai_user_configs` (`owner_id` PK)
5. `ai_conversations` (`owner_id`)
6. `portfolio_holdings` (`owner_id`)
7. `portfolio_cash_balances` (`owner_id`)
8. `portfolio_daily_snapshots` (`owner_id`)
9. `portfolio_price_alerts` (`owner_id`)
10. `portfolio_audit_log` (`owner_id`)
11. `footprints` (`owner_id`)
12. `habits` (`owner_id`)
13. `habit_logs` (`owner_id`)
14. `life_day_logs` (`owner_id`)
15. `life_profiles` (`owner_id` PK)
16. `bookmarks` (`owner_id`)
17. `todos` (`owner_id`)
18. `reminders` (`owner_id`)
19. `finance_categories` (`owner_id`)
20. `transactions` (`owner_id`)
21. `finance_shares` (`owner_id` PK)
22. `recurring_bills` (`owner_id`)
23. `bot_message_targets` (`owner_id`)

### 2.3 The 11 Account-Keyed Tables (`ACCOUNT_ID_FOLLOWS`, `ACCOUNT_ID_DISCARDED`, `ACCOUNT_ID_UNTOUCHED`)
All `account_id` columns become `UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE` (except `web_users.account_id` which is nullable):
1. `matrix_links` (`account_id`)
2. `webauthn_credentials` (`account_id`)
3. `sessions` (`account_id`)
4. `matrix_link_challenges` (`account_id`)
5. `passkey_enrollment_tokens` (`account_id`)
6. `recovery_codes` (`account_id`)
7. `recovery_requests` (`account_id`)
8. `security_events` (`account_id`)
9. `web_users` (`account_id`, nullable, `ON DELETE SET NULL`)
10. `mcp_tokens` (`account_id`)
11. `account_rooms` (`account_id`)

Total columns converted: **35 columns across 35 tables**.

---

## 3. Implementation Tasks

### Task 1: Migration `0002_account_uuid.sql`
- **File:** `backend/migrations/0002_account_uuid.sql`
- **Content:**
  1. **Safety Guard Block (`DO $$ ... $$`):**
     - Queries all 35 tables for non-null values not matching `^acct_([0-9a-fA-F]{32}|[0-9a-fA-F-]{36})$`.
     - Queries for any orphaned rows where `owner_id` / `account_id` does not exist in `accounts(id)`.
     - Raises an exception naming the table, column, and offending value(s) if any violation is found.
  2. **Drop Existing Foreign Keys:**
     - Drop foreign key constraints on `webhooks`, `ai_user_configs`, `ai_conversations`, `mcp_tokens`, `account_rooms`.
  3. **Column Type Alteration (`USING substr(..., 6)::uuid`):**
     - Alter `accounts.id` to `UUID`.
     - Alter all 23 `owner_id` columns to `UUID`.
     - Alter all 11 `account_id` columns to `UUID`.
  4. **Foreign Key Constraints:**
     - Add `FOREIGN KEY (...) REFERENCES accounts(id) ON DELETE CASCADE` (and `ON DELETE SET NULL` for `web_users.account_id`).
- **Tests:** Add integration test in `backend/src/db/migrate.rs` asserting all 35 columns have `data_type = 'uuid'`.

### Task 2: Update `domain::accounts::AccountId` Type
- **File:** `backend/src/domain/accounts/mod.rs`
- **Specification:**
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
  pub struct AccountId(uuid::Uuid);

  impl AccountId {
      pub fn new(id: uuid::Uuid) -> Self { Self(id) }
      pub fn from_uuid(id: uuid::Uuid) -> Self { Self(id) }
      pub fn as_uuid(&self) -> &uuid::Uuid { &self.0 }
      pub fn uuid(&self) -> uuid::Uuid { self.0 }
      pub fn into_inner(self) -> uuid::Uuid { self.0 }
      pub fn generate() -> Self { Self(uuid::Uuid::new_v4()) }
      pub fn to_handle(&self) -> uuid::Uuid { self.0 }
      pub fn from_handle(handle: &uuid::Uuid) -> Self { Self(*handle) }
      pub fn from_seed(seed: &str) -> Self {
          Self(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, seed.as_bytes()))
      }
  }
  ```
- **Serialization / Deserialization:**
  - `Display` and `Serialize` write `format!("acct_{}", self.0.simple())`.
  - `Deserialize`, `FromStr`, `TryFrom<&str>` strip `acct_` prefix if present and parse via `uuid::Uuid::parse_str`.
- **Tests:** Unit tests in `backend/src/domain/accounts/mod.rs` covering parsing, serialization, case insensitivity, hyphenated & simple UUIDs, and error cases.

### Task 3: Portfolio Audit Log Hashing & Pre-migration Regression Test
- **Files:**
  - `backend/src/domain/portfolio/audit.rs`
  - `backend/src/domain/portfolio/models.rs`, `alerts.rs`, `history.rs`
- **Changes:**
  - Update `owner_id` fields to `AccountId`.
  - Ensure `compute_entry_hash` hashes `owner_id.to_string()` (`acct_<32 hex chars>`).
  - Add pinned regression test with a pre-migration digest verifying historical hash preservation.

### Task 4: Repositories & SQLx Queries Conversion
- **Files:**
  - `backend/src/infrastructure/postgres/repositories/accounts/*.rs`
  - `backend/src/infrastructure/postgres/repositories/rooms.rs`
  - `backend/src/infrastructure/postgres/repositories/todos.rs`
  - `backend/src/infrastructure/postgres/repositories/reminders.rs`
  - `backend/src/infrastructure/postgres/repositories/finance/*.rs`
  - `backend/src/infrastructure/postgres/repositories/portfolio/*.rs`
  - `backend/src/infrastructure/postgres/repositories/bookmarks.rs`
  - `backend/src/infrastructure/postgres/repositories/habits.rs`
  - `backend/src/infrastructure/postgres/repositories/life.rs`
  - `backend/src/infrastructure/postgres/repositories/footprints.rs`
  - `backend/src/infrastructure/postgres/repositories/webhooks.rs`
  - `backend/src/infrastructure/postgres/repositories/llm.rs`
  - `backend/src/infrastructure/postgres/merge.rs`
  - `backend/src/accounts/*.rs` (compatibility facades)
- **Changes:**
  - Bind `account_id.as_uuid()` or `*account_id.as_uuid()` for `owner_id` and `account_id`.
  - Read `uuid::Uuid` from rows and convert using `AccountId::from_uuid(...)`.
  - In `merge.rs`, bind `Uuid` in dynamic queries.
- **SQLx Cache:**
  - Run `createdb sqlx_prepare` and `sqlx migrate run`.
  - Regenerate offline cache with `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets`.

### Task 5: Test Suite Updates & Test Helpers
- **Files:**
  - `backend/src/interface/http/auth/types.rs`: provide test helper for creating test accounts.
  - Update tests that pass synthetic account strings like `"acct_alice"` to use valid UUIDs (e.g. `AccountId::from_seed("alice")`) and ensure the account exists in `accounts` when foreign key constraints are enforced.
  - Update `test_migrates_a_real_database` in `backend/src/db/catalog.rs`.

### Task 6: Quality Gates & Verification
1. `cargo fmt --all -- --check`
2. `cargo check -p matrix-bot --all-targets`
3. `cargo test -p matrix-bot --test architecture`
4. `cargo test -p matrix-bot --all-targets`
5. `cargo test -p matrix-bot-inference --all-targets`
6. `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
7. `npm --prefix frontend run build`
8. `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets`
9. Update `docs/HANDOFF.md` §4 to mark Step 7 complete.
