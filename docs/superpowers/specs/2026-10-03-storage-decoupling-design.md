# Sub-project 3 — Storage Decoupling Design

**Status:** Draft for review (2026-10-03).
**Roadmap:** `SPEC.md` Phase 5, §11 (Object Storage Architecture) & §10.2 (Omnichannel Receipt Processing).
**Target Branch:** `feat/storage-decoupling` (stacked on `master`).

---

## 1. Goal

Establish an extensible, decoupled object storage architecture (`StorageService`) in `matrix-bot` for all unstructured binary media (receipt images, audio recordings, OCR scans, attachments, and exports).

```text
inbound:   platform attachment (image/audio) → StorageService::put(&key, bytes)
           → Enqueue durable job: media.process_receipt { storage_key, account_id, conversation }
job:       worker (SKIP LOCKED) → StorageService::get(&key) → OCR + LLM
           → FinanceService::record_expense(..., receipt_storage_key)
           → Outbox event: ReceiptProcessed → Origin Channel Notification
web/api:   GET /api/v1/storage/{key} (authenticated, owner-isolated) or presigned URL
```

Unstructured binary media is never stored in PostgreSQL as `BYTEA` or SQLite blobs. State and metadata remain in PostgreSQL; binaries live in Object Storage.

---

## 2. Settled Decisions

| # | Decision | Why |
|---|---|---|
| D1 | **`StorageService` is a trait with two concrete implementations: `LocalStorage` and `S3Storage`.** | Trait boundary justified by two distinct production implementations (local development vs S3/Cloudflare R2/MinIO production). |
| D2 | **Storage keys are strict domain value objects (`StorageKey`).** Format: `accounts/{account_id}/{category}/{uuid}.{ext}`. | Eliminates directory traversal attacks (`..`, `\`, leading `/`) and enforces multi-tenant owner isolation at the type level. |
| D3 | **Local storage presigned URLs use HMAC-SHA256 query tokens against the bot's Axum endpoint.** | Matches S3's presigned capability without exposing internal filesystem paths or requiring third-party web servers in local dev. |
| D4 | **Media processing is asynchronous via durable jobs (`media.process_receipt`).** | Receipt OCR and LLM reasoning take several seconds; processing asynchronously prevents chat gateway timeouts and provides automatic retry. |
| D5 | **Transactions table stores `receipt_storage_key` (`0009_transaction_receipt_storage_key.sql`).** | Links financial transactions directly to their audit receipt image in object storage. |
| D6 | **Storage retrieval endpoint enforces strict ownership.** | `/api/v1/storage/{key}` verifies that `key.account_id()` equals the authenticated `user.account_id`, preventing cross-account access. |

---

## 3. Architecture & Layers

### 3.1 Domain Layer: `domain/storage/` (Pure)
- No dependencies on I/O, tokio, aws-sdk, or filesystem.
- `StorageCategory`: Enum: `Receipts`, `Audio`, `Attachments`, `Exports`.
- `StorageKey`: Strong value object:
  - Canonical format: `accounts/<account_id>/<category>/<uuid>.<ext>`.
  - Rejects: `..`, `\`, null bytes, leading `/`, trailing `/`, non-whitelisted characters.
  - Length capped at 256 bytes.
  - Exposes `account_id(&self) -> AccountId`.
- `StorageError`: Pure error type (`NotFound`, `InvalidKey`, `AccessDenied`, `Backend`, `Unsupported`).

### 3.2 Infrastructure Layer: `infrastructure/storage/`
- `StorageService` trait:
  ```rust
  #[async_trait::async_trait]
  pub trait StorageService: Send + Sync {
      async fn put(&self, key: &StorageKey, data: bytes::Bytes, content_type: &str) -> Result<(), StorageError>;
      async fn get(&self, key: &StorageKey) -> Result<bytes::Bytes, StorageError>;
      async fn delete(&self, key: &StorageKey) -> Result<(), StorageError>;
      async fn get_presigned_url(&self, key: &StorageKey, ttl: std::time::Duration) -> Result<String, StorageError>;
  }
  ```
- `LocalStorageService`:
  - Configured by `STORAGE_LOCAL_DIR` (defaults to `data/storage`).
  - Canonicalization check: resolved absolute path must start with base storage directory.
  - Atomic writes via temporary staging file in same directory before rename.
  - Generates HMAC-signed presigned URLs: `/api/v1/storage/{key}?sig=...&exp=...`.
- `S3StorageService`:
  - AWS SDK S3 client configured via `S3_ENDPOINT`, `S3_BUCKET`, `S3_REGION`, `S3_ACCESS_KEY`, `S3_SECRET_KEY`, `S3_FORCE_PATH_STYLE`.
  - Native S3 presigned URL generation with specified TTL.

### 3.3 Application Layer: `application/`
- `AppServices` holds `pub storage: Arc<dyn StorageService>`.
- `FinanceService`:
  - `record_expense` accepts optional `receipt_storage_key: Option<StorageKey>`.
  - New method `link_receipt(owner, transaction_id, receipt_storage_key)`.

### 3.4 Job Runtime: `interface/jobs/media.rs`
- Handler for `media.process_receipt`:
  - Claimed via durable queue with heartbeat.
  - Retrieves image bytes via `storage.get(&job.storage_key)`.
  - Invokes OCR inference service.
  - Invokes LLM receipt prompt to extract merchant, total amount, currency, category, date.
  - Executes `FinanceService::record_expense` with `receipt_storage_key`.
  - Publishes `SystemEvent::ReceiptProcessed`.
  - Channel outbox dispatcher sends confirmation back to originating conversation.

### 3.5 Interface Layer: `interface/http/storage.rs`
- `GET /api/v1/storage/*key`:
  - Authenticated via session cookie / passkey.
  - Validates `key.account_id() == auth_user.account_id`.
  - Streams media bytes with appropriate `Content-Type`.
  - Also accepts HMAC token query parameter for signed temporary links.

---

## 4. Database Changes

Migration `0009_transaction_receipt_storage_key.sql`:
```sql
ALTER TABLE transactions ADD COLUMN IF NOT EXISTS receipt_storage_key TEXT;
CREATE INDEX IF NOT EXISTS idx_transactions_receipt_storage_key ON transactions(receipt_storage_key);
```
- Forward-only, idempotent DDL.
- `transactions` is already registered in `OWNED_TABLES` (`domain/accounts/mod.rs:170`), so account merges preserve ownership automatically.

---

## 5. Security & Invariants

1. **Owner Isolation:** Every storage key is prefixed with `accounts/{account_id}/`. Storage APIs refuse access to any key whose account prefix does not match the requester.
2. **Path Traversal Prevention:** Key validation bans all path traversal sequences (`..`, `/`, `\`) before reaching any filesystem or S3 API.
3. **No Unauthenticated Media Exposure:** All stored assets require either active session authentication matching the owner OR an unexpired cryptographic signature.
4. **Content-Type Whitelisting:** Uploads enforce mime-type sniffing or explicit whitelisted mime-types (`image/jpeg`, `image/png`, `image/webp`, `audio/ogg`, `audio/mpeg`). Executable formats are rejected.
5. **Atomic Local Operations:** Local writes stage to a `.tmp` file and rename in-place to avoid corrupt or partial reads.
