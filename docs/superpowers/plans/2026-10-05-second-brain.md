# Second Brain & Knowledge Base — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Implement task-by-task with strict TDD: failing test $\rightarrow$ minimal code $\rightarrow$ green $\rightarrow$ refactor $\rightarrow$ verify gates. **Do not begin implementation until user gives explicit instruction.**

**Status:** Plan ready for review  
**Target Branch:** `feat/second-brain` (branched from `master`)  
**Parent Spec:** [`docs/superpowers/specs/2026-10-05-second-brain-design.md`](../specs/2026-10-05-second-brain-design.md)  
**Authority:** [`AGENTS.md`](../../../AGENTS.md), [`docs/architecture.md`](../../architecture.md), [`docs/database.md`](../../database.md)

---

## Architecture & Layering Overview

```text
backend/src/
  domain/knowledge/                  Pure entities: Note, NoteId, KnowledgeChunk, SearchResult
  infrastructure/
    postgres/repositories/knowledge.rs Compile-checked sqlx::query! over notes & knowledge_embeddings
    inference/embedding.rs           EmbeddingProvider trait, LocalOnnxClient, CloudClient
  application/knowledge/             NoteService, SearchService (tx boundaries + outbox events)
  interface/
    http/knowledge.rs                Axum REST handlers + DTOs (OpenAPI contracts)
    chat/knowledge.rs                KnowledgeModule: BotModule (!note, /note, !search)
    jobs/knowledge.rs                Durable jobs: knowledge.embed_item, knowledge.backfill_embeddings
  llm/tools/knowledge.rs             MCP & AI domain tools: knowledge_search, knowledge_create_note
inference/src/
  embeddings/                        POST /v1/embeddings ONNX server (bge-small / MiniLM, 384-dim)
frontend/src/
  modules/knowledge/                 React 19 SPA view: notes list, markdown editor, hybrid search
```

---

## Tasks

### Task 1: Domain Knowledge & Note Models (`domain/knowledge/`)
- [x] Create directory `backend/src/domain/knowledge/` and modules:
  - `note.rs`: `Note`, `NoteId`, `NoteFormat` (`Markdown`, `Plain`, `Quote`)
  - `search.rs`: `KnowledgeItemType` (`Note`, `Bookmark`, `Journal`, `Footprint`), `KnowledgeChunk`, `SearchResultItem`, `SimilarityScore`
  - `error.rs`: Pure `KnowledgeError` enum (`InvalidTitle`, `InvalidContent`, `InvalidTag`, `NotFound`)
- [x] Implement domain invariants:
  - Title validation: non-empty, trimmed, maximum 200 characters
  - Tag normalization: stripped of `#`, lowercase alphanumeric plus hyphens (`^[a-z0-9_-]+$`), deduplicated
  - Similarity score normalization: clamped between `0.0` and `1.0` derived from cosine distance: `1.0 - (distance / 2.0)`
- [x] Unit tests for domain types and invariants in `domain/knowledge/note.rs` and `search.rs`
- [x] Register `pub mod knowledge;` in `backend/src/domain/mod.rs`
- [x] Verify: `cargo test -p matrix-bot --lib domain::knowledge`

---

### Task 2: Database Migration `0011_knowledge_base.sql` & Repository
- [x] Create migration file `backend/migrations/0011_knowledge_base.sql`:
  - `CREATE EXTENSION IF NOT EXISTS vector;`
  - `CREATE TABLE notes (...)` with `BIGINT` id, `UUID` account_id FK, format check, and GIN indexes on tags and full-text search
  - `CREATE TABLE knowledge_embeddings (...)` with `vector(384)`, unique constraint on `(account_id, item_type, item_id, chunk_index)`
  - HNSW index on `embedding` using `vector_cosine_ops` with `m = 16, ef_construction = 64`
- [x] Update table classifications in `backend/src/domain/accounts/mod.rs`:
  - Add `notes` to `OWNED_TABLES`
  - Add `knowledge_embeddings` to `ACCOUNT_ID_FOLLOWS`
- [x] Create repository `backend/src/infrastructure/postgres/repositories/knowledge.rs`:
  - `insert_note`, `update_note`, `get_note_by_id`, `list_notes`, `archive_note`, `delete_note`
  - `upsert_embedding_chunk`, `delete_embeddings_for_item`, `search_hybrid` (combining HNSW vector distance and `to_tsvector` BM25 score)
