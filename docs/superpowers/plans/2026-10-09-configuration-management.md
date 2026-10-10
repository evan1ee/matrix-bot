# Configuration Management & Secret Credential Storage — Implementation Plan

> **For agentic workers:** steps use checkboxes (`- [ ]`). Work task by task with strict TDD: failing test → minimal code → green → refactor → run the task's gates. Each task ends in **one commit** that also updates [`HANDOFF.md`](../HANDOFF.md). **Don't start implementation until the user explicitly says so.**

**Status:** Plan ready. Design approved 2026-10-09.
**Target Branch:** `feat/config-management` (from `master`)
**Spec:** [`specs/2026-10-09-configuration-management-design.md`](../specs/2026-10-09-configuration-management-design.md). §-references below point there.
**Authority:** [`AGENTS.md`](../../../AGENTS.md), [`docs/architecture.md`](../../architecture.md), [`docs/database.md`](../../database.md), [`docs/api.md`](../../api.md), [`docs/llm.md`](../../llm.md)

---

## Gates (run at the end of every task that touches the listed area)

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p matrix-bot --lib
cargo test -p matrix-bot --test architecture
cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1   # tasks touching SQL
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # tasks touching API DTOs
npm --prefix frontend run build && npm --prefix frontend run test:e2e           # task 6
```

After any query or migration change: regenerate `.sqlx/` against a migrated scratch database and commit it.

---

## Dependency Graph

```text
Task 1: Domain types & registry (Phase 1)
   │
   ▼
Task 2: Migration 0014 & Repositories (Phase 2)
   │
   ▼
Task 3: Cryptography & Application Services (Phase 3)
   │
   ▼
Task 4: Admin REST API & Authorization (Phase 4)
   │
   ▼
Task 5: Consumer Integration & Dynamic Reload (Phase 5)
   │
   ▼
Task 6: Frontend Admin Console (Phase 6)
   │
   ▼
