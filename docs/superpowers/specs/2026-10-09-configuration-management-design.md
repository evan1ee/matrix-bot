# Configuration Management & Secret Credential Storage — Design Specification

> **Date:** 2026-10-09
> **Status:** Approved 2026-10-09. Plan: [`plans/2026-10-09-configuration-management.md`](../plans/2026-10-09-configuration-management.md)
> **Target Branch:** `feat/config-management`
> **Authority:** [`AGENTS.md`](../../../AGENTS.md), [`docs/architecture.md`](../../architecture.md), [`docs/database.md`](../../database.md), [`docs/api.md`](../../api.md), [`docs/llm.md`](../../llm.md)
> **Scope:** Backend configuration refactoring, AES-256-GCM encrypted credential storage, Admin REST API, and React Admin UI integration.

---

## 1. Objective

Refactor configuration management in `matrix-bot` to cleanly separate **deployment configuration**, **mutable runtime application settings**, and **secret provider credentials**.

The system enables authorized administrators to configure runtime settings and manage external service credentials through a dedicated Admin UI without requiring server restarts or manual `.env` file editing for routine operational changes.

### Core Principles & Guarantees

1. **Deployment vs. Runtime Separation:** Environment variables remain the authoritative source for bootstrap, network binding, database connectivity, and master encryption key material. Mutable operational settings live in PostgreSQL.
2. **Encrypted Secret Storage:** All API keys, tokens, passwords, and client secrets are authenticated-encrypted at rest with AES-256-GCM using unique nonces and authenticated additional data (AAD). Encryption master keys are supplied exclusively via deployment configuration and never touch the database.
3. **Zero Plaintext Secret Leakage:** Read APIs, logs, traces, errors, SSE events, and audit logs never expose plaintext secret material or ciphertext. Secret inputs in the UI are write-only.
4. **Strong Authorization & Step-Up:** Administrative endpoints require authenticated active accounts with the `admin` role **and** recent WebAuthn passkey step-up verification (`VerifiedUser`), along with strict Origin CSRF protection.
5. **Architectural Purity & Layer Invariants:** Strict adherence to the repository's modular monolith and layer-first structure (`domain` → `application` → `infrastructure/postgres` → `interface/http`). No SQL in handlers or services; no transport models in the domain.
6. **Graceful Degradation & Compatibility:** Preserves documented defaults and environment fallbacks. Matrix sessions and SQLite crypto stores remain completely untouched in their dedicated store.

---

## 2. Settled Decisions & Rulings

| # | Decision | Why | Cost if wrong |
|---|---|---|---|
| **D1** | **Three-tier configuration classification:** Deployment (environment), Runtime Settings (`app_settings` table), Secret Credentials (`provider_secrets` table). | Prevents polluting deployment boundaries while removing operational friction of restarting the daemon for routine API key rotations or prompt/model tweaks. | Clear separation avoids security leaks and unnecessary complexity. |
| **D2** | **Master encryption key remains outside PostgreSQL.** Key material is provided via `SECRETS_ENCRYPTION_KEY` (32 bytes hex/base64) in the process environment or secret volume. | Storing encryption keys alongside ciphertext in the same database provides zero confidentiality against database compromise or backups. | A deployment without `SECRETS_ENCRYPTION_KEY` fails closed for secret management. |
| **D3** | **Authenticated Encryption with AES-256-GCM.** RustCrypto's `aes-gcm` (256-bit key, 96-bit random nonce, 128-bit authentication tag) with provider & secret name bound as Authenticated Additional Data (AAD). | Proven cryptographic standard; AAD binds the ciphertext to its intended context, preventing ciphertext replaying or swapping across providers. | Nonce reuse or missing AAD would compromise confidentiality. |
| **D4** | **Controlled Setting Registry:** Setting keys are strictly typed and registered in `domain/config/registry.rs`. Unknown or arbitrary keys are rejected at validation time. | Prevents `app_settings` from degenerating into a chaotic unindexed junk drawer. Enforces typed validation and documented defaults. | Adding a new setting requires adding an entry to the domain registry. |
| **D5** | **Precedence Order:** `Persisted Runtime Setting > Process Environment Variable > Built-in Code Default`. | Allows admins to override deployment defaults on the fly while maintaining zero-config startup defaults and preserving existing `.env` workflows. | Overrides take effect immediately for hot-reloadable components. |
| **D6** | **Matrix cryptographic sessions remain untouched.** Matrix login tokens, cross-signing keys, and SQLite crypto stores (`data/bot_store/`) remain managed by `matrix-sdk`. | Migrating Matrix device crypto into a generic provider table violates Matrix SDK invariants and hard safety rules. | Low; keeps Matrix crypto isolated and safe. |
| **D7** | **Atomic In-Memory Configuration Snapshot (`ArcSwap<RuntimeConfigSnapshot>`).** Configuration is reloaded atomically into an immutable snapshot. Components read the snapshot without lock contention. | Zero locking overhead on request paths; guarantees callers never observe partially applied configuration updates. | Snapshot generation must be efficient. |
| **D8** | **Hot-Reloadable Registry Only; Restart-Required Settings Stay in `.env`.** Only settings with live hot-reloadable runtime consumers (20 keys) live in `app_settings` / `SettingRegistry`. | Channels with long-lived background sockets (Discord Gateway, Telegram polling, IMAP poller) and `EMBEDDING_MODEL` require a restart or background re-index. Keeping restart-required settings in `.env` avoids misleading Admin UI controls and keeps `domain/config/registry.rs` focused. | Admin Console displays only hot-reloadable runtime settings (`hot_reloadable: true`). |
| **D9** | **Dedicated Administrative Audit Log (`config_audit_log`).** Global configuration changes are tracked separately from per-account security events (`security_events`). | Global config affects the entire instance and multiple tenants. Blending them into per-user security events complicates auditability and account merge semantics. | Separate audit table with retention controls. |
| **D10** | **Controlled CLI / SQL Bootstrap Administrator (`matrix-bot admin grant|revoke <account_id>`).** Public registration and passkey enrollment never promote an account to `admin` automatically. Admin role is granted via CLI or SQL only after the account has enrolled at least one passkey. | Eliminates first-user race conditions and ensures every administrator account is backed by WebAuthn passkey authentication before accessing admin endpoints. | Operator runs `matrix-bot admin grant <account_id>` once after enrolling their passkey. |
| **D11** | **Write-Only Secrets in API and UI.** `GET /api/v1/admin/secrets` returns only metadata (`id`, `provider`, `name`, `status`, `key_version`, `last_tested_at`, `last_test_status`, `updated_at`). Ciphertext and plaintext are never returned. | Complete immunity against browser-side credential exfiltration, XSS credential scraping, and shoulder surfing. | Users cannot view an existing secret value; they can only replace it. |
| **D12** | **Redacted in-memory Secret Types (`SecretValue`).** Wraps secret strings with buffer zeroization on `Drop` and custom `Debug`/`Display` implementations rendering `[REDACTED]`. | Eliminates accidental secret exposure in `tracing` logs, backtraces, panic outputs, and format strings. | Developers must call `.expose_secret()` explicitly when passing credentials to HTTP client builders. |

