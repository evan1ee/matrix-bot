# Engineering Housekeeping & Hardening Design

**Date:** 2026-10-04  
**Status:** Draft for approval  
**Context:** [`SPEC.md`](../../SPEC.md), [`docs/superpowers/HANDOFF.md`](../HANDOFF.md) (§11 Deferred items)  
**Target Branch:** `feat/hardening-housekeeping`

---

## 1. Problem Statement

With all four core architecture sub-projects (PostgreSQL Foundation, Omnichannel, Storage & Autonomous Agent, and Web Console) complete and merged to `master`, several known deferred items in `HANDOFF.md §11` remain:

1. **Automated Clippy in CI**: The workspace has achieved zero compiler warnings and zero clippy warnings across all targets, but CI only runs `cargo fmt --check` and `cargo test`. Without `cargo clippy --workspace --all-targets -- -D warnings` in CI, regressions could be introduced unnoticed.
2. **Database Statement Timeout (`statement_timeout`)**: R7 in `HANDOFF.md` noted `statement_timeout` was deferred. Unbounded statements on the shared connection pool pose starvation risks if an unexpected slow query blocks a connection indefinitely.
3. **Link Token Accumulation**: One-time `/link` authorization tokens are single-use and expire in 10 minutes (`LINK_TOKEN_TTL`), but consumed or expired rows in `link_tokens` are never deleted.
4. **Channel-Specific Help Formatting**: In Discord and Telegram, typing `/help` prints command usage formatted with Matrix `!` command prefixes (e.g., `!todo`, `!finance`) rather than platform-native `/` slash commands.
5. **Layering Seam for MCP**: `interface/http/mcp.rs` and `interface/chat/mcp.rs` make direct calls into `infrastructure/postgres/repositories/mcp::McpTokenRepository`, bypassing the `application/` layer.

---

## 2. Architecture & Design Rationale

### 2.1 CI Automation (`.github/workflows/ci.yml`)

- Add `clippy` to the `components:` list in `backend` and `inference` jobs.
- Add step `Run clippy`:
  - In `backend`: `cargo clippy -p matrix-bot --all-targets -- -D warnings`
  - In `inference`: `cargo clippy -p matrix-bot-inference --all-targets -- -D warnings`
- Keep jobs parallel, cached via `Swatinem/rust-cache@v2`.

### 2.2 Database Statement Timeout (`statement_timeout`)

- In `backend/src/db/pool.rs`:
  - PostgreSQL allows setting a default session statement timeout via connection query parameters or pool initialization.
  - Set a safe default statement timeout of 15 seconds (`15000ms`) via `after_connect`:
    ```rust
    .after_connect(|conn, _meta| {
        Box::pin(async move {
            sqlx::query("SET statement_timeout = '15s'").execute(conn).await?;
            Ok(())
        })
    })
    ```
  - This protects interactive REST requests, Matrix sync processing, and chat handlers from hanging transactions while leaving ample room for standard transactional writes (which take < 50ms).
  - Background migration scripts and test pools run unlocked without statement timeouts.

### 2.3 Link Token Retention & Purging

- In `infrastructure/postgres/repositories/link_tokens.rs`:
  - Add `pub async fn purge_expired(conn: impl PgExecutor<'_>, cutoff: DateTime<Utc>) -> Result<u64, sqlx::Error>`.
  - Deletes rows where `expires_at < cutoff` (or where `consumed_at < cutoff`).
- In `application/identity/link.rs`:
  - Add `pub async fn purge_expired(&self, older_than: Duration) -> Result<u64, AppError>`.
- In `events/purge.rs` (the existing hourly purge task):
  - In addition to purging outbox events, execute `link_service.purge_expired(Duration::from_secs(86400)).await` (retaining expired tokens for 24h for audit/forensics before deleting).

### 2.4 Channel-Specific `/help` Formatting

- In `domain/channels/mod.rs` & `interface/chat/core/help.rs`:
  - Update `generate_help_html(module_helps: &[(String, Vec<CommandHelp>)], channel: Channel) -> String`.
  - For `Channel::Discord` and `Channel::Telegram`:
    - Prefix: `/` instead of `!`
    - Header: `"🤖 Discord Bot Commands"` / `"🤖 Telegram Bot Commands"`
    - Example/tip: `"Tip: You can use /command or mention me in messages."`
  - For `Channel::Matrix` and `Channel::Email`:
    - Keep `!` prefix and `"🤖 Matrix Bot Commands"`.
- Update `CoreModule::handle_command`:
  - Pass `ctx.sender.channel()` to `generate_help_html`.

### 2.5 `McpService` Application Layer Extraction

- Create `backend/src/application/mcp/mod.rs`:
  ```rust
  #[derive(Clone)]
  pub struct McpService {
      pool: DbPool,
  }
  ```
  - Expose application use cases:
    - `pub async fn create_token(&self, account_id: &AccountId, label: &str, expires_at: Option<DateTime<Utc>>) -> Result<(String, StoredMcpToken), AppError>`
    - `pub async fn list_tokens(&self, account_id: &AccountId) -> Result<Vec<StoredMcpToken>, AppError>`
    - `pub async fn revoke_token(&self, account_id: &AccountId, id: &str) -> Result<bool, AppError>`
    - `pub async fn revoke_token_by_prefix(&self, account_id: &AccountId, prefix: &str) -> Result<bool, AppError>`
    - `pub async fn resolve_token(&self, raw_token: &str) -> Result<Option<StoredMcpToken>, AppError>`
- Wire `McpService` into `AppServices` (`application/mod.rs`).
- Repoint `interface/http/mcp.rs` and `interface/chat/mcp.rs` to use `services.mcp`.
- Update `tests/architecture.rs` to guarantee no interface layer references `McpTokenRepository` directly.

---

## 3. Verification Criteria

1. `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings.
2. `cargo test -p matrix-bot --test architecture`: all architecture tests pass.
3. `cargo test -p matrix-bot --all-targets`: full test suite green.
4. `npm --prefix frontend run test:e2e`: 199/199 Playwright E2E tests green.
5. `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: byte-identical OpenAPI contract.
