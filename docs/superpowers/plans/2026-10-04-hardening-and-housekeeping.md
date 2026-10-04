# Engineering Housekeeping & Hardening Plan

**Status:** Completed  
**Design:** [`docs/superpowers/specs/2026-10-04-hardening-and-housekeeping-design.md`](file:///d:/01_Data/code/matrix-hello-bot/docs/superpowers/specs/2026-10-04-hardening-and-housekeeping-design.md)  
**Target Branch:** `feat/hardening-housekeeping` (branched from `master`)

---

## Task Breakdown

### Task 1: Add Clippy Lint Check to CI Workflows
- [x] Edit `.github/workflows/ci.yml`:
  - In `backend` job, add `clippy` component to `dtolnay/rust-toolchain@stable`.
  - Add step:
    ```yaml
    - name: Clippy check
      run: cargo clippy -p matrix-bot --all-targets -- -D warnings
    ```
  - In `inference` job, add `clippy` component to `dtolnay/rust-toolchain@stable`.
  - Add step:
    ```yaml
    - name: Clippy check
      run: cargo clippy -p matrix-bot-inference --all-targets -- -D warnings
    ```
- [x] Verify locally:
  - `cargo clippy --workspace --all-targets -- -D warnings` exits 0.

---

### Task 2: Configure Database Pool Statement Timeout
- [x] Edit `backend/src/db/pool.rs`:
  - In `connect_pool`, add `.after_connect` hook to set `statement_timeout = '15s'`.
  - Ensure unconstrained test database creations and schema setup remain unaffected.
- [x] Add unit test in `backend/src/db/pool.rs`:
  - Assert that a statement exceeding the timeout (`SELECT pg_sleep(1)`) fails if timeout is set short (or verify `SHOW statement_timeout` equals `15s`).
- [x] Verify: `cargo test -p matrix-bot --test architecture` and `cargo test -p matrix-bot --lib db::pool`.

---

### Task 3: Link Token Retention & Hourly Purge
- [x] In `backend/src/infrastructure/postgres/repositories/link_tokens.rs`:
  - Add `pub async fn purge_expired(conn: impl PgExecutor<'_>, cutoff: DateTime<Utc>) -> Result<u64, sqlx::Error>`.
  - Unit test in `repositories/link_tokens.rs` asserting expired tokens are deleted while active unexpired ones are kept.
- [x] In `backend/src/application/identity/link.rs`:
  - Add `pub async fn purge_expired(&self, older_than: chrono::Duration) -> Result<u64, AppError>`.
- [x] In `backend/src/events/purge.rs` & `backend/src/events/mod.rs`:
  - Pass `link: LinkService` to `purge::run`.
  - Run `link.purge_expired(chrono::Duration::hours(24))` on every hourly tick.
- [x] Verify: `cargo test -p matrix-bot --all-targets`.

---

### Task 4: Channel-Specific `/help` Formatting
- [x] In `backend/src/interface/chat/core/help.rs`:
  - Update `generate_help_html(module_helps: &[(String, Vec<CommandHelp>)], channel: Channel) -> String`.
  - Prefix with `/` on Discord/Telegram, `!` on Matrix/Email.
  - Channel-specific header and tip message.
  - Unit test covering Matrix and Discord/Telegram output formats.
- [x] In `backend/src/interface/chat/core/mod.rs`:
  - Pass `ctx.sender.channel()` to `generate_help_html`.
- [x] Verify: `cargo test -p matrix-bot --all-targets`.

---

### Task 5: Application Layer `McpService` Extraction
- [x] Create `backend/src/application/mcp/mod.rs`:
  - `McpService { pool: DbPool }`.
  - Encapsulate `create_token`, `list_tokens`, `revoke_token`, `revoke_token_by_prefix`, `resolve_token`.
- [x] Add `McpService` to `AppServices` in `backend/src/application/mod.rs`.
- [x] Refactor `backend/src/interface/http/mcp.rs`:
  - Handlers accept `services: Extension<AppServices>` (or read from state) and call `services.mcp`.
  - Eliminate direct imports of `McpTokenRepository`.
- [x] Refactor `backend/src/interface/chat/mcp.rs`:
  - Use `services.mcp` instead of `McpTokenRepository`.
- [x] Refactor `backend/src/application/accounts/principal.rs`:
  - Use `services.mcp` or delegate via `McpService`.
- [x] Add rule to `backend/tests/architecture.rs`:
  - Assert `interface/` does not import `McpTokenRepository`.
- [x] Verify:
  - `cargo test -p matrix-bot --test architecture`
  - `cargo test -p matrix-bot --all-targets`
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json` (zero diff).

---

### Task 6: Verification, Documentation, and Review
- [x] Run full test suite and gates:
  - `cargo check -p matrix-bot --all-targets`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test -p matrix-bot --test architecture`
  - `npm --prefix frontend run test:e2e`
- [x] Update `docs/superpowers/HANDOFF.md` (§4 progress, §9 rulings, §11 clear deferred items).
- [x] Commit with conventional commits on `feat/hardening-housekeeping`.