---

## 3. Configuration Inventory & Classification

Every variable in the workspace is assigned to exactly one category:

### 3.1 Deployment & Restart-Required Configuration (Environment Only)

Loaded during application boot via `Config::from_env()`. Immutable at runtime. Never persisted in PostgreSQL settings tables.

| Variable | Type | Purpose & Default |
|---|---|---|
| `DATABASE_URL` | String | PostgreSQL 18 connection URL (`postgres://...`). **Required.** |
| `DATABASE_MAX_CONNECTIONS` | u32 | Max pooled database connections. Default: `10`. |
| `JOB_WORKERS` | usize | Concurrent job queue worker tasks (1..=32). Default: `4`. |
| `API_HOST` | String | HTTP server bind address. Default: `0.0.0.0`. |
| `API_PORT` | u16 | HTTP server port. Default: `8080`. |
| `SECRETS_ENCRYPTION_KEY` | String | **New.** 256-bit AES master key in hex (64 chars) or base64. Fails closed if missing for secret operations. |
| `SECRETS_ENCRYPTION_KEYS` | String | **New.** Optional versioned keys (`1:<key1>,2:<key2>`) for key rotation. |
| `INVITE_SECRET` / `API_REGISTRATION_SECRET` | String | Secret required to register a new account when registration is closed. |
| `REGISTRATION_OPEN` / `ALLOW_OPEN_REGISTRATION` | bool | Whether public registration is enabled without an invite secret. |
| `WEBAUTHN_RP_ID` | String | Relying Party ID (bare hostname, e.g., `localhost` or `bot.example.com`). Baked into passkeys. |
| `WEBAUTHN_ORIGIN` | String | Full browser origin with scheme & port (`http://localhost:5173`). CSRF checked. |
| `COOKIE_SECURE` / `ENVIRONMENT` | String/bool | Cookie `Secure` attribute flag. Inferred from `ENVIRONMENT=production`. |
| `CORS_ALLOWED_ORIGINS` | List | Cross-origin access list. Empty by default (same-origin). |
| `TRUST_FORWARDED_FOR` | bool | Whether reverse-proxy headers (`X-Forwarded-For`) are trusted from private peers. |
| `STORAGE_DRIVER` | Enum | Object storage backend (`local` or `s3`). |
| `STORAGE_LOCAL_DIR` | String | Local file storage root (`data/storage`). |
| `STORAGE_SIGNING_SECRET` | String | HMAC secret for presigned media URLs. |
| `AI_ALLOW_PRIVATE_BASE_URL` | bool | SSRF protection: whether AI endpoints may point to private/loopback IP addresses. |
| `MATRIX_HOMESERVER` | String | Matrix homeserver URL (`https://matrix.org`). |
| `MATRIX_USERNAME` | String | Bot Matrix user ID. |
| `MATRIX_PASSWORD` | String | Bot Matrix account password (deployment credential for bot login). |
| `MATRIX_RECOVERY_KEY` | String | Matrix security recovery key for E2EE device verification. |
| `MATRIX_VERIFICATION_ALLOWLIST` | List | MXIDs permitted to auto-verify with the bot. |
| `EMBEDDING_MODEL` | String | Knowledge embedding model (`multilingual-e5-small`); changing it re-tags and re-embeds chunks in the background. |
| `DISCORD_TOKEN`, `DISCORD_GUILD_ID` | String/u64 | Discord adapter boot configuration (restart required). |
| `TELEGRAM_BOT_TOKEN`, `TELEGRAM_MODE`, `TELEGRAM_WEBHOOK_SECRET` | String | Telegram adapter boot configuration (restart required). |
| `EMAIL_IMAP_URL`, `EMAIL_SMTP_URL`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`, `EMAIL_FROM`, `EMAIL_AUTHSERV_ID`, `EMAIL_POLL_SECONDS`, `EMAIL_PROCESSED_FOLDER` | String/u64 | Email adapter boot configuration (restart required). |

### 3.2 Runtime Application Settings (`app_settings` — 20 Hot-Reloadable Keys)

Non-secret hot-reloadable configuration managed via Admin API / UI. Persisted in PostgreSQL. Validated against domain schemas. Documented defaults and environment fallbacks.

| Setting Identifier (`key`) | Value Type | Default | Hot Reload? | Description |
|---|---|---|---|---|
| `llm.default_provider` | String | `"openai"` | Yes | Active default LLM provider integration (`openai`, `anthropic`, `xai`, `deepseek`, `ollama`, `custom`). |
| `llm.default_model` | String | `"gpt-4o-mini"` | Yes | Default model name passed in inference requests. |
| `llm.request_timeout_seconds` | Integer | `60` | Yes | HTTP client timeout for LLM calls (5–300 seconds). |
| `llm.default_temperature` | Float | `0.7` | Yes | Default sampling temperature (0.0–2.0). |
| `llm.default_max_tokens` | Integer | `2048` | Yes | Max completion tokens (64–8192). |
| `search.tavily_enabled` | Boolean | `true` | Yes | Whether Tavily web search is used when configured. |
| `search.brave_enabled` | Boolean | `false` | Yes | Whether Brave web search is used when configured. |
| `search.timeout_seconds` | Integer | `15` | Yes | Web search request timeout (3–60 seconds). |
| `inference.ocr.enabled` | Boolean | `true` | Yes | Enable/disable OCR image text recognition. |
| `inference.ocr.base_url` | String | `""` | Yes | Base URL for OCR inference service. |
| `inference.stt.enabled` | Boolean | `true` | Yes | Enable/disable speech-to-text audio transcription. |
| `inference.stt.base_url` | String | `""` | Yes | Base URL for STT inference service. |
| `inference.stt.model` | String | `"sensevoice-small"` | Yes | STT transcription model name. |
| `inference.stt.language` | String | `"auto"` | Yes | Default language hint for speech recognition. |
| `inference.tts.enabled` | Boolean | `true` | Yes | Enable/disable `!say` text-to-speech audio synthesis. |
| `inference.tts.base_url` | String | `""` | Yes | Base URL for TTS inference service. |
| `inference.tts.model` | String | `"kokoro"` | Yes | Text-to-speech model name. |
| `inference.tts.voice` | String | `"0"` | Yes | House default TTS speaker voice. |
| `inference.tts.dialect` | String | `"openai"` | Yes | Dialect format (`openai` or `elevenlabs`). |
| `inference.embedding.base_url`| String | `""` | Yes | Base URL for knowledge embedding inference. |
| `registration.allow_link_signup`| Boolean | `true` | Yes | Allow allowlisted chat identities to create accounts via `/link`. |

### 3.3 Secret Credentials (`provider_secrets` — 6 Hot-Reloadable Providers)

Sensitive hot-reloadable credentials managed via Admin API / UI. Encrypted at rest. Never returned over read APIs. Restart-required credentials (`DISCORD_TOKEN`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_WEBHOOK_SECRET`, `EMAIL_PASSWORD`, `S3_SECRET_KEY`) remain strictly in `.env`.

