# Sub-project 3, Storage Decoupling — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Implement task-by-task with TDD.

**Status:** approved 2026-10-03.
**Branch:** `feat/storage-decoupling` (stacked on `master`).
**Spec:** `docs/superpowers/specs/2026-10-03-storage-decoupling-design.md`.

---

## Architecture & Responsibilities

- `domain/storage/`: Pure `StorageKey`, `StorageCategory`, `StorageError`. Zero I/O, zero tokio, zero aws-sdk. Validates path safety, format, length, owner extraction.
- `infrastructure/storage/`:
  - `StorageService` trait: `put`, `get`, `delete`, `get_presigned_url`.
  - `LocalStorageService`: Writes to `STORAGE_LOCAL_DIR` (defaults to `data/storage`), atomic writes, path traversal prevention, HMAC-SHA256 presigned URLs.
  - `S3StorageService`: AWS S3 / Cloudflare R2 / MinIO compatible using `aws-sdk-s3`.
- `backend/migrations/0009_transaction_receipt_storage_key.sql`: Adds `receipt_storage_key TEXT` to `transactions`.
- `application/finance/`: Links transactions to `receipt_storage_key`.
- `application/mod.rs`: Injects `storage: Arc<dyn StorageService>` into `AppServices`.
- `interface/jobs/media.rs`: Durable job handler `media.process_receipt` (downloads receipt $\rightarrow$ OCR $\rightarrow$ LLM structured extraction $\rightarrow$ records expense $\rightarrow$ outbox confirmation).
- `interface/http/storage.rs`: Authenticated media streaming & presigned URL verification.
- `interface/channels/matrix/attachments.rs`: Inbound receipt photo uploads to storage $\rightarrow$ enqueues `media.process_receipt`.

---

## Tasks