- [x] Integration tests in `knowledge.rs` using `test_pool()`
- [x] Run `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` and commit `.sqlx/` cache
- [x] Verify: `cargo test -p matrix-bot --lib infrastructure::postgres::repositories::knowledge`

---

### Task 3: Local & Remote Embedding Provider (`infrastructure/inference/embedding.rs` & `inference`)
- [x] Add ONNX embedding pipeline in `inference/src/embeddings/`:
  - Load `bge-small-en-v1.5` or `all-MiniLM-L6-v2` ONNX model (384 dimensions)
  - Implement tokenization, mean pooling, and L2 normalization
  - Mount endpoint `POST /v1/embeddings` adhering to OpenAI embedding schema
- [x] Create `backend/src/infrastructure/inference/embedding.rs`:
  - Define `EmbeddingProvider` trait:
    ```rust
    #[async_trait::async_trait]
    pub trait EmbeddingProvider: Send + Sync {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, InferenceError>;
        fn dimension(&self) -> usize { 384 }
    }
    ```
  - Implement `LocalOnnxEmbeddingClient` calling `POST /v1/embeddings` on `matrix-bot-inference`
  - Implement `CloudEmbeddingClient` calling OpenAI-compatible embedding API (`AI_EMBEDDING_ENDPOINT`)
- [x] Unit tests with mocked HTTP responses
- [x] Verify: `cargo test -p matrix-bot-inference` and `cargo test -p matrix-bot --lib infrastructure::inference::embedding`

---

### Task 4: Application Services (`application/knowledge/`) & Outbox Events
- [x] Define system events in `backend/src/events/mod.rs`:
  - `NoteCreated { id, account_id, title }`
  - `NoteUpdated { id, account_id, title }`
  - `NoteDeleted { id, account_id }`
- [x] Create `backend/src/application/knowledge/mod.rs`:
  - `NoteService { pool }`:
    - `create_note(cmd)`: opens transaction, writes note, publishes `NoteCreated` event, commits
    - `update_note(cmd)`, `archive_note(id)`, `delete_note(id)`, `list_notes(query)`, `get_note(id)`
  - `SearchService { pool, embedder }`:
    - `search(account_id, query, filter_types, limit)`: embeds query string, executes `repositories::knowledge::search_hybrid`, returns ranked domain items
- [x] Inject `NoteService` and `SearchService` into `AppServices` (`application/mod.rs`)
- [x] Wire service initialization in `backend/src/app.rs`
- [x] Integration tests covering transaction atomicity and outbox emission
- [x] Verify: `cargo test -p matrix-bot --lib application::knowledge`

---

### Task 5: Background Durable Jobs (`interface/jobs/knowledge.rs`)
- [x] Create `backend/src/interface/jobs/knowledge.rs`:
  - Define `knowledge.embed_item` job handler:
    - Payload: `{ "account_id": Uuid, "item_type": KnowledgeItemType, "item_id": i64 }`
    - Deduplication key: `<item_type>:<item_id>`
    - Execution: fetches content, chunks text (500 chars / 100 overlap), computes embeddings, upserts into `knowledge_embeddings`
  - Define `knowledge.backfill_embeddings` startup job handler:
    - Scans unindexed bookmarks, journal entries, and footprints in batches of 50
    - Enqueues `knowledge.embed_item` tasks with low priority
- [x] Register job handlers in `interface/jobs/mod.rs` and dispatch table
- [x] Register background enqueue hook in outbox event dispatcher
- [x] Unit tests for job execution, chunking, and backfill logic
- [x] Verify: `cargo test -p matrix-bot --lib interface::jobs::knowledge`

---

### Task 6: Omnichannel Capture & Chat Commands (`interface/chat/knowledge.rs`)
- [x] Create `backend/src/interface/chat/knowledge.rs` implementing `BotModule`:
  - Command `!note add <title> | <content>` / `/note <content>`: quick note creation
  - Command `!note list [tag]` / `/notes`: displays recent notes
  - Command `!search <query>` / `/search <query>`: executes hybrid search, replies with top 3 cards + relevance scores
- [x] Voice note capture pipeline:
  - In `interface/channels/matrix/attachments.rs` and inbound voice handling:
  - If caption starts with `/note` or sent in DM: transcribe audio via STT $\rightarrow$ LLM formats title and removes filler words $\rightarrow$ calls `NoteService::create_note`
- [x] Photo quote capture pipeline:
  - If image sent with `/note`: OCR extraction $\rightarrow$ format as quote note