| Provider (`provider`) | Slot / Name | Payload Type | Description |
|---|---|---|---|
| `openai` / `anthropic` | `"default"` | API Key | LLM provider bearer token or API key. |
| `tavily` | `"default"` | API Key | Tavily Search API key. |
| `stt` | `"default"` | API Key | Hosted speech-to-text API token (optional for local ONNX). |
| `tts` | `"default"` | API Key | Hosted text-to-speech API token (OpenAI or ElevenLabs). |
| `generic` | `<custom_slot>` | API Key | Custom LLM provider or Brave search (`brave`) API key. |

---

## 4. Target Architecture & Layer Alignment

The configuration capability integrates seamlessly into the established layer-first architecture without new external dependencies or separate microservices:

```text
 React Admin UI (Settings, Secrets, Audit)
         │
         │ Authenticated HTTP + Origin CSRF + Step-Up Passkey (VerifiedUser + AdminUser)
         ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ interface/http/admin/                                        │
 │   settings.rs · secrets.rs · status.rs · audit.rs            │
 └──────────────────────┬───────────────────────────────────────┘
                        │ Result<_, ApiError>
                        ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ application/config/ & application/secrets/                   │
 │   AppConfigService: get, update, reset, export_status        │
 │   SecretService: store, delete, test_connection, get_meta    │
 │   Publishes SystemEvent::ConfigUpdated via Outbox in Tx      │
 └───────────────┬──────────────────────────────┬───────────────┘
                 │                              │
                 ▼                              ▼
 ┌──────────────────────────────┐ ┌─────────────────────────────┐
 │ domain/config/ & secrets/    │ │ infrastructure/crypto/      │
 │   SettingKey · SettingValue  │ │   AesGcmSecretProtector     │
 │   SecretId · SecretMetadata  │ │   MasterKeyProvider (env)   │
 │   Validation & Invariants    │ │   Encrypt / Decrypt (AAD)   │
 └───────────────┬──────────────┘ └─────────────┬───────────────┘
                 │                              │
                 ▼                              ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ infrastructure/postgres/repositories/                        │
 │   app_settings.rs · provider_secrets.rs · config_audit.rs    │
 └──────────────────────────────┬───────────────────────────────┘
                                │ SQLx query! (PostgreSQL 18)
                                ▼
                       ┌──────────────────┐
                       │   PostgreSQL 18  │
                       │   app_settings   │
                       │ provider_secrets │
                       │ config_audit_log │
                       │  accounts (role) │
                       └──────────────────┘
```