### Task 1: Domain Storage Types (`domain/storage/`)
- [x] Create `backend/src/domain/storage/mod.rs`
- [x] Define `StorageCategory` enum (`Receipts`, `Audio`, `Attachments`, `Exports`)
- [x] Define `StorageKey` value object:
  - Canonical format: `accounts/{account_id}/{category}/{uuid}.{ext}`
  - Path traversal checks: reject `..`, `\`, leading/trailing `/`, characters outside `[a-zA-Z0-9_.-]`
  - Maximum key length: 256 bytes
  - Method `account_id(&self) -> AccountId`
  - Method `generate(owner: &AccountId, category: StorageCategory, ext: &str) -> Self`
- [x] Define pure `StorageError` enum (`NotFound`, `InvalidKey`, `AccessDenied`, `Backend`, `Unsupported`)
- [x] Unit tests covering path traversal attacks (`../`, `..\\`, absolute paths, null bytes), formatting, and owner extraction
- [x] Declare `pub mod storage;` in `backend/src/domain/mod.rs`
- [x] Verify: `cargo test -p matrix-bot --lib domain::storage`

### Task 2: Storage Infrastructure & Local Provider (`infrastructure/storage/`)
- [x] Create `backend/src/infrastructure/storage/mod.rs`
- [x] Define `StorageService` trait:
  ```rust
  #[async_trait::async_trait]
  pub trait StorageService: Send + Sync {
      async fn put(&self, key: &StorageKey, data: bytes::Bytes, content_type: &str) -> Result<(), StorageError>;
      async fn get(&self, key: &StorageKey) -> Result<bytes::Bytes, StorageError>;
      async fn delete(&self, key: &StorageKey) -> Result<(), StorageError>;
      async fn get_presigned_url(&self, key: &StorageKey, ttl: std::time::Duration) -> Result<String, StorageError>;
  }
  ```
- [x] Implement `StorageConfig` supporting `STORAGE_DRIVER` (`local` | `s3`), `STORAGE_LOCAL_DIR`, `STORAGE_SIGNING_SECRET`, etc.
- [x] Create `backend/src/infrastructure/storage/local.rs`:
  - `LocalStorageService`: safe canonicalization within `STORAGE_LOCAL_DIR`
  - Atomic file writes using staging `.tmp` files
  - HMAC-SHA256 presigned URL generation
- [x] Declare `pub mod storage;` in `backend/src/infrastructure/mod.rs`
- [x] Unit tests for `LocalStorageService`: round-trip `put`/`get`/`delete`, directory creation, atomic overwrite, traversal attempts
- [x] Verify: `cargo test -p matrix-bot --lib infrastructure::storage`

### Task 3: S3 Storage Provider (`infrastructure/storage/s3.rs`)
- [x] Add `aws-sdk-s3 = { version = "1", default-features = false, features = ["rustls"] }` and `aws-config = { version = "1", default-features = false, features = ["rustls"] }` to `backend/Cargo.toml`
- [x] Create `backend/src/infrastructure/storage/s3.rs`:
  - `S3StorageService`: client initialization, presigned client
  - Implements `StorageService` trait
- [x] Unit tests with mocked/configured builder
- [x] Verify: `cargo check -p matrix-bot --all-targets`

### Task 4: Database Migration & Finance Repository (`0009` & `repositories/finance.rs`)
- [x] Add migration `backend/migrations/0009_transaction_receipt_storage_key.sql`:
  - `ALTER TABLE transactions ADD COLUMN IF NOT EXISTS receipt_storage_key TEXT;`
  - `CREATE INDEX IF NOT EXISTS idx_transactions_receipt_storage_key ON transactions(receipt_storage_key);`
- [x] Update `backend/src/infrastructure/postgres/repositories/finance.rs` to persist and fetch `receipt_storage_key`
- [x] Update `TransactionRow` and domain conversion
- [x] Run `cargo sqlx prepare --check` against scratch database
- [x] Verify repository tests pass

### Task 5: Application Layer Wiring (`application/finance/` & `AppServices`)
- [x] Update `FinanceService`:
  - `record_expense` accepts optional `receipt_storage_key: Option<StorageKey>`
  - Add `link_receipt` method to attach a receipt to an existing transaction
- [x] Add `storage: Arc<dyn StorageService>` to `AppServices`
- [x] Build storage in `app.rs` composition root based on `Config`
- [x] Verify: `cargo test -p matrix-bot --lib application::finance`

### Task 6: Durable Background Job (`interface/jobs/media.rs`)
- [x] Define `media.process_receipt` job payload:
  `ProcessReceiptJob { storage_key: StorageKey, account_id: AccountId, conversation: ConversationAddress, caption: Option<String> }`
- [x] Create `backend/src/interface/jobs/media.rs`:
  - Handler downloads bytes from `storage.get(&storage_key)`
  - Calls OCR client to extract text
  - Synthesizes LLM structured receipt prompt
  - Invokes `FinanceService::record_expense` with `receipt_storage_key`
  - Publishes `SystemEvent::ReceiptProcessed` (or appropriate event)
  - Notifies origin conversation via outbox
- [x] Wire job into `backend/src/jobs/` dispatcher
- [x] Tests for job execution and failure recovery

### Task 7: HTTP Storage Endpoint (`interface/http/storage.rs`)
- [x] Create `backend/src/interface/http/storage.rs`:
  - `GET /api/v1/storage/*key`:
    - Validates caller session (`AuthUser`) owns the key (`key.account_id() == user.account_id`)
    - OR validates unexpired HMAC token
    - Streams media bytes with appropriate `Content-Type`
- [x] Register routes in `routes.rs` and OpenAPI in `openapi.rs`
- [x] Verify byte-identical OpenAPI or intentional additions
- [x] Unit & integration tests for authorization (cross-account read forbidden)

### Task 8: Inbound Chat Channel Attachment Ingestion
- [x] Update `backend/src/interface/channels/matrix/attachments.rs`:
  - When image attachment is received, save to storage with `StorageCategory::Receipts`
  - Enqueue `media.process_receipt` job
  - Send acknowledgment reply to user
- [x] Add tests for receipt upload ingestion flow

### Task 9: Architecture Boundary Tests & Documentation
- [x] Add architecture test: `storage_crates_stay_inside_the_storage_infrastructure` (assert `aws_sdk_s3` never appears outside `infrastructure/storage/`)
- [x] Update `docs/architecture.md`, `README.md`, `.env.example`
- [x] Update `HANDOFF.md` with status and rulings
- [x] Full verification gate pass
