# Receipt Preservation & Lightbox Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A receipt scanned or uploaded on the web is kept in object storage, linked to its transaction, and viewable from the ledger.

**Architecture:** The DB column, repository, `link_receipt` and the owner/HMAC `GET /storage/*key` route already exist. Add the missing service rule (`detach_receipt`, key ownership check), three REST endpoints, the DTO field, and a lightbox in the finance module.

**Tech Stack:** Rust/Axum/SQLx, utoipa OpenAPI, React 19 + React Query.

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` — Blueprint 5 (Phase B).

## Global Constraints

- Presigned receipt URLs live 300 seconds (`expires_in_seconds: 300`).
- Upload whitelist: `image/jpeg`, `image/png`, `image/webp`, `application/pdf`; max 10 MiB; type decided from magic bytes, not the client's header.
- A receipt key must be `StorageCategory::Receipts` under the **caller's own** account; anything else → validation error (400).
- Cross-account / unknown transaction → 404, never 403 (no ownership oracle). No receipt linked → 404.
- Every mutation publishes `SystemEvent::TransactionUpdated` in its transaction.
- Rulings: (1) `receipt_storage_key` check lives in `FinanceService` so every caller (chat, jobs, REST) gets it; (2) upload stores first and OCRs best-effort — a disabled/failed OCR still returns the key with `extracted: null`; (3) PDFs are stored but not OCR'd (the OCR service takes images); (4) no new `ReceiptAttached/Detached` events — `TransactionUpdated` carries the row.

## Review Focus

- Create a transaction with another account's receipt key → 400, no row.
- Create with a non-receipt key (`audio/…`) → 400.
- Upload of a non-image/non-PDF body (e.g. text with `.png` name) → 400; oversized → 413/400, nothing stored.
- Detach on a transaction with no receipt → 404.
- Deleting a transaction does not leave the receipt link dangling in the DTO (row is gone).

## Task 1: Service rules

**Files:** Modify `backend/src/application/finance/transactions.rs`, `backend/src/domain/finance/mod.rs` (error variant), tests in `backend/src/application/finance/tests.rs`.

**Interfaces:**
- Produces: `FinanceError::InvalidReceiptKey` (→ `AppError::Validation`); `FinanceService::detach_receipt(&self, owner: &AccountId, transaction_id: i64) -> Result<Transaction, AppError>` (`TransactionNotFound` if the row is absent **or** has no receipt).

- [ ] Failing tests: `create_rejects_foreign_or_non_receipt_key`, `link_receipt_rejects_foreign_key`, `detach_receipt_clears_key_and_publishes_update`, `detach_receipt_without_receipt_is_not_found`, `detach_receipt_is_owner_scoped`.
- [ ] Add pure `fn receipt_key_ok(owner, key) -> bool` in domain finance (unit-tested) and call it in `record_transaction_in_tx` and `link_receipt`.
- [ ] Implement `detach_receipt` with the existing `repo::set_receipt_storage_key(…, None)`, reading the row first with a lock-free owner-scoped get; publish `TransactionUpdated`.
- [ ] `cargo test -p matrix-bot finance::` passes; commit `feat: validate and detach transaction receipts`.

## Task 2: REST endpoints

**Files:** Modify `backend/src/interface/http/finance.rs` (DTO field, `TransactionRequest.receipt_storage_key`), `backend/src/interface/http/ocr.rs` (extract shared `run_ocr`), create `backend/src/interface/http/finance_receipts.rs`, modify `routes.rs`, `openapi.rs`.

**Interfaces:**
- Produces: `Transaction.receipt_storage_key: Option<String>`; `POST /api/v1/finance/receipts/upload` (multipart `file`) → `ReceiptUploadResponse { receipt_storage_key: String, extracted: Option<ReceiptExtractionDto> }`; `GET /api/v1/finance/transactions/{id}/receipt-url` → `ReceiptUrlResponse { url: String, expires_in_seconds: i64 }`; `DELETE /api/v1/finance/transactions/{id}/receipt` → `Transaction`.
- Consumes: Task 1.

- [ ] Failing tests (in `finance_receipts.rs`): `sniff_receipt_type_accepts_jpeg_png_webp_pdf`, `sniff_receipt_type_rejects_text_and_empty`; API-level tests with the module's existing `ApiState` test helper: `receipt_url_404_without_receipt`, `receipt_url_404_for_other_owner`, `upload_rejects_text_payload`.
- [ ] Implement `fn sniff_receipt_type(&[u8]) -> Option<(&'static str /*ext*/, &'static str /*mime*/)>` (JPEG `FF D8 FF`, PNG signature, `RIFF….WEBP`, `%PDF-`).
- [ ] Upload: read multipart `file` capped at 10 MiB, sniff, `StorageKey::generate(owner, Receipts, ext)`, `storage.put`, best-effort OCR for images (reuse `ocr.rs` logic), return key.
- [ ] receipt-url: `get_transaction` (owner-scoped) → key → `storage.get_presigned_url(&key, 300s)`.
- [ ] Register routes, add to `openapi.rs`; `cargo run --bin generate-openapi`; `npm --prefix frontend run generate-api`.
- [ ] Tests pass; commit `feat: receipt upload, presigned view and detach endpoints`.

## Task 3: Frontend

**Files:** Modify `frontend/src/modules/finance/types.ts`, `components/ReceiptDropzone.tsx`, `components/AddTransactionModal.tsx`, `components/TransactionList.tsx`, `frontend/e2e/fake-api.ts`; create `components/ReceiptLightboxModal.tsx`, `frontend/e2e/finance-receipt.spec.ts`.

**Interfaces:**
- Consumes: Task 2 endpoints and generated types.

- [ ] Dropzone posts to `/finance/receipts/upload` (falls back to showing OCR error but keeps key when present) and passes `receiptStorageKey` to `onReceiptScanned`; the modal sends it in the create body.
- [ ] `TransactionList` shows a paperclip button (`aria-label="View receipt"`) when `receipt_storage_key` is set.
- [ ] `ReceiptLightboxModal`: fetches receipt-url, shows image (or PDF link), zoom, rotate 90°, download, remove receipt (confirm), `Escape` closes; Design.md styling.
- [ ] Mock routes in `fake-api.ts`; Playwright spec: upload → create → paperclip → lightbox opens → Escape closes.
- [ ] `npm --prefix frontend run build` and `test:e2e -- finance-receipt` pass; commit `feat: receipt lightbox in finance ledger`.

## Task 4: Gates and hand-off

- [ ] `cargo fmt --all`, `cargo check`, `cargo clippy -D warnings`, `cargo test -p matrix-bot --all-targets`, openapi drift, `git diff --check`.
- [ ] Add Phase B / Blueprint 5 rulings and status to `docs/superpowers/HANDOFF.md`; commit `docs: record receipt viewer progress`.