### Layer Rules Enforced

1. **`domain/`:** Pure, synchronous rules. Owns `SettingKey`, `SettingValue`, `SecretMetadata`, `SecretId`, `ConfigAuditEntry`, and domain validation logic. Zero imports of Axum, SQLx, Reqwest, or AES ciphers.
2. **`application/`:** Coordinates use cases, transaction boundaries, audits, outbox event publishing, and snapshot cache refreshes. `AppConfigService` and `SecretService` return `Result<_, AppError>`.
3. **`infrastructure/`:**
   - `infrastructure/postgres/repositories/`: Plain asynchronous SQLx repository functions over `impl PgExecutor<'_>`.
   - `infrastructure/crypto/`: Implements AES-256-GCM authenticated encryption, nonce generation, and master key resolution.
   - `infrastructure/inference/` & integrations: Implements provider validation calls (`test_connection`).
4. **`interface/http/`:** Handlers parse DTOs, validate request shapes, authenticate via `AdminUser` (which enforces `role == Admin` + recent passkey verification), invoke application services, and format response DTOs. Never runs SQL or touches encryption keys.
5. **Composition Root (`app.rs`):** Instantiates `SecretProtector`, constructs repositories, initializes `AppConfigService` and `SecretService`, builds the initial `RuntimeConfigSnapshot`, and registers them into `AppServices`.

---

## 5. Database Schema & Migrations

A single forward-only migration: `backend/migrations/0015_configuration_and_secrets.sql`.

### 5.1 Migration DDL

```sql
-- 0015_configuration_and_secrets.sql: Admin settings, encrypted secrets, audit log, and account roles.

-- 1. Account Roles: Add role column to accounts table
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'user';
ALTER TABLE accounts ADD CONSTRAINT accounts_role_check CHECK (role IN ('user', 'admin'));
CREATE INDEX IF NOT EXISTS idx_accounts_role ON accounts(role);

-- 2. Runtime Application Settings
CREATE TABLE IF NOT EXISTS app_settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_by  UUID REFERENCES accounts(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_app_settings_updated_at ON app_settings(updated_at);

-- 3. Encrypted Provider Secrets
CREATE TABLE IF NOT EXISTS provider_secrets (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider          TEXT NOT NULL,
    name              TEXT NOT NULL DEFAULT 'default',
    secret_ciphertext BYTEA NOT NULL,
    nonce             BYTEA NOT NULL,
    key_version       INT NOT NULL DEFAULT 1,
    status            TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'revoked')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_by        UUID REFERENCES accounts(id) ON DELETE SET NULL,
    last_tested_at    TIMESTAMPTZ,
    last_test_status  TEXT CHECK (last_test_status IN ('healthy', 'unhealthy', 'failed', 'unknown')),
    last_test_error   TEXT,
    metadata          JSONB,
    CONSTRAINT uq_provider_secrets_slot UNIQUE (provider, name)
);

CREATE INDEX IF NOT EXISTS idx_provider_secrets_provider ON provider_secrets(provider);

-- 4. Dedicated Configuration Audit Log
CREATE TABLE IF NOT EXISTS config_audit_log (
    id          BIGSERIAL PRIMARY KEY,
    actor_id    UUID REFERENCES accounts(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('setting', 'secret', 'system')),
    target_key  TEXT NOT NULL,
    outcome     TEXT NOT NULL CHECK (outcome IN ('success', 'failure')),
    detail      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_config_audit_log_created_at ON config_audit_log(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_config_audit_log_target ON config_audit_log(target_type, target_key);
```

### 5.2 Ownership & Account Merge Invariants

- **`OWNED_TABLES`:** Neither `app_settings` nor `provider_secrets` is added to `OWNED_TABLES`, because they are instance-wide system configuration tables, not per-user domain assets.
- **Account Merge Remapping (`infrastructure/postgres/merge.rs`):** `config_audit_log.actor_id`, `app_settings.updated_by`, and `provider_secrets.updated_by` are remapped to the surviving account when an account is merged, preserving audit trail integrity without row duplication.

---

## 6. Secret Encryption & Key Management

### 6.1 Cryptographic Architecture

- **Algorithm:** AES-256-GCM (Galois/Counter Mode) via RustCrypto `aes-gcm = "0.10"`.
- **Master Key:** 256-bit symmetric key (32 bytes) configured via `SECRETS_ENCRYPTION_KEY` environment variable (encoded as 64-character hexadecimal or standard base64).
- **Nonce:** 96-bit (12 bytes) cryptographically secure random nonce generated per encryption operation via `rand::thread_rng()`. Nonce reuse is mathematically prevented.
- **Authenticated Additional Data (AAD):** Bound to `format!("{provider}:{name}:{key_version}")`. Any attempt to swap ciphertext between different providers (e.g., using a Discord token in place of an OpenAI key) or alter the key version causes authenticated decryption to fail.
- **Ciphertext Storage:** Stored in PostgreSQL as raw bytes (`BYTEA`), keeping ciphertext and nonce separated in dedicated columns.
- **Fail-Closed Security:** If `SECRETS_ENCRYPTION_KEY` is not configured, any attempt to encrypt or decrypt secrets immediately returns `AppError::External { service: "crypto", ... }`. Plaintext storage is impossible.