Task 7: Documentation, Runbooks & Final Verification
```

---

### Task 1: Domain Types, Settings Registry, and Secret Metadata (§7, Phase 1)

Pure domain code; zero SQL, zero external network, zero crypto implementations in domain.

- [x] `domain/config/mod.rs` & `registry.rs`:
  - `SettingKey` enum representing the 20 hot-reloadable runtime keys:
    `LlmDefaultProvider`, `LlmDefaultModel`, `LlmRequestTimeoutSeconds`, `LlmDefaultTemperature`, `LlmDefaultMaxTokens`,
    `SearchTavilyEnabled`, `SearchBraveEnabled`, `SearchTimeoutSeconds`,
    `InferenceOcrEnabled`, `InferenceOcrBaseUrl`, `InferenceSttEnabled`, `InferenceSttBaseUrl`, `InferenceSttModel`, `InferenceSttLanguage`,
    `InferenceTtsEnabled`, `InferenceTtsBaseUrl`, `InferenceTtsModel`, `InferenceTtsVoice`, `InferenceTtsDialect`,
    `InferenceEmbeddingBaseUrl`,
    `RegistrationAllowLinkSignup`.
  - `SettingValue` enum (`String`, `Integer(i64)`, `Float(f64)`, `Boolean(bool)`).
  - `SettingDefinition`: holds key, default value, validation function, and `hot_reloadable: bool` (`true` for all 20 registered runtime keys; restart-required keys remain in `.env`).
  - `SettingRegistry`: lookup by key, validation, unknown key rejection.
- [x] `domain/secrets/mod.rs`:
  - `SecretId(uuid::Uuid)` newtype.
  - `SecretProvider` enum (`OpenAi`, `Anthropic`, `Tavily`, `Stt`, `Tts`, `Generic`).
  - `SecretStatus` enum (`Active`, `Disabled`, `Revoked`).
  - `SecretMetadata`: public non-sensitive representation (`id`, `provider`, `name`, `status`, `key_version`, `masked_preview`, `last_tested_at`, `last_test_status`, `last_test_error`, `created_at`, `updated_at`).
  - `EncryptedSecret`: struct holding ciphertext bytes, nonce bytes, and key version.
  - `SecretValue`: wraps plaintext string with `Drop` memory zeroization and redacted `Debug`/`Display` returning `"[REDACTED]"`. Explicit `expose_secret(&self) -> &str`.
- [x] `domain/config/audit.rs`:
  - `ConfigAuditAction` enum (`SettingUpdated`, `SettingReset`, `SecretCreated`, `SecretUpdated`, `SecretDeleted`, `SecretTested`, `ConfigReloaded`, `AdminGranted`, `AdminRevoked`).
  - `ConfigAuditTarget` enum (`Setting(String)`, `Secret(uuid::Uuid)`, `Account(AccountId)`, `System`).
  - `ConfigAuditEntry` struct. Enforces invariant that no secret ciphertext or plaintext is stored.
- [x] `domain/accounts/` & `application/accounts/`:
  - Add `AccountRole` enum (`User`, `Admin`) and `role: AccountRole` to `Account`.
  - Controlled CLI admin bootstrap and recovery (`matrix-bot admin grant|revoke <account_id>`), requiring an enrolled passkey (`has_passkey`).
- [x] Unit tests in each module:
  - Validation of setting bounds (e.g., timeouts, temperatures, port numbers, URLs).
  - Rejection of unknown setting keys.
  - `SecretValue` redaction in `format!("{val}")` and `format!("{val:?}")`.
  - `ConfigAuditEntry` sanitization assertions.
- [x] Gates: `cargo test -p matrix-bot --lib`, `cargo test -p matrix-bot --test architecture`, `cargo clippy`.

---

### Task 2: Database Migration 0014 and Repositories (§5, §9.1, Phase 2)

- [x] Write `backend/migrations/0015_configuration_and_secrets.sql` (0014 taken by notes):
  - Add `role` column to `accounts` (`role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin'))`).
  - Create table `app_settings` (`key TEXT PRIMARY KEY`, `value JSONB NOT NULL`, `updated_at TIMESTAMPTZ`, `updated_by UUID REFERENCES accounts(id) ON DELETE SET NULL`).
  - Create table `provider_secrets` (`id UUID PRIMARY KEY DEFAULT gen_random_uuid()`, `provider TEXT NOT NULL`, `name TEXT NOT NULL DEFAULT 'default'`, `secret_ciphertext BYTEA NOT NULL`, `nonce BYTEA NOT NULL`, `key_version INT NOT NULL DEFAULT 1`, `status TEXT NOT NULL DEFAULT 'active'`, `created_at TIMESTAMPTZ`, `updated_at TIMESTAMPTZ`, `updated_by UUID REFERENCES accounts(id) ON DELETE SET NULL`, `last_tested_at TIMESTAMPTZ`, `last_test_status TEXT`, `last_test_error TEXT`, `metadata JSONB`, `CONSTRAINT uq_provider_secrets_slot UNIQUE (provider, name)`).
  - Create table `config_audit_log` (`id BIGSERIAL PRIMARY KEY`, `actor_id UUID REFERENCES accounts(id) ON DELETE SET NULL`, `action TEXT NOT NULL`, `target_type TEXT NOT NULL`, `target_key TEXT NOT NULL`, `outcome TEXT NOT NULL`, `detail JSONB`, `created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP`).
- [x] Verify `OWNED_TABLES` invariants: neither `app_settings` nor `provider_secrets` is in `OWNED_TABLES` (global tables).
- [x] Update `domain/accounts/merge.rs` & `application/accounts/merge.rs`:
  - Remap `config_audit_log.actor_id`, `app_settings.updated_by`, and `provider_secrets.updated_by` to the winning account on admin merge.
- [x] Repository `infrastructure/postgres/repositories/app_settings.rs`:
  - `get_setting(executor, key) -> Result<Option<SettingRow>>`
  - `list_settings(executor) -> Result<Vec<SettingRow>>`
  - `upsert_setting(executor, key, value, updated_by) -> Result<()>`
  - `delete_setting(executor, key) -> Result<bool>`
- [x] Repository `infrastructure/postgres/repositories/provider_secrets.rs`:
  - `list_secret_metadata(executor) -> Result<Vec<SecretMetadataRow>>`
  - `get_secret_by_id(executor, id) -> Result<Option<EncryptedSecretRow>>`
  - `get_secret_by_provider(executor, provider, name) -> Result<Option<EncryptedSecretRow>>`
  - `insert_secret(executor, row) -> Result<uuid::Uuid>`
  - `update_secret(executor, id, ciphertext, nonce, key_version, updated_by) -> Result<()>`
  - `update_test_status(executor, id, status, error, tested_at) -> Result<()>`
  - `delete_secret(executor, id) -> Result<bool>`
