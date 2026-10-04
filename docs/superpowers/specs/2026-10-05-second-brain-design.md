# Second Brain Architecture & Knowledge Base Specification

> **Date:** 2026-10-05  
> **Status:** Draft for User Approval  
> **Target Branch:** `feat/second-brain`  
> **Authority:** [`AGENTS.md`](../../../AGENTS.md), [`docs/architecture.md`](../../architecture.md), [`docs/database.md`](../../database.md)

---

## 1. Executive Summary & Vision

This specification defines the **Second Brain** subsystem for `matrix-bot`: a conversational, multi-modal, and privacy-preserving personal knowledge base.

Unlike static note repositories, this system is **active and omnichannel**:
1. **Frictionless Multi-Modal Capture:** Capture thoughts, book quotes, voice memos, and articles in less than 3 seconds via Matrix, Discord, Telegram, Email, Web, or MCP (Cursor/Claude).
2. **Unified Cross-Domain Semantic Search (Hybrid RAG):** PostgreSQL 18 with `pgvector` indexes notes, bookmarks, journal entries, and travel footprints into a unified embedding space with hybrid keyword (BM25) and semantic (cosine similarity) ranking.
3. **Local-First & Privacy-Preserving:** Embeddings are generated on-device by `matrix-bot-inference` (using a compact 384-dimensional ONNX model such as BGE-small or MiniLM) with zero token costs and zero data leakage, while supporting an optional cloud API fallback.
4. **Decoupled Asynchronous Processing:** Note creation is instantaneous (< 10ms); vector generation and indexing execute asynchronously via durable PostgreSQL background jobs (`jobs/`) driven by transactional outbox events (`events/`).

```text
 ┌────────────────────────────────────────────────────────────────────────┐
 │                      OMNICHANNEL INGESTION                             │
 │   Matrix (E2EE) • Discord • Telegram • Email • Web UI • MCP (Cursor)   │
 └──────────────────────────────────┬─────────────────────────────────────┘
                                    │
                                    ▼
 ┌────────────────────────────────────────────────────────────────────────┐
 │                    INBOUND PIPELINE & SMART CAPTURE                    │
 │    • Text Commands (!note, /note) ──► Instant Direct Store             │
 │    • Voice Memos (Telegram/Matrix) ──► STT ──► Auto-Title & Clean      │
 │    • Photo OCR (Book Pages/Quotes) ──► OCR ──► Formatted Note          │
 └──────────────────────────────────┬─────────────────────────────────────┘
                                    │
                                    ▼
 ┌────────────────────────────────────────────────────────────────────────┐
 │                     APPLICATION SERVICE LAYER                          │
 │     NoteService  •  SearchService  •  Transactional Outbox Events      │
 └──────────────────────┬───────────────────────────┬─────────────────────┘
                        │ Commit tx                 │ Outbox event
                        ▼                           ▼
 ┌─────────────────────────────────────┐   ┌──────────────────────────────┐
 │     POSTGRESQL 18 + PGVECTOR        │   │      DURABLE JOB SYSTEM      │
 │  • notes                            │   │  job: knowledge.embed_item   │
 │  • knowledge_embeddings (vector 384)│◄──┤  (calls local ONNX inference)│
 │  • HNSW cosine similarity index     │   └──────────────────────────────┘
 └─────────────────────────────────────┘
```

---

## 2. Settled Decisions (Interview Alignment)

During the `/grill-me` architectural interview on 2026-10-05, the following core decisions were settled:

- **D1 (Core Scope):** Vertical slice delivering multi-modal capture (text, voice STT, photo OCR), a dedicated `notes` entity, and cross-domain semantic hybrid search (`pgvector` + BM25 full-text search across notes, bookmarks, and journals).
- **D2 (Embedding Engine):** Pluggable `EmbeddingProvider` trait with local ONNX inference in `matrix-bot-inference` as default (384 dimensions, e.g. `bge-small-en-v1.5` or `all-MiniLM-L6-v2`), with optional OpenAI-compatible cloud fallback.
- **D3 (Data Modeling):** Dedicated `notes` domain table for user notes, coupled with a centralized `knowledge_embeddings` table indexing text chunks from notes, bookmarks, journal entries, and footprints.
- **D4 (Capture Pipeline):** Fast deterministic text capture (`/note`); voice clips transcribed via STT and cleaned of verbal fillers with auto-title extraction; book/document photos OCR'd into formatted quote notes.
- **D5 (Retrieval Experience):** Dual mode: explicit `/search` returns top ranked matching cards with similarity scores; conversational chat (`/ask` or freeform prompt) synthesizes answers with clickable citations.
- **D6 (Embedding Lifecycle):** Asynchronous durable background jobs (`knowledge.embed_item`) triggered by outbox events, providing sub-millisecond save latency with automatic retry backoff.
- **D7 (Historical Backfill):** Migration `0011_knowledge_base.sql` is fast DDL; on startup, an asynchronous low-priority background job scans and embeds historical items in batches without locking.

---

## 3. Database Schema & Persistence (`0011_knowledge_base.sql`)

PostgreSQL 18 is the single source of truth. We introduce the `vector` extension and two owner-keyed tables:

```sql
-- 1. Enable pgvector extension
CREATE EXTENSION IF NOT EXISTS vector;

-- 2. Dedicated Notes Table
CREATE TABLE notes (
    id              BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    title           TEXT NOT NULL,
    content         TEXT NOT NULL,
    format          TEXT NOT NULL DEFAULT 'markdown'
                    CHECK (format IN ('markdown', 'plain', 'quote')),
    source_channel  TEXT NOT NULL DEFAULT 'web'
                    CHECK (source_channel IN ('matrix', 'discord', 'telegram', 'email', 'web', 'mcp')),
    source_ref      TEXT,          -- message ID, email Message-ID, or origin URL
    tags            TEXT[] NOT NULL DEFAULT '{}',
    is_archived     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notes_account_updated ON notes (account_id, updated_at DESC);
CREATE INDEX idx_notes_tags ON notes USING GIN (tags);
CREATE INDEX idx_notes_search ON notes USING GIN (to_tsvector('english', title || ' ' || content));

-- 3. Centralized Knowledge Vector Index
-- Spans notes, bookmarks, journal entries, and footprints for unified semantic retrieval
CREATE TABLE knowledge_embeddings (
    id              BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    item_type       TEXT NOT NULL
                    CHECK (item_type IN ('note', 'bookmark', 'journal', 'footprint')),
    item_id         BIGINT NOT NULL,
    chunk_index     INTEGER NOT NULL DEFAULT 0,
    content_snippet TEXT NOT NULL,
    embedding       vector(384) NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_knowledge_chunk UNIQUE (account_id, item_type, item_id, chunk_index)
);

CREATE INDEX idx_knowledge_embeddings_account ON knowledge_embeddings (account_id, item_type);

-- HNSW Vector Index for fast approximate nearest neighbor search via cosine distance
CREATE INDEX idx_knowledge_embeddings_vector 
    ON knowledge_embeddings USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);
```

### Table Classifications (Invariants)
- `notes` is added to `OWNED_TABLES` in `domain/accounts/mod.rs` (account merges re-key `account_id`).
- `knowledge_embeddings` is added to `ACCOUNT_ID_FOLLOWS` (regenerated or moved on account merge).

---

## 4. Domain Layer (`domain/knowledge/`)

Pure business logic and invariants; synchronous and decoupled from web, database, and inference libraries.