### 6.2 Key Rotation Architecture

1. `provider_secrets.key_version` tracks the key version used for each row.
2. The `MasterKeyProvider` supports a primary key version and a map of historical versions (e.g., `SECRETS_ENCRYPTION_KEYS="1:<hex>,2:<hex>"`).
3. Secret reads use the key corresponding to `row.key_version`.
4. An administrative maintenance routine can re-encrypt all stored secrets with the newest key version in a single atomic transaction without service interruption.

### 6.3 Memory Safety & Redaction

```rust
/// Redacted secret payload in memory.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(secret: String) -> Self {
        Self(secret)
    }

    /// Explicit opt-in to expose plaintext secret to authorized outbound clients.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl std::fmt::Display for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        // Zeroize memory buffer on drop
        unsafe {
            std::ptr::write_bytes(self.0.as_mut_ptr(), 0, self.0.capacity());
        }
    }
}
```

---

## 7. Domain Layer Design

Pure domain types in `backend/src/domain/config/` and `backend/src/domain/secrets/`:

### 7.1 Config Domain (`domain/config/`)

- `SettingKey`: Strongly-typed enum representing supported keys (`LlmDefaultProvider`, `LlmDefaultModel`, `SearchTavilyEnabled`, etc.), parsing to/from string identifiers.
- `SettingValue`: Strongly-typed JSON-serializable value variants (`String`, `Integer(i64)`, `Float(f64)`, `Boolean(bool)`).
- `SettingDefinition`: Defines key, value type, default value, validation function, and hot-reload capability.
- `SettingRegistry`: Catalog of all recognized settings. Rejects unknown keys with `ConfigError::UnknownKey`.

### 7.2 Secrets Domain (`domain/secrets/`)

- `SecretId(Uuid)`: Strongly typed newtype identifier.
- `SecretProvider`: Enum of supported hot-reloadable integrations (`OpenAi`, `Anthropic`, `Tavily`, `Stt`, `Tts`, `Generic`).
- `SecretStatus`: Lifecycle enum (`Active`, `Disabled`, `Revoked`).
- `SecretMetadata`: Public representation returning ID, provider, slot name, status, key version, masked preview (e.g., `sk-...4a1b`), validation status, and timestamps.
- `EncryptedSecret`: Domain value object containing ciphertext, nonce, and key version.

### 7.3 Audit Domain (`domain/config/audit.rs`)

- `ConfigAuditAction`: Enum (`SettingUpdated`, `SettingReset`, `SecretCreated`, `SecretUpdated`, `SecretDeleted`, `SecretTested`, `ConfigReloaded`).
- `ConfigAuditTarget`: Enum (`Setting(String)`, `Secret(Uuid)`).
- `ConfigAuditEntry`: Fully sanitized audit record. Enforces invariant that no secret ciphertext or plaintext is included.

---

## 8. Application Layer Design

### 8.1 Services

1. **`AppConfigService` (`application/config/mod.rs`):**
   - `get_setting(&self, key: SettingKey) -> Result<SettingValue, AppError>`
   - `update_setting(&self, actor: &AccountId, key: SettingKey, value: SettingValue) -> Result<(), AppError>`
   - `reset_setting(&self, actor: &AccountId, key: SettingKey) -> Result<(), AppError>`
   - `get_status(&self) -> Result<ConfigurationStatusDto, AppError>`
   - Manages atomic configuration snapshot reload.
   - Emits `SystemEvent::ConfigUpdated` in the same database transaction.

2. **`SecretService` (`application/secrets/mod.rs`):**
   - `list_metadata(&self) -> Result<Vec<SecretMetadata>, AppError>`
   - `store_secret(&self, actor: &AccountId, provider: SecretProvider, name: &str, secret: SecretValue, metadata: Option<Value>) -> Result<SecretId, AppError>`
   - `replace_secret(&self, actor: &AccountId, id: SecretId, secret: SecretValue) -> Result<(), AppError>`
   - `delete_secret(&self, actor: &AccountId, id: SecretId) -> Result<(), AppError>`
   - `get_decrypted_secret(&self, provider: SecretProvider, name: &str) -> Result<Option<SecretValue>, AppError>`
   - `test_credential(&self, actor: &AccountId, id: SecretId) -> Result<TestResultDto, AppError>`
   - Emits `SystemEvent::SecretChanged` in the database transaction.

### 8.2 Wiring in `AppServices`

`AppServices` gains:
```rust
pub struct AppServices {
    // ... existing services ...
    pub app_config: AppConfigService,
    pub secrets: SecretService,
}
```

Built in `AppServices::new(pool)` using dependency injection.

---

## 9. Infrastructure Layer Design

### 9.1 PostgreSQL Repositories

- `infrastructure/postgres/repositories/app_settings.rs`:
  - `get_setting(pool, key) -> Result<Option<SettingRow>>`
  - `list_settings(pool) -> Result<Vec<SettingRow>>`
  - `upsert_setting(tx, key, value, updated_by) -> Result<()>`
  - `delete_setting(tx, key) -> Result<bool>`