- [x] Repository `infrastructure/postgres/repositories/config_audit.rs`:
  - `insert_audit_entry(executor, entry) -> Result<()>`
  - `list_audit_entries(executor, limit, offset) -> Result<Vec<ConfigAuditRow>>`
- [x] Integration tests in `infrastructure/postgres/repositories/` against PostgreSQL test schema:
  - Setting upsert and delete round-trips.
  - Secret unique constraint `(provider, name)` violation handling.
  - Audit log insertions with null and non-null actor IDs.
- [x] Run `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets` and commit `.sqlx/`.
- [x] Gates: `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1`, `cargo clippy`.

---

### Task 3: Cryptography & Application Services (§6, §8, §9.2, Phase 3)

- [x] Add `aes-gcm = "0.10"`, `arc-swap = "1.7"`, and `hex = "0.4"` dependencies to `backend/Cargo.toml`.
- [x] `infrastructure/crypto/aes_gcm.rs`:
  - `AesGcmSecretProtector` struct implementing AES-256-GCM authenticated encryption.
  - Generates unique 96-bit CSPRNG nonces per encryption.
  - Binds AAD `format!("{provider}:{name}:{key_version}")`.
  - `MasterKeyProvider`: loads 256-bit symmetric key from `SECRETS_ENCRYPTION_KEY` env var. Fails closed with `CryptoError::KeyUnavailable` if missing. Supports versioned keys.
  - `FailClosedSecretProtector`: safe fallback when encryption key is unconfigured.
  - Comprehensive unit tests: encryption/decryption round trips, corrupted ciphertext detection, AAD mismatch detection, nonce uniqueness.
- [x] `application/config/mod.rs` (`AppConfigService`):
  - `get_setting(&self, key: SettingKey) -> Result<SettingValue, AppError>`
  - `list_settings(&self) -> Result<Vec<SettingItem>, AppError>`
  - `update_setting(&self, actor: &AccountId, key: SettingKey, value: SettingValue) -> Result<(), AppError>`:
    Opens transaction, validates value, writes to `app_settings`, logs to `config_audit_log`, publishes `SystemEvent::ConfigUpdated`, commits, updates snapshot.
  - `reset_setting(&self, actor: &AccountId, key: SettingKey) -> Result<(), AppError>`
  - `get_status(&self) -> Result<ConfigurationStatusDto, AppError>`
  - In-memory `ArcSwap<RuntimeConfigSnapshot>` for lock-free snapshot access (`snapshot.rs`).
- [x] `application/secrets/mod.rs` (`SecretService`):
  - `list_metadata(&self) -> Result<Vec<SecretMetadata>, AppError>`
  - `store_secret(&self, actor: &AccountId, provider: SecretProvider, name: &str, secret: SecretValue, metadata: Option<Value>) -> Result<SecretId, AppError>`:
    Encrypts secret with `AesGcmSecretProtector`, writes to `provider_secrets`, logs non-sensitive audit entry, commits.
  - `replace_secret(&self, actor: &AccountId, id: SecretId, secret: SecretValue) -> Result<(), AppError>`
  - `delete_secret(&self, actor: &AccountId, id: SecretId) -> Result<(), AppError>`
  - `get_decrypted_secret(&self, provider: SecretProvider, name: &str) -> Result<Option<SecretValue>, AppError>`
  - `test_credential(&self, actor: &AccountId, id: SecretId) -> Result<TestResultDto, AppError>`
- [x] Wire `app_config` and `secrets` into `AppServices` (`application/mod.rs` and `AppServices::new`).
- [x] Application service tests (atomicity, outbox event verification, audit log verification).
- [x] Gates: `cargo test -p matrix-bot --lib`, `cargo test -p matrix-bot live_postgres`, `cargo clippy`, `cargo test --test architecture`.

---

### Task 4: Admin REST API Endpoints & Authorization (§10, Phase 4)

