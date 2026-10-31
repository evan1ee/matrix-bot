# Knowledge Embedding Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop bookmark, footprint and journal deletions leaking vector embeddings, and make footprint and journal writes enqueue embedding jobs.

**Architecture:** Each service already owns a write transaction that publishes an outbox event. We add `delete_embeddings_for_item` inside the delete transactions and `knowledge_embed_item_versioned` job enqueues inside the create/update transactions. No schema change, no new API. Read-side orphan filtering already exists in retrieval (verify, do not rebuild).

**Tech Stack:** Rust, SQLx (compile-checked macros, `.sqlx/` cache), PostgreSQL 18 + pgvector.

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` — Blueprint 8 (the unified `GET /search/hybrid` REST route, `!search` command and frontend badges are out of scope here; see Scope note).

**Scope note:** Blueprint 8's HTTP/chat/UI search surface already exists as `application/knowledge/retrieval.rs` and `interface/chat/knowledge.rs` (all four item types are handled). Task 4 only verifies that, it adds no new surface.

## Global Constraints

- Embedding deletion MUST happen in the same transaction as the parent row deletion.
- Versioned job keys (`NewJob::knowledge_embed_item_versioned`) for anything that can be edited; `NewJob::knowledge_embed_item` only for create-once.
- Item type strings: `"bookmark"`, `"footprint"`, `"journal"` (`KnowledgeItemType::as_str`).
- No SQL in `application/`; repositories are plain fns over `impl sqlx::PgExecutor`.
- After changing a `sqlx::query!`, regenerate `.sqlx/` against a migrated scratch DB (test Postgres on :55433) and commit it.

## Review Focus

- Deleting a footprint/bookmark/day log owned by another account leaves the victim's embeddings untouched (owner-scoped delete).
- Deleting a nonexistent item returns NotFound / `false` and does not touch embeddings.
- Failure after the embedding delete but before commit rolls both back (row and embeddings both survive).
- Upserting the same day log twice yields two distinct versioned job keys (edit re-embeds); an identical repeat is deduplicated.
- `upsert_day_log` on a brand-new date enqueues one job whose `item_id` is the new row id.

## Task 1: Bookmark delete removes embeddings

**Files:**
- Modify: `backend/src/application/bookmarks/mod.rs` (`delete`, tests module)

**Interfaces:**
- Consumes: `repositories::knowledge::delete_embeddings_for_item(executor, &AccountId, &str, i64) -> Result<u64?, sqlx::Error>` (check exact return type at `repositories/knowledge/search.rs:54`).
- Produces: no new API.

- [ ] **Step 1: Failing test** `delete_removes_embeddings_in_same_transaction` in the existing tests module: create a bookmark, insert an embedding row via `repositories::knowledge::upsert_embedding_chunk(.., "bookmark", id, 0, "snippet", &embedding, "test-model")`, call `service.delete`, assert `COUNT(*) FROM knowledge_embeddings WHERE owner_id=$1 AND item_type='bookmark' AND item_id=$2` (via a repository helper already used in `knowledge/search.rs` tests, or `count_chunks`-style fn there) is 0. Add `delete_other_owners_bookmark_keeps_embeddings`: bob's `delete` of alice's id returns NotFound and alice's embedding remains.
- [ ] **Step 2:** `cargo test -p matrix-bot bookmarks::tests::delete_removes` → FAIL (1 chunk remains).
- [ ] **Step 3:** In `BookmarkService::delete`, after the `!deleted` guard and before `outbox::publish`, call `repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, KnowledgeItemType::Bookmark.as_str(), id).await?;`.
- [ ] **Step 4:** Run both tests → PASS.
- [ ] **Step 5:** `git commit -m "fix: delete bookmark embeddings with the bookmark"`

## Task 2: Footprint embeddings on create/update/delete

**Files:**
- Modify: `backend/src/application/footprints/mod.rs` (`create`, `update`, `delete`, new tests module if absent)

**Interfaces:**
- Consumes: `NewJob::knowledge_embed_item_versioned(owner: AccountId, KnowledgeItemType, i64, &str)`, `repositories::jobs::enqueue(&mut *tx, &NewJob)`.
- Produces: none.

- [ ] **Step 1: Failing tests** (`FootprintService::new(pool, SettingsService::new(pool.clone()))` — mirror existing setup in other service tests):
  - `create_enqueues_versioned_embed_job`: after `create`, exactly one pending job of type `knowledge.embed_item` whose dedupe key is `footprint:{id}:{updated_at.to_rfc3339()}`.
  - `update_enqueues_new_version_job`: after `update` of title, two jobs with different keys.
  - `delete_removes_embeddings`: seed a `"footprint"` embedding chunk, `delete`, assert 0 remain; a failed delete (unknown id → NotFound) leaves an unrelated owner's chunk intact.
- [ ] **Step 2:** `cargo test -p matrix-bot footprints::` → FAIL.
- [ ] **Step 3:** Add a private `fn embed_job(owner: &AccountId, fp: &Footprint) -> NewJob` using the version string `fp.updated_at.to_rfc3339()`; enqueue it after `repositories::footprints::create` and `update` inside their transactions; in `delete`, call `delete_embeddings_for_item(.., "footprint", id)` before publish.
- [ ] **Step 4:** Tests → PASS.
- [ ] **Step 5:** `git commit -m "fix: footprint embedding jobs and cleanup on delete"`

## Task 3: Journal (day log) embeddings

**Files:**
- Modify: `backend/src/infrastructure/postgres/repositories/life.rs` (`delete_day_log` → `Result<Option<i64>, sqlx::Error>` using `DELETE … RETURNING id`)
- Modify: `backend/src/application/life/mod.rs` (`upsert_day_log`, `delete_day_log`)
- Modify: all callers of `life_repo::delete_day_log` (grep); `.sqlx/` cache

**Interfaces:**
- Produces: `life_repo::delete_day_log(conn, &AccountId, &str) -> Result<Option<i64>, sqlx::Error>`; service signature `delete_day_log -> Result<bool, AppError>` unchanged.

- [ ] **Step 1: Failing tests:** repository `delete_day_log_returns_id_and_is_owner_scoped` (alice's log id returned for alice; bob gets `None`); service `upsert_day_log_enqueues_versioned_job` (key `journal:{id}:{updated_at}`; second upsert after a changed mood gives a second job); `delete_day_log_removes_embeddings`.
- [ ] **Step 2:** `cargo test -p matrix-bot life` → FAIL (compile error then assertion).
- [ ] **Step 3:** Change the repo query to `DELETE FROM life_day_logs WHERE owner_id=$1 AND log_date=$2 RETURNING id` with `fetch_optional`. In the service enqueue `NewJob::knowledge_embed_item_versioned(*owner_id, KnowledgeItemType::Journal, log.id, &log.updated_at)` after upsert; in delete, when `Some(id)`, call `delete_embeddings_for_item(.., "journal", id)` before the event.
- [ ] **Step 4:** Regenerate `.sqlx/`: `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` against the migrated scratch DB. Run tests → PASS.
- [ ] **Step 5:** `git commit -m "fix: journal embedding jobs and cleanup on delete"`

## Task 4: Verify read-side orphan handling and gates

**Files:**
- Test: `backend/src/application/knowledge/retrieval.rs` (only if no orphan test exists)

- [ ] **Step 1:** Grep retrieval for an existing test that a hit whose item row is gone is dropped (`retrieval.rs:529` area suggests one for bookmarks). If missing for footprint/journal, add `orphaned_footprint_and_journal_hits_are_filtered`.
- [ ] **Step 2:** Run gates: `cargo fmt --all -- --check`, `cargo check -p matrix-bot --all-targets`, `cargo clippy -p matrix-bot --all-targets -- -D warnings`, `cargo test -p matrix-bot --all-targets --test-threads=2` (needs `TEST_DATABASE_URL`), `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets`.
- [ ] **Step 3:** Update `docs/superpowers/HANDOFF.md` (status row + ruling "Embedding cleanup is in-transaction; no sweep job") and commit `docs: record knowledge embedding lifecycle`.