- `infrastructure/postgres/repositories/provider_secrets.rs`:
  - `list_secret_metadata(pool) -> Result<Vec<SecretMetadataRow>>`
  - `get_secret_by_id(pool, id) -> Result<Option<EncryptedSecretRow>>`
  - `get_secret_by_provider(pool, provider, name) -> Result<Option<EncryptedSecretRow>>`
  - `insert_secret(tx, row) -> Result<Uuid>`
  - `update_secret(tx, id, ciphertext, nonce, key_version, updated_by) -> Result<()>`
  - `update_test_status(pool, id, status, error, tested_at) -> Result<()>`
  - `delete_secret(tx, id) -> Result<bool>`
- `infrastructure/postgres/repositories/config_audit.rs`:
  - `insert_audit_entry(tx, entry) -> Result<()>`
  - `list_audit_entries(pool, limit, offset) -> Result<Vec<ConfigAuditRow>>`

### 9.2 Cryptographic Infrastructure

`infrastructure/crypto/aes_gcm.rs`:
- Implements `SecretProtector` trait / struct.
- Exposes `encrypt(plaintext: &SecretValue, aad: &[u8]) -> Result<EncryptedSecret, CryptoError>`
- Exposes `decrypt(encrypted: &EncryptedSecret, aad: &[u8]) -> Result<SecretValue, CryptoError>`

### 9.3 Provider Credential Testers

`infrastructure/inference/testers/`:
Modular test routines that verify external credentials without side effects:
- `test_llm_credential(endpoint: &str, api_key: &str) -> Result<(), String>`: Submits a minimalist request (`models` or ping).
- `test_tavily_credential(api_key: &str) -> Result<(), String>`: Executes a lightweight query validation probe.

---

## 10. Admin REST API Specification

All administrative endpoints are placed under `/api/v1/admin/` and registered in `interface/http/routes.rs`.

### 10.1 Authentication & Authorization Guards: `AdminSession` & `AdminUser`

Two dedicated Axum extractors in `interface/http/auth/extractor.rs`:
- **`AdminSession`** (used by ordinary admin `GET` endpoints):
  1. Valid session cookie or bearer token resolved to `AuthUser`.
  2. Rejects MCP machine tokens (`mb_mcp_…`) and recovery-restricted sessions.
  3. Verifies `user.role == "admin"` in `accounts` and that the account has at least one enrolled passkey (`has_passkey`).
- **`AdminUser`** (used by all admin write operations `PUT`/`POST`/`DELETE` and `POST /api/v1/admin/secrets/:id/test`):
  1. Enforces all `AdminSession` checks above.
  2. Requires fresh WebAuthn passkey step-up within the last 5 minutes (`STEP_UP_WINDOW_MINUTES = 5`), refusing stale sessions with `403 Forbidden ("Confirm with your passkey to continue")`.
  3. Origin CSRF check is verified for mutating methods (`POST`, `PUT`, `DELETE`, `PATCH`).

All admin write and secret-testing routes are registered in the `SENSITIVE` routes list in `interface/http/mod.rs` for automated step-up test enforcement.

### 10.2 Endpoint Catalog

| Method | Path | Request Body | Response Body | Description |
|---|---|---|---|---|
| `GET` | `/api/v1/admin/config-status` | — | `ConfigStatusDto` | Consolidated overview of runtime configuration, active providers, and credential statuses. |
| `GET` | `/api/v1/admin/settings` | — | `Vec<SettingItemDto>` | List all supported runtime settings with effective values, sources, and definitions. |
| `GET` | `/api/v1/admin/settings/:key` | — | `SettingItemDto` | Read single setting value and metadata. |
| `PUT` | `/api/v1/admin/settings/:key` | `{ "value": <JSON> }` | `SettingItemDto` | Validate and update a runtime setting. Triggers atomic snapshot reload. |
| `DELETE` | `/api/v1/admin/settings/:key` | — | `{ "status": "reset" }` | Reset setting to environment fallback or built-in default. |
| `GET` | `/api/v1/admin/secrets` | — | `Vec<SecretMetadataDto>` | List all configured credential metadata. **Zero secret material returned.** |
| `POST` | `/api/v1/admin/secrets` | `CreateSecretDto` | `SecretMetadataDto` | Store an encrypted credential. Body includes plaintext; response includes only metadata. |
| `PUT` | `/api/v1/admin/secrets/:id` | `UpdateSecretDto` | `SecretMetadataDto` | Replace or update credential secret payload or lifecycle status. |
| `DELETE` | `/api/v1/admin/secrets/:id` | — | `{ "status": "deleted" }` | Delete credential. |
| `POST` | `/api/v1/admin/secrets/:id/test` | — | `SecretTestResultDto` | Test connectivity with the external provider. Updates `last_tested_at` and `last_test_status`. |
| `GET` | `/api/v1/admin/audit-logs` | Query: `limit`, `offset` | `AuditLogPageDto` | Paginated audit log of configuration and secret changes. |

### 10.3 OpenAPI Types & Data Transfer Objects

```json
// Example GET /api/v1/admin/settings/llm.default_model response
{
  "key": "llm.default_model",
  "value": "gpt-4o",
  "effective_source": "database",
  "default_value": "gpt-4o-mini",
  "env_value": null,
  "hot_reloadable": true,
  "description": "Default model name passed in inference requests",
  "updated_at": "2026-10-09T18:30:00Z"
}

// Example GET /api/v1/admin/secrets response (Notice: NO plaintext, NO ciphertext)
[
  {
    "id": "7b3b4f62-3117-48f5-934c-d9c9b1425ea3",
    "provider": "openai",
    "name": "default",
    "status": "active",
    "key_version": 1,
    "masked_preview": "sk-...7e2a",
    "last_tested_at": "2026-10-09T18:25:00Z",
    "last_test_status": "healthy",
    "last_test_error": null,
    "created_at": "2026-10-01T12:00:00Z",
    "updated_at": "2026-10-09T18:20:00Z"
  }
]
```