- [x] `interface/http/auth/extractor.rs`:
  - Introduce `AdminSession` (ordinary admin `GET`s: requires `role == AccountRole::Admin`, enrolled passkey, non-MCP token, non-recovery-restricted session) and `AdminUser` (admin writes & secret tests: `AdminSession` + 5-minute WebAuthn passkey step-up `STEP_UP_WINDOW_MINUTES = 5`).
- [x] Create `interface/http/admin/`:
  - `settings.rs`:
    - `GET /api/v1/admin/settings` (`list_settings`, `AdminSession`)
    - `GET /api/v1/admin/settings/:key` (`get_setting`, `AdminSession`)
    - `PUT /api/v1/admin/settings/:key` (`update_setting`, `AdminUser`)
    - `DELETE /api/v1/admin/settings/:key` (`reset_setting`, `AdminUser`)
  - `secrets.rs`:
    - `GET /api/v1/admin/secrets` (`list_secrets`, `AdminSession`): returns only metadata.
    - `POST /api/v1/admin/secrets` (`create_secret`, `AdminUser`): accepts secret payload, returns metadata only.
    - `PUT /api/v1/admin/secrets/:id` (`replace_secret`, `AdminUser`): returns metadata only.
    - `DELETE /api/v1/admin/secrets/:id` (`delete_secret`, `AdminUser`)
    - `POST /api/v1/admin/secrets/:id/test` (`test_secret`, `AdminUser`)
  - `status.rs`:
    - `GET /api/v1/admin/config-status` (`config_status`, `AdminSession`)
  - `audit.rs`:
    - `GET /api/v1/admin/audit-logs` (`list_audit_logs`, `AdminSession`)
- [x] Register routes in `interface/http/routes.rs` under `/admin/...`.
- [x] Add all admin mutating and secret-test routes to `SENSITIVE` routes list in `interface/http/mod.rs`.
- [x] Add unit and integration tests:
  - Unauthorized (no cookie / token) → `401 Unauthorized`.
  - Non-admin user (`role == "user"`) → `403 Forbidden`.
  - Admin user without enrolled passkey → `403 Forbidden`.
  - Admin user with stale step-up (> 5 min) → `200 OK` on ordinary `GET`s, `403 Forbidden ("Confirm with your passkey to continue")` on writes and secret tests.
  - Verified Admin user → `200 OK`.
  - Assert response JSON from `GET /api/v1/admin/secrets` never contains `"secret"`, `"ciphertext"`, or `"nonce"` fields.
- [x] Regenerate OpenAPI contract: `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`.
- [x] Gates: `cargo test -p matrix-bot --all-targets`, `generate-openapi`.

---

### Task 5: Consumer Integration & Dynamic Runtime Reload (§11, §9.3, Phase 5)

- [x] LLM dynamic resolution (`backend/src/llm/resolved_config.rs`, `backend/src/llm/turn.rs`):
  - Inject active snapshot into resolution pipeline (`llm.default_provider`, `llm.default_model`, `llm.default_temperature`, `llm.default_max_tokens`, `llm.request_timeout_seconds`).
  - Resolution precedence: `AiUserConfig > Runtime Snapshot (app_settings + provider_secrets) > Environment (AiEnvConfig) > Defaults`.
  - Verify model and provider updates take effect immediately on the next AI request without restart.
- [x] Web Search (`backend/src/llm/search.rs`):
  - Read `search.tavily_enabled`, `search.brave_enabled`, `search.timeout_seconds`, and decrypted Tavily/Brave API keys from snapshot before falling back to `TAVILY_API_KEY` env var.
- [x] Inference services (STT, TTS, OCR, Embedding Base URL) & Link Signup (`application/identity/link.rs`):
  - Inspect snapshot for `inference.ocr.*`, `inference.stt.*`, `inference.tts.*`, `inference.embedding.base_url`, and `registration.allow_link_signup`.
  - Keep restart-required settings and credentials (`EMBEDDING_MODEL`, `DISCORD_*`, `TELEGRAM_*`, `EMAIL_*`, `S3_*`) in `.env` only.
- [x] Provider connection testers (`infrastructure/inference/testers/`):
  - `test_llm_credential(endpoint, key)`: validates credentials against provider ping/models endpoint.
  - `test_tavily_credential(key)`: validates Tavily search key.
- [x] Outbox listener integration (`events/`):
  - On `SystemEvent::ConfigUpdated` / `SystemEvent::SecretChanged`, local cache reloads snapshot.