### Types & Entities
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteFormat {
    Markdown,
    Plain,
    Quote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeItemType {
    Note,
    Bookmark,
    Journal,
    Footprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub account_id: AccountId,
    pub title: String,
    pub content: String,
    pub format: NoteFormat,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub is_archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Business Rules & Invariants
1. **Title Invariant:** A note's title is capped at 200 characters and cannot be empty. If omitted during capture, it is synthesized from the first 50 characters of content or generated by the capture summarizer.
2. **Tag Invariant:** Tags are normalized to lowercase alphanumeric plus hyphens/underscores (`^[a-z0-9_-]+$`), stripped of leading `#`, and deduplicated.
3. **Search Invariant:** Similarity scores are normalized between `0.0` (orthogonal/opposite) and `1.0` (identical) derived from cosine distance: `score = 1.0 - (distance / 2.0)`. Results with score < 0.60 are pruned from conversational RAG context.

---

## 5. Inference & Embedding Infrastructure

### 5.1 Local ONNX Model in `matrix-bot-inference`
- **Model:** `bge-small-en-v1.5` or `all-MiniLM-L6-v2` (ONNX format, ~130MB).
- **Endpoint:** `POST /v1/embeddings` hosted on the existing inference server.
- **Contract:** Accepts `{ "input": ["text chunk 1", "text chunk 2"], "model": "bge-small" }` and returns `{ "data": [{ "embedding": [f32; 384], "index": 0 }] }`.
- **L2 Normalization:** Vectors are unit-normalized prior to storage to accelerate cosine distance calculations.

### 5.2 Backend Embedding Client (`infrastructure/inference/embedding.rs`)
```rust
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, InferenceError>;
    fn dimension(&self) -> usize { 384 }
}

pub struct LocalOnnxEmbeddingClient {
    client: reqwest::Client,
    base_url: String,
}

pub struct CloudEmbeddingClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}
```

---

## 6. Application Layer (`application/knowledge/`)

Coordinates use cases, database transactions, and outbox event emissions.

### 6.1 `NoteService`
```rust
pub struct NoteService {
    pool: PgPool,
}

impl NoteService {
    pub async fn create_note(&self, cmd: CreateNoteCommand) -> Result<Note, AppError>;
    pub async fn update_note(&self, cmd: UpdateNoteCommand) -> Result<Note, AppError>;
    pub async fn archive_note(&self, account_id: AccountId, id: NoteId) -> Result<(), AppError>;
    pub async fn delete_note(&self, account_id: AccountId, id: NoteId) -> Result<(), AppError>;
    pub async fn list_notes(&self, query: ListNotesQuery) -> Result<Vec<Note>, AppError>;
    pub async fn get_note(&self, account_id: AccountId, id: NoteId) -> Result<Note, AppError>;
}
```

- **Transactional Outbox Emission:** Every mutation writes `SystemEvent::NoteCreated`, `NoteUpdated`, or `NoteDeleted` inside the transaction.
- **Job Enqueueing:** The state commit notifies the background worker to execute `knowledge.embed_item`.

### 6.2 `SearchService` (Hybrid Search Engine)
```rust
pub struct SearchService {
    pool: PgPool,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl SearchService {
    /// Executes hybrid retrieval combining pgvector cosine distance and PostgreSQL tsvector BM25.
    pub async fn search(
        &self,
        account_id: AccountId,
        query: &str,
        filter_types: Option<Vec<KnowledgeItemType>>,
        limit: usize,
    ) -> Result<Vec<SearchResultItem>, AppError>;
}
```

#### SQL Hybrid Query
```sql
WITH vector_matches AS (
    SELECT 
        item_type,
        item_id,
        content_snippet,
        (1.0 - (embedding <=> $3)) AS vector_score
    FROM knowledge_embeddings
    WHERE account_id = $1
      AND ($2::text[] IS NULL OR item_type = ANY($2))
    ORDER BY embedding <=> $3 ASC
    LIMIT $4 * 2
),
text_matches AS (
    SELECT
        'note' AS item_type,
        id AS item_id,
        ts_rank(to_tsvector('english', title || ' ' || content), plainto_tsquery('english', $5)) AS text_score
    FROM notes
    WHERE account_id = $1
      AND to_tsvector('english', title || ' ' || content) @@ plainto_tsquery('english', $5)
    LIMIT $4 * 2
)
SELECT 
    v.item_type,
    v.item_id,
    v.content_snippet,
    COALESCE(v.vector_score, 0.0) * 0.7 + COALESCE(t.text_score, 0.0) * 0.3 AS combined_score
FROM vector_matches v
LEFT JOIN text_matches t 
       ON v.item_type = t.item_type AND v.item_id = t.item_id
ORDER BY combined_score DESC
LIMIT $4;
```

---

## 7. Background Jobs (`interface/jobs/knowledge.rs`)

### 7.1 `knowledge.embed_item`
- **Payload:** `{ "account_id": "...", "item_type": "note", "item_id": 42 }`
- **Dedupe Key:** `<item_type>:<item_id>`
- **Execution:**
  1. Fetches item text from the corresponding repository (`notes`, `bookmarks`, `life_day_logs`, or `footprints`).
  2. Breaks text into chunks (target 500 chars with 100-char overlap).
  3. Calls `embedder.embed(&chunks)`.
  4. Upserts records into `knowledge_embeddings` within a single transaction.

### 7.2 `knowledge.backfill_embeddings` (Startup Sweep)
- Enqueued as a single-instance low-priority job on boot with advisory lock.
- Scans items missing from `knowledge_embeddings` in batches of 50.
- Respects worker timeouts, pausing between batches to yield resources to live requests.

---

## 8. Multi-Modal Capture Pipeline

```text
Incoming Chat Message (DM or Channel)
  │
  ├── 1. Text Command: "!note Ideas for memory pool" or "/note #architecture Model..."
  │      └── InboundPipeline: Parses command, extracts tags, creates Note immediately.
  │
  ├── 2. Voice Message: User speaks into Telegram/Matrix voice memo
  │      └── Adapter receives audio ──► Local STT (SenseVoice) ──► Clean transcript
  │      └── LLM auto-formats: Title + bullet points + tags ──► Note created.
  │
  └── 3. Image Upload: Photo of a book page or whiteboard
         └── Adapter receives image ──► Local OCR (PaddleOCR) ──► Extract text
         └── Note created with format = 'quote', tagged #reading.
```

---

## 9. Interface Layer Specifications

### 9.1 REST & OpenAPI Contracts
- `POST /api/v1/notes` — Create note (body: `title`, `content`, `tags`, `format`).
- `GET /api/v1/notes` — List notes with pagination and `#tag` filtering.
- `GET /api/v1/notes/:id` — Retrieve note by ID.
- `PUT /api/v1/notes/:id` — Update note.
- `DELETE /api/v1/notes/:id` — Archive or delete note.
- `POST /api/v1/knowledge/search` — Hybrid search endpoint (body: `query`, `types`, `limit`).

### 9.2 Chat Commands across All Channels
- `!note add <title> | <content>` or `/note <content>` — Quick-capture note.
- `!note list [tag]` or `/notes` — Display 5 most recent notes.
- `!search <query>` or `/search <query>` — Execute hybrid search across Second Brain, returning top 3 matching cards with relevance scores.
- Conversational `/ask <question>` — LLM tool call invokes `knowledge_search`, producing a natural synthesized answer with citations.

### 9.3 Unified MCP Tools (Claude Desktop & Cursor)
- `knowledge_search`: Semantic and full-text search across personal notes, bookmarks, and journals.
- `knowledge_create_note`: Allows Cursor or Claude to save architectural decisions and design notes directly into the Second Brain.
- `knowledge_list_tags`: Returns frequently used tags and knowledge clusters.

---

## 10. Web Console SPA Module (`frontend/src/modules/knowledge/`)

A new first-class module registered in `frontend/src/core/registry.ts`:
- **Navigation:** Added to `Sidebar.tsx` as **"Second Brain"** (`Brain` icon).
- **Views:**
  - **Notes Tab:** Split view with quick-capture markdown editor and search-as-you-type list.
  - **Search & Discovery Tab:** Unified search input with chips filtering domain sources (`All`, `Notes`, `Bookmarks`, `Journal`, `Places`).
  - **Tag Explorer:** Sidebar chip list showing counts per tag.
- **Performance:** Lazy loaded via `React.lazy()` with `<React.Suspense>` fallback, preserving the `< 500 kB` initial bundle size.

---

## 11. Verification & Quality Gates

Each task implementing this specification must strictly pass:

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets                  # warning-free
cargo clippy --workspace --all-targets -- -D warnings    # zero warnings
cargo test -p matrix-bot --lib
cargo test -p matrix-bot --test architecture             # 35+ architecture boundary tests
cargo test -p matrix-bot live_postgres -- --ignored
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run build                          # bundle size < 500 kB
npm --prefix frontend run test:e2e                       # 100% green
```