---

## 11. Runtime Application & Dynamic Snapshot Architecture

### 11.1 The Configuration Application Problem

Saving a setting in PostgreSQL and having a running Tokio worker or Axum request observe that change without restarting the process requires careful state management.

Continuous polling of PostgreSQL adds connection pool overhead. On the other hand, complex distributed message buses introduce unnecessary operational complexity.

### 11.2 Atomic Snapshot Solution (`RuntimeConfigSnapshot`)

1. **In-Memory Snapshot (`RuntimeConfigSnapshot`):**
   An immutable struct containing all resolved settings and decrypted active provider secrets.
2. **Atomic Reference (`ArcSwap<RuntimeConfigSnapshot>`):**
   Stored in `AppConfigService`. Reading the configuration is a lock-free, zero-copy pointer read.
3. **Change-Driven Refresh:**
   When an admin updates a setting or replaces a secret via the Admin API:
   - The change is committed in PostgreSQL.
   - `AppConfigService::reload_snapshot()` fetches fresh rows, decrypts secrets, builds a new `RuntimeConfigSnapshot`, and atomically swaps the pointer.
   - A `SystemEvent::ConfigUpdated` is committed to the transactional outbox.
4. **Multi-Process / Cluster Sync:**
   The existing events runtime LISTEN loop receives `outbox_events` and signals local cache refresh. In addition, a bounded 60-second TTL refresh serves as an automatic safety net against missed notifications.

### 11.3 Integration with Consumers

- **LLM Service (`llm/resolved_config.rs`):**
  `ResolvedAiConfig::from_user_config` resolves dynamically:
  `User Config > Runtime Snapshot (app_settings + provider_secrets) > Environment (AiEnvConfig) > Defaults`.
  Model and provider switches take effect on the very next chat turn or agent step.
- **Web Search (`llm/search.rs`):**
  Inspects the active snapshot for `search.tavily_enabled`, `search.brave_enabled`, `search.timeout_seconds`, and decrypted Tavily/Brave API keys before falling back to environment or DuckDuckGo.
- **Inference Clients (STT / TTS / OCR / Embedding Base URL) & Link Signup (`application/identity/link.rs`):**
  Inference clients inspect the snapshot for `inference.ocr.*`, `inference.stt.*`, `inference.tts.*`, and `inference.embedding.base_url`. `LinkService` inspects `registration.allow_link_signup`.
- **Restart-Required Settings (`.env`):**
  Channel adapters (`DISCORD_*`, `TELEGRAM_*`, `EMAIL_*`) and `EMBEDDING_MODEL` remain configured in `.env`.

---

## 12. Frontend Admin Console

A dedicated **Configuration & Admin** module in the React SPA (`frontend/src/modules/admin/`), accessible to users with `role === "admin"`.

### 12.1 Navigation & Design Language

- Complies with [`Design.md`](../../../Design.md): monochrome-first, Geist Sans and Geist Mono, sentence-case headers, visible keyboard focus outlines, `Escape` key closes modals, zero decorative glow/glassmorphism.
- Rendered in `Sidebar.tsx` as `Admin` with a shield icon, conditionally shown when `currentUser.role === 'admin'`.

### 12.2 Module Layout & Tabs

```text
Admin Console
├── 1. Overview & Status   (Service health, effective config, hot-reloadable status summary)
├── 2. General Settings    (Registration policies & link signup toggle)
├── 3. LLM & AI Models     (Default provider, models, temperature, max tokens, timeout)
├── 4. Search & Inference  (Tavily/Brave search, STT, TTS, OCR, Embedding base URL)
├── 5. Credentials Vault   (Encrypted secret metadata table, replace secret modal, test probes)
└── 6. Audit Log           (Paginated configuration & secret change history)
```

### 12.3 Frontend Security Safeguards

1. **No Secret Field Pre-population:** Credential input fields are strictly write-only (`type="password"`) and never display saved secrets. Placeholders display `•••••••••••••••• (Configured)` or masked previews (`sk-...7e2a`).
2. **State Scrubbing:** Input state holding sensitive values is cleared immediately upon successful form submission or modal cancellation.
3. **No Sensitive Persistence:** Plaintext secret values are never stored in `localStorage`, `sessionStorage`, client-side analytics, or query parameters.
4. **Explicit Confirmation Modal:** Resetting a setting or deleting a credential requires keyboard-accessible confirmation.

---

## 13. Phased Implementation Roadmap

The refactoring is executed incrementally across 6 phases, maintaining full test suite compliance and zero regressions at each step:

```text
 Phase 1: Inventory & Domain Types ──► Phase 2: Database Migration & Repositories
                                                │
                                                ▼
 Phase 4: Admin API Endpoints ◄────── Phase 3: Cryptography & Secret Service
       │
       ▼
 Phase 5: Consumer Integration ──────► Phase 6: Admin UI & Documentation
```

### Phase 1: Inventory & Domain Types
- Create `domain/config/`: `SettingKey`, `SettingValue`, `SettingDefinition`, `SettingRegistry`.
- Create `domain/secrets/`: `SecretId`, `SecretProvider`, `SecretStatus`, `SecretMetadata`, `EncryptedSecret`.
- Create `domain/config/audit.rs`: `ConfigAuditAction`, `ConfigAuditEntry`.
- Add unit tests validating typed conversions, boundary constraints, and unknown key rejection.