- [x] Tests:
  - Changing Tavily key in `provider_secrets` immediately routes search queries using the new key.
  - Changing LLM model in `app_settings` immediately sends the updated model string in chat turns.
  - Changing `registration.allow_link_signup` immediately toggles link signup.
  - All 20 registered settings verified as `hot_reloadable: true` and indexed consistently.
- [x] Gates: `cargo test -p matrix-bot --all-targets`, `cargo clippy`.

---

### Task 6: Frontend Admin Console & Integration (§12, Phase 6)

- [x] Add `admin` navigation item to `frontend/src/components/Sidebar.tsx` with Shield icon, conditionally visible if `user.role === 'admin'`.
- [x] Create `frontend/src/modules/admin/`:
  - `AdminModule.tsx`: main module container with tabs:
    - **Overview**: System health cards, effective settings status, hot-reloadable configuration summary.
    - **General Settings**: Form editors for registration policies.
    - **LLM & Models**: Provider selection, model name, temperature, max tokens, timeout.
    - **Search & Inference**: Tavily/Brave search toggles, STT/TTS/OCR/Embedding endpoints and models.
    - **Secrets Vault**: Table of credentials metadata, masked previews, "Replace Secret" modal, "Test Connection" button, "Delete" modal.
    - **Audit Log**: Paginated table of administrative configuration changes.
- [x] Implement write-only credential form inputs (`type="password"`, form state scrubbed on submit, never populating previous secret values).
- [x] Update `frontend/src/modules/admin/useAdmin.ts` so `GET` queries use normal session auth while all mutations wrap in `useSensitiveAction` (`withStepUp`).
- [x] Add fake API endpoints to `frontend/e2e/fake-api.ts`.
- [x] Write Playwright E2E test: `frontend/e2e/admin.spec.ts`.
- [x] Gates: `npm --prefix frontend run build`, `npm --prefix frontend run test:e2e`.

---

### Task 7: Documentation, Runbooks & Final Verification (§14, §15)

- [x] Update `.env.example`, `README.md`, `docs/architecture.md`, `docs/api.md`, `docs/llm.md`, and `docs/database.md` to reflect the configuration architecture, 20 hot-reloadable keys, and CLI admin bootstrap (`matrix-bot admin grant|revoke <account_id>`).
- [x] Update `docs/superpowers/HANDOFF.md` with status, rulings, and verification evidence.
- [x] Execute full verification suite.
- [x] Clean git diff check: no secrets, debug code, or temporary files.

---

## Rulings Recorded in HANDOFF.md

- **Ruling R-CFG1:** `app_settings` and `provider_secrets` are instance-wide system configuration tables and are **not** added to `OWNED_TABLES`. Merging an admin account remaps `updated_by` and `actor_id` in `infrastructure/postgres/merge.rs`.
- **Ruling R-CFG2:** Secret read APIs return metadata only (`SecretMetadataDto`), with ciphertext and plaintext completely omitted from response DTOs.
- **Ruling R-CFG3:** Ordinary admin `GET` endpoints use `AdminSession` (`role == 'admin'`, enrolled passkey required, non-MCP token, non-recovery-restricted session), while admin writes and `POST /api/v1/admin/secrets/:id/test` require `AdminUser` (`AdminSession` + 5-minute WebAuthn passkey step-up `STEP_UP_WINDOW_MINUTES = 5`).
- **Ruling R-CFG4:** Missing `SECRETS_ENCRYPTION_KEY` fails closed for secret management operations; unencrypted fallback storage is prohibited.
- **Ruling R-CFG5:** Runtime configuration snapshot is stored in an `ArcSwap<RuntimeConfigSnapshot>` (preloaded at boot), guaranteeing lock-free reads on request and AI turn paths.
- **Ruling R-CFG6:** Initial administrator bootstrap and recovery are CLI/SQL-only (`matrix-bot admin grant|revoke <account_id>`); public registration and passkey enrollment never grant `admin` implicitly.
- **Ruling R-CFG7:** Only the 20 hot-reloadable settings live in `SettingRegistry` / Admin Console (all wired to live runtime consumers); the 12 restart-required settings (`EMBEDDING_MODEL`, `DISCORD_*`, `TELEGRAM_*`, `EMAIL_*`) stay in `.env`.