- [x] Register `KnowledgeModule` in `interface/channels/matrix/modules.rs` (before AI catch-all)
- [x] Tests for command parsing and formatted reply rendering
- [x] Verify: `cargo test -p matrix-bot --lib interface::chat::knowledge`

---

### Task 7: REST API, OpenAPI Spec & MCP Tools (`interface/http/knowledge.rs`, `llm/tools/knowledge.rs`)
- [x] Create `backend/src/interface/http/knowledge.rs`:
  - `POST /api/v1/notes`: Create note
  - `GET /api/v1/notes`: List notes with `#tag` and archive filters
  - `GET /api/v1/notes/:id`: Get note details
  - `PUT /api/v1/notes/:id`: Update note
  - `DELETE /api/v1/notes/:id`: Delete or archive note
  - `POST /api/v1/knowledge/search`: Hybrid search endpoint
- [x] Register endpoints in `interface/http/routes.rs` under `api_routes()`
- [x] Create MCP tools in `backend/src/llm/tools/knowledge.rs`:
  - `knowledge_search`: Semantic and full-text search across personal notes and bookmarks
  - `knowledge_create_note`: Capture ideas from Claude Desktop / Cursor directly into the Second Brain
  - `knowledge_list_tags`: List tag taxonomy
- [x] Register tools in `llm/tools/definitions.rs`
- [x] Run `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json` to verify zero drift
- [x] Verify: `cargo test -p matrix-bot --lib interface::http::knowledge`

---

### Task 8: Web Console Module (`frontend/src/modules/knowledge/`)
- [x] Create `frontend/src/modules/knowledge/`:
  - `types.ts`: TypeScript interfaces matching backend DTOs
  - `api.ts`: React Query hooks (`useNotes`, `useKnowledgeSearch`, `useCreateNote`, `useUpdateNote`, `useDeleteNote`)
  - `NoteEditorModal.tsx`: Markdown note editor with live preview
  - `KnowledgeSearchPanel.tsx`: Instant search input with source chips (`All`, `Notes`, `Bookmarks`, `Journal`, `Places`)
  - `TagFilterChips.tsx`: Sidebar tag cluster selector
  - `KnowledgeMainView.tsx`: Main view component
- [x] Register module in `frontend/src/core/registry.ts`, `Sidebar.tsx` ("Second Brain"), and `AppShell.tsx`
- [x] Lazy load `KnowledgeMainView` with `React.lazy()` in `App.tsx` to preserve initial bundle target < 500 kB
- [x] Add mock routes to `frontend/e2e/fake-api.ts`
- [x] Create Playwright E2E test suite `frontend/e2e/knowledge.spec.ts`:
  - Create note, edit note, filter by tag, search notes, verify keyboard navigation
- [x] Verify: `npm --prefix frontend run build` and `npm --prefix frontend run test:e2e`

---

### Task 9: Architecture Boundary Enforcement & Documentation
- [x] Add architecture tests in `backend/tests/architecture.rs`:
  - `knowledge_domain_is_pure`: asserts `domain/knowledge` imports only pure crates
  - `knowledge_application_knows_no_transports`: asserts `application/knowledge` imports no web/chat SDKs
  - `knowledge_interface_runs_no_sql`: asserts `interface/http/knowledge.rs` and `interface/chat/knowledge.rs` contain zero SQL
  - Assert all new files in `domain/knowledge/`, `application/knowledge/`, and repositories are < 300 lines
- [x] Update `docs/architecture.md`: add Knowledge Base & Vector RAG subsystem section
- [x] Update `docs/database.md`: document `0011_knowledge_base.sql`, HNSW index, and `knowledge_embeddings` schema
- [x] Update `docs/api.md`: document `/api/v1/notes` and `/api/v1/knowledge/search` endpoints
- [x] Update `docs/superpowers/HANDOFF.md`: record completion state and architectural rulings
- [x] Run full verification suite across backend and frontend

---

## Verification Checklist

Before closing this plan, all gates must be green:

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets                  # warning-free
cargo clippy --workspace --all-targets -- -D warnings    # zero warnings
cargo test -p matrix-bot --lib
cargo test -p matrix-bot --test architecture             # all architecture tests green
cargo test -p matrix-bot live_postgres -- --ignored
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json # zero OpenAPI drift
npm --prefix frontend run build                          # bundle size < 500 kB
npm --prefix frontend run test:e2e                       # 100% green
```