### Phase 2: Database Migration & Repositories
- Write `backend/migrations/0014_configuration_and_secrets.sql`.
- Add `accounts.role` column and bootstrap allowlist checks.
- Implement SQLx repositories:
  - `infrastructure/postgres/repositories/app_settings.rs`
  - `infrastructure/postgres/repositories/provider_secrets.rs`
  - `infrastructure/postgres/repositories/config_audit.rs`
- Add integration tests for repository CRUD, transactions, and unique slot constraints.
- Run `cargo sqlx prepare`.

### Phase 3: Cryptography & Application Services
- Add `aes-gcm = "0.10"` to `backend/Cargo.toml`.
- Implement `infrastructure/crypto/aes_gcm.rs` with nonce generation and AAD binding.
- Implement `application/config/mod.rs` (`AppConfigService`) and `application/secrets/mod.rs` (`SecretService`).
- Wire `AppConfigService` and `SecretService` into `AppServices`.
- Add comprehensive cryptographic tests: encryption/decryption roundtrips, corrupted ciphertext rejection, AAD mismatch rejection, and `SecretValue` redaction.

### Phase 4: Admin REST API Endpoints
- Implement `AdminUser` extractor in `interface/http/auth/extractor.rs`.
- Implement `interface/http/admin/`:
  - `settings.rs` (`list_settings`, `get_setting`, `update_setting`, `reset_setting`)
  - `secrets.rs` (`list_secrets`, `create_secret`, `replace_secret`, `delete_secret`, `test_secret`)
  - `status.rs` (`config_status`)
  - `audit.rs` (`list_audit_logs`)
- Register endpoints in `interface/http/routes.rs` and `SENSITIVE` routes list.
- Regenerate OpenAPI contract: `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`.
- Add API authorization and step-up tests.

### Phase 5: Consumer Integration & Dynamic Reload
- Update `llm/resolved_config.rs` to consult the runtime snapshot.
- Update `llm/search.rs` to read the active Tavily credential and search toggle from the snapshot.
- Update inference and channel configuration consumers to read effective settings.
- Implement provider test connection probes (`OpenAI`, `Tavily`, `Telegram`, `Discord`, `Email`).
- Verify outbox event publishing (`SystemEvent::ConfigUpdated`) and atomic snapshot swap.

### Phase 6: Admin UI, Documentation & Verification
- Implement `frontend/src/modules/admin/` with Overview, Settings, Models, Channels, Secrets Vault, and Audit Log tabs.
- Connect API hooks using TanStack Query and `core/apiClient.ts`.
- Add Playwright E2E tests: `frontend/e2e/admin.spec.ts`.
- Update `.env.example`, `docs/architecture.md`, `docs/api.md`, and `docs/llm.md`.
- Execute all quality gates.

---

## 14. Verification Suite & Quality Gates

Every phase must satisfy the repository's strict verification gates:

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p matrix-bot --lib
cargo test -p matrix-bot --test architecture
cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run build
npm --prefix frontend run test:e2e
```

### Key Security Invariants to Prove in Automated Tests

1. **Step-Up Verification:** Synthetic requests without recent passkey verification receive `403 Forbidden` on all `/api/v1/admin/*` endpoints.
2. **Role Authorization:** Accounts with `role == "user"` receive `403 Forbidden` when attempting any administrative call.
3. **Zero Plaintext Leakage:**
   - Database queries on `provider_secrets` confirm ciphertext bytes only.
   - API serialization of `SecretMetadata` omits secret values and ciphertexts.
   - `tracing` event captures during secret operations assert zero occurrences of secret plaintext.
   - `config_audit_log` assertions verify no secret payloads or credentials in audit details.
4. **AAD Binding:** Modifying provider or slot metadata causes decryption of the ciphertext to fail immediately.
5. **Fail-Closed Guarantee:** Unsetting `SECRETS_ENCRYPTION_KEY` triggers `AppError::External` on secret write or read rather than falling back to unencrypted storage.

---

## 15. Rollback & Disaster Recovery Procedures

1. **Database Rollback:** Forward-only migration strategy. If emergency rollback is required, an explicit down-migration or database snapshot restore (`pg_restore`) is performed per `docs/database.md`.
2. **Encryption Key Recovery:** The `SECRETS_ENCRYPTION_KEY` must be backed up securely in production password/secret vaults. If the master key is lost, stored encrypted credentials cannot be decrypted; however, deployment will boot with fallback environment variables, allowing the administrator to re-enter provider credentials with a new master key.
3. **Environment Variable Precedence Safety Net:** If PostgreSQL settings become corrupt or inaccessible, administrators can wipe the `app_settings` rows; the application immediately and safely falls back to `.env` variables and built-in defaults.

---

## 16. Out of Scope

To prevent scope creep and maintain architectural simplicity (per AGENTS.md §4.3 and specification §14), the following items are explicitly **out of scope**:

- Replacing PostgreSQL with another database or introducing external vector/cache services.
- Migrating all deployment variables into PostgreSQL.
- Integrating external secrets engines (Vault, AWS Secrets Manager, Kubernetes Secrets) as mandatory dependencies.
- Modifying Matrix session persistence or crypto SQLite stores.
- Rewriting the channel adapter architecture or `BotModule` registry.
- Implementing multi-tenant credential sharing before multi-tenant ownership models are established.
