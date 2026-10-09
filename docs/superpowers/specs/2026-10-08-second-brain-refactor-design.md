# Second Brain Refactor — Design

> **Date:** 2026-10-08
> **Status:** Implemented 2026-10-09 on `feat/notes-refactor` (operator deploy steps pending, see HANDOFF §3b). Plan: [`plans/2026-10-09-notes-refactor.md`](../plans/2026-10-09-notes-refactor.md)
> **Target branch:** `feat/notes-refactor`
> **Supersedes:** the 2026-10-08 "Markdown files as source of truth" proposal (rejected, see §2)
> **Builds on:** [`2026-10-05-second-brain-design.md`](2026-10-05-second-brain-design.md)
> **Authority:** [`AGENTS.md`](../../../AGENTS.md), [`docs/architecture.md`](../../architecture.md), [`docs/database.md`](../../database.md)

---

## 1. Objective

Turn the existing Second Brain into a **Markdown-native knowledge base with derived, rebuildable indexes**, while keeping PostgreSQL as the single source of truth for both content and state.

> **Markdown is the canonical content *format*. PostgreSQL is the single *store*. Every index (FTS, trigram, embeddings, excerpts) is derived from the stored Markdown and can be rebuilt from it.**

This is a refactor, not a rewrite. Notes, search, embeddings, jobs, outbox, capture, MCP, REST and the web console stay usable at every step.

---

## 2. Settled decisions (interview 2026-10-08)

| # | Decision | Why | Cost if wrong |
|---|---|---|---|
| D1 | **PostgreSQL stays the source of truth for content.** `notes.content` stays. No `ContentStore`, no file store, no dual-write, no reconciliation job. | The user edits only in the web app. The one requirement that justifies files as the source of truth (editing in external tools) doesn't apply, and files bring a second store with no shared transaction (orphans, compare-and-swap, merge moving files, a second backup). | If external editing becomes a need later, add a file *mirror* on top of export (§10). The schema already carries `content_hash` for change detection. |
| D2 | **Markdown is the only content format.** `plain` and `quote` collapse into `markdown`. | Quote and OCR captures already store `> ` blockquote Markdown, and plain text is valid Markdown. | Low: blockquote rendering is unchanged. |
| D3 | **Mixed English and Chinese content.** FTS uses the `simple` config plus a `pg_trgm` index. | The `english` config can't tokenize CJK, and the pgvector image has no `zhparser`. | English loses stemming (`running` ≠ `run`). Vector search covers most of that gap. |
| D4 | **The embedding model becomes `multilingual-e5-small` (384-dim)**, tracked per chunk. | `bge-small-en-v1.5` produces near-meaningless vectors for Chinese. Same dimension, so the HNSW index doesn't change. | A model change becomes a re-embed, which the rebuild path (§8) is built to handle. |
| D5 | **Search = lexical list + vector list fused with RRF**, degrading to lexical-only when the embedder fails. | New notes are findable before they're embedded, an embedder outage no longer means a 500, and each item takes one result slot. | The RRF constant needs tuning (k=60 is the usual default). |
| D6 | **`Note` (metadata + excerpt) is split from `NoteDocument` (+ markdown).** Lists and outbox events carry `Note` only. | Today every list response and every `NoteCreated`/`NoteUpdated` outbox row and SSE frame carries the full body. | A deliberate API change (§9). The frontend changes in the same commit. |
| D7 | **Optimistic concurrency on `content_hash`** for the web editor. Chat, MCP and LLM appends stay last-writer-wins. | Stops an LLM edit and an open editor silently overwriting each other. | The editor needs a 409 path ("note changed, reload"). |
| D8 | **In scope:** `note_type`, Markdown export/import. **Deferred:** entities/relations/decisions, RecallService, `brain.*` MCP tools. **Dropped:** `slug`/`path` columns, `metadata JSONB`. | Grow in layers (AGENTS §1). The deferred items have no current consumer. | The deferred items get their own spec, with this refactor's rebuild path as the foundation. |
| D9 | **Export/import is synchronous and web-only, with step-up.** A zip of `.md` files with YAML frontmatter. | Personal scale, matches `!export`'s web-only rule, and needs no new job or storage flow. | Very large vaults hit the caps. Revisit with a job writing to `StorageCategory::Exports`. |
| D10 | **Every user-facing "Second Brain" label becomes "Notes".** Identifiers (`knowledge`, `notes`, `knowledge_*` tools, job names) don't change. | "Second Brain" reads oddly in the UI. "Notes" says what the page holds and matches `!note` and `/notes`. | Copy-only change. Rename again if the page grows beyond notes. |

Rulings decided during the write-up (same authority, recorded per AGENTS §4.1):

- **Ruling R1:** the maximum note body is 100,000 chars, enforced in `domain::knowledge::note::validate_content`. The AGENTS §5.1 500-char cap covers payload *fields*, and note bodies are an explicit exception — without a cap, one request could store an unbounded body and an unbounded embedding fan-out — if wrong, raise the constant.
- **Ruling R2:** Markdown parsing (`pulldown-cmark`) lives in `application/knowledge/markdown.rs`, not `domain/`. The domain may only import `std/chrono/uuid/serde/thiserror` — the cost is that the plain-text extraction isn't "pure domain", but it is still a pure synchronous function with unit tests.
- **Ruling R3:** archived notes are excluded from search by default, and `include_archived` opts in. Archived means "out of the way" — if wrong, flip the default.
- **Ruling R4:** the degraded-search signal is a response header (`X-Search-Mode: hybrid | lexical`), not a body field. The search response is a bare JSON array, and wrapping it would break every client — the cost is that headers are easy to miss, so the frontend shows a subtle notice when `lexical` is returned.
- **Ruling R5:** the column naming stays `owner_id` (not the proposal's `account_id`). Every owned table and `OWNED_TABLES` uses `owner_id` — renaming one table buys nothing.

---

## 3. Current state (what the code does today)

| Area | Today | Problem |
|---|---|---|
| Schema | [`0011_knowledge_base.sql`](../../../backend/migrations/0011_knowledge_base.sql): `notes(content, format ∈ {markdown,plain,quote}, …)`, `idx_notes_search` GIN on `to_tsvector('english', title‖content)` | The index is never used by search. It also indexes raw Markdown syntax. |
| Search | [`search_hybrid`](../../../backend/src/infrastructure/postgres/repositories/knowledge/search.rs): `0.7·cosine + 0.3·ts_rank_cd(to_tsvector(content_snippet))` over `knowledge_embeddings` only | Notes that haven't been embedded are invisible. An embedder failure means a 500. Several chunks of one note fill several result slots. The FTS is computed at query time and unindexed. |
| Embedding | `embed_item` re-chunks `title + raw content` on every create/update/archive | Re-embeds even when nothing changed, embeds Markdown syntax, and uses an English-only model. |
| Domain | [`Note`](../../../backend/src/domain/knowledge/note.rs) carries the full `content`, and `NoteFormat` has three variants | Lists, chat `!note list` and outbox events load and serialize full bodies. |
| HTTP update | `update_note` handler reads the existing note and merges optional fields | Business logic in a handler, and last-writer-wins. |
| Capture | [`interface/chat/knowledge.rs`](../../../backend/src/interface/chat/knowledge.rs): text, quoted, photo OCR (`> ` blockquote), voice STT | Fine. Its output is already Markdown. |
| Storage | [`StorageService`](../../../backend/src/infrastructure/storage/mod.rs) (local/S3) for binary blobs | Unchanged by this refactor. |

---

## 4. Target architecture

```text
 Capture (chat · web · MCP · LLM tools · import)
                 │
                 ▼
          NoteService  (application/knowledge)
   validate → normalize Markdown → derive (hash, plain text, excerpt)
                 │  one transaction
                 ▼
 ┌──────────────────────── PostgreSQL ────────────────────────┐
 │ notes                                                      │
 │   content (Markdown, authoritative)                        │
 │   content_hash · search_text · excerpt   ← derived on write│
 │   search_vector (GENERATED) · trigram index ← derived by DB│
 │   indexed_content_hash · indexed_at       ← embed job state│
 │ outbox (NoteCreated/Updated/Deleted, metadata only)        │
 │ jobs   (knowledge.embed_item, knowledge.backfill_embeddings)│
 └───────────────┬────────────────────────────────────────────┘
                 │ embed job (async)
                 ▼
   search_text → chunk → EmbeddingProvider (e5, "passage: ")
                 │
                 ▼
   knowledge_embeddings (rebuildable, tagged with embedding_model)

 Search: query ─┬─ lexical (notes.search_vector + trigram; chunk FTS for other types)
                └─ vector  (e5 "query: ", current model only, best chunk per item)
                       └──► RRF (pure domain fn) ──► results  [X-Search-Mode]
```

Layer placement follows the Rooms pattern. No new top-level layer, no new trait.

---

## 5. Data model

### 5.1 Migrations (forward-only, each one concern)

**`0012_notes_markdown_only.sql` — guarded data rewrite (separate from DDL, per docs/database.md)**

```sql
-- Guard: name any row whose format is outside the known set before rewriting.
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM notes WHERE format NOT IN ('markdown','plain','quote')) THEN
    RAISE EXCEPTION 'unexpected notes.format values: %',
      (SELECT array_agg(DISTINCT format) FROM notes WHERE format NOT IN ('markdown','plain','quote'));
  END IF;
END $$;
UPDATE notes SET format = 'markdown' WHERE format <> 'markdown';
```

**`0013_notes_derived_columns.sql` — DDL**

```sql
CREATE EXTENSION IF NOT EXISTS pg_trgm;

ALTER TABLE notes DROP CONSTRAINT notes_format_check;
ALTER TABLE notes ADD CONSTRAINT notes_format_check CHECK (format = 'markdown');

ALTER TABLE notes
  ADD COLUMN note_type            TEXT NOT NULL DEFAULT 'note'
        CHECK (note_type IN ('note','idea','fact','observation','decision')),
  ADD COLUMN content_hash         TEXT,          -- hex SHA-256 of content; NOT NULL after backfill (0014)
  ADD COLUMN search_text          TEXT,          -- Markdown-stripped plain text
  ADD COLUMN excerpt              TEXT,          -- first ~200 chars of search_text
  ADD COLUMN indexed_content_hash TEXT,          -- hash the current embeddings were built from
  ADD COLUMN indexed_at           TIMESTAMPTZ,
  ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', title), 'A') ||
        setweight(to_tsvector('simple', COALESCE(search_text, content)), 'B')
  ) STORED;

DROP INDEX idx_notes_search;
CREATE INDEX idx_notes_search_vector ON notes USING GIN (search_vector);
CREATE INDEX idx_notes_search_trgm   ON notes USING GIN ((title || ' ' || COALESCE(search_text, content)) gin_trgm_ops);

ALTER TABLE knowledge_embeddings
  ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'bge-small-en-v1.5';
CREATE INDEX idx_knowledge_embeddings_model ON knowledge_embeddings (embedding_model);
```

`COALESCE(search_text, content)` keeps FTS working on rows the backfill hasn't reached yet.

**`0014_notes_derived_not_null.sql`**: written after the backfill job has run in every environment. It starts with a guard naming the notes where `content_hash IS NULL OR search_text IS NULL OR excerpt IS NULL`, then sets `NOT NULL` on all three and simplifies the generated column and index to `search_text` (which means dropping and re-adding the generated column).

No `slug`, `path` or `metadata` columns (D8). `notes` and `knowledge_embeddings` are already in `OWNED_TABLES`, and no new owned table is added.

### 5.2 Domain (`domain/knowledge/note.rs`)

```rust
pub struct ContentHash(String);                 // hex SHA-256; constructed only by the hashing fn
pub enum NoteType { Note, Idea, Fact, Observation, Decision }

pub struct Note {                               // metadata — what lists, events and chat lists carry
    pub id: NoteId,
    pub owner_id: AccountId,
    pub title: String,
    pub note_type: NoteType,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub excerpt: String,
    pub content_hash: ContentHash,
    pub is_archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub indexed_at: Option<DateTime<Utc>>,
}

pub struct NoteDocument { pub note: Note, pub markdown: String }

pub enum ExpectedHash { Any, Exactly(ContentHash) }   // D7
pub struct NotePatch { title: Option<String>, markdown: Option<String>,
                       tags: Option<Vec<String>>, note_type: Option<NoteType>,
                       expected: ExpectedHash }
```

- `NoteFormat` is deleted. `validate_content` adds the 100k-char cap (R1).
- `KnowledgeError::Conflict` maps to `AppError::Conflict` (409).
- SHA-256 uses the existing `sha2` dependency. The hashing function lives in `application/knowledge` because `sha2` isn't on the domain's allowed list. `ContentHash::from_hex` validates the format.

### 5.3 Markdown processing (`application/knowledge/markdown.rs`, R2)

One pure module, unit-tested in place:

- `to_plain_text(md) -> String`: uses `pulldown-cmark` (a new dependency, no transitive bloat) and keeps text, code-block contents, link text, image alt text and table cells. It drops syntax, HTML tags and URLs, and joins block boundaries with `\n`.
- `excerpt(plain, 200) -> String`: cuts on a char boundary and collapses whitespace.
- `chunks(plain) -> Vec<String>`: reuses `domain::knowledge::search::chunk_text(…, 500, 100)`, but over the *plain text* prefixed with the title.
- Frontmatter render/parse for export/import (§10) also lives here.

Every derivation runs in `NoteService` on write. No caller computes them itself.

---

## 6. Application services

### 6.1 `NoteService` (`application/knowledge/notes.rs`, split out of today's 491-line `mod.rs`)

| Method | Behaviour |
|---|---|
| `create(owner, NewNote)` | validate → derive (hash, search_text, excerpt) → one tx: insert, `outbox::publish(NoteCreated(Note))`, enqueue `knowledge.embed_item` → commit |
| `get(owner, id) -> NoteDocument` | one query that includes `content` |
| `list(owner, filter) -> Vec<Note>` | metadata query only. **Never selects `content`.** |
| `update(owner, id, NotePatch)` | tx: `SELECT … FOR UPDATE` → if `Exactly(h)` and `h ≠ content_hash`, return `Conflict` → merge the patch (the logic moves out of the HTTP handler) → re-derive if the markdown changed → update → publish `NoteUpdated` → enqueue the embed job **only if `content_hash` or `title` changed** → commit |
| `append(owner, id, markdown)` | the chat/LLM path: `FOR UPDATE`, appends with a blank-line separator, `ExpectedHash::Any` |
| `archive` / `delete` | unchanged semantics. `delete` also deletes the embeddings in the same tx (as today) |
| `export(owner)` / `import(owner, files)` | §10 |

### 6.2 Indexing (`application/knowledge/indexing.rs`)

`embed_item(owner, Note, id)`:

1. Load `(title, search_text, content_hash, indexed_content_hash)` for the note.
2. **Skip** if `indexed_content_hash == content_hash` *and* every chunk's `embedding_model == configured model`.
3. Build chunks, prefix each with `"passage: "` (an e5 requirement), embed (outside any tx), then in one tx: delete the old chunks, insert the new ones with `embedding_model`, and set `indexed_content_hash`/`indexed_at` — **but only if `content_hash` still equals the hash read in step 1** (`UPDATE … WHERE content_hash = $read`). If the note changed meanwhile, roll back; the newer job wins.
4. Bookmarks, journal and footprints keep their current text builders, minus the skip check. They have no hash column, so they rely on the model check plus their own triggers.

`backfill_embeddings(batch)` widens its scan to:

- notes where `indexed_content_hash IS DISTINCT FROM content_hash`, and
- **any item with a chunk whose `embedding_model ≠ configured model`** (D4: this is how a model switch re-embeds everything).

It enqueues at priority −1, deduplicated by the existing job unique key.

`backfill_derived_fields(batch)` is a new one-shot path, run by the backfill job before `0014`. It fills `content_hash/search_text/excerpt` for rows where they're NULL, 500 rows per tx. It's idempotent.

### 6.3 Search — `SearchService` (`application/knowledge/search.rs`)

1. Trim the query. Empty → `[]`.
2. **Lexical list** (repository, ≤ 50): notes via `search_vector @@ websearch_to_tsquery('simple', q)` ranked by `ts_rank_cd`, unioned with trigram matches (`word_similarity(q, text) > 0.3`, which is what makes CJK work), plus FTS over `knowledge_embeddings.content_snippet` for the non-note types. Archived items are excluded unless requested (R3).
3. **Vector list** (≤ 50): embed `"query: " + q` with a 3 s timeout. Filter to `embedding_model = configured`. Keep the best chunk per `(item_type, item_id)` (`DISTINCT ON`).
4. **Fuse** with `domain::knowledge::search::rrf(lexical, vector, k = 60)`, a pure function with exact tie-breaks (higher fused score, then the earlier lexical rank, then the lower `item_id`). Each item appears once, and its snippet is the best vector chunk, falling back to `excerpt`.
5. If step 3 fails or times out, return the lexical results only with mode `Lexical`, and log it with `warn!` (operation, error kind, latency).

Return `SearchOutcome { items, mode: SearchMode }`. HTTP maps `mode` to `X-Search-Mode` (R4), the LLM tool adds `"mode"` to its JSON result, and chat appends "(keyword results only)" when the mode is lexical.

Renaming `SearchService` → `RetrievalService` is deferred to the Recall spec, where a second consumer appears.

---

## 7. Events, jobs and the outbox

- `SystemEvent::NoteCreated(Note)` / `NoteUpdated(Note)` now carry the metadata-only `Note`, including `content_hash`. This is enough for any consumer to reload the document by `(owner_id, id)`. `NoteDeleted { id, owner_id }` is unchanged. The names stay as they are (`Memory*` renames are deferred to D8).
- The event wire shape changes (`content`/`format` removed; `note_type`, `excerpt`, `content_hash`, `indexed_at` added). Update the matching `contracts/` fixture if one exists, and check the frontend `modules/knowledge/index.tsx` SSE handler (it only checks `event.type`).
- Jobs: `knowledge.embed_item` (gains the skip logic) and `knowledge.backfill_embeddings` (wider scan, also runs `backfill_derived_fields`). **No** `knowledge.reconcile_content` (D1).

---

## 8. Rebuildability invariant

> Deleting every row of `knowledge_embeddings`, or clearing `search_text/excerpt/content_hash/indexed_*`, must lose no information. The next `knowledge.backfill_embeddings` passes restore them from `notes.content` and the source domain tables.

Verified by a `live_postgres` test: seed notes → embed → `DELETE FROM knowledge_embeddings` and `UPDATE notes SET indexed_content_hash = NULL` → run the backfill and job handlers with `MockEmbeddingProvider` → assert the chunks are identical to the first run. A second test switches the configured model name and asserts that every chunk is re-tagged.

---

## 9. Interface changes

### 9.1 REST (`interface/http/knowledge.rs`)

| Endpoint | Change |
|---|---|
| `GET /notes` | Items become `NoteSummary { id, owner_id, title, note_type, excerpt, source_channel, source_ref, tags, content_hash, is_archived, created_at, updated_at, indexed_at }`. **`content` and `format` are removed.** |
| `GET /notes/:id` | Returns `Note` = the summary fields + `content` + `format` (always `"markdown"`, kept so the contract stays stable for single-note readers). |
| `POST /notes` | `format` is accepted and ignored. `note_type` is optional. Returns `Note`. |
| `PUT /notes/:id` | Adds optional `expected_content_hash` and `note_type`. Returns 409 `{"error":"note changed since it was loaded","code":409}` on a mismatch. The handler just maps the DTO to `NotePatch`. |
| `POST /knowledge/search` | The body array is unchanged. Adds the `X-Search-Mode` header and an optional `include_archived`. |
| `GET /notes/export` | New. `VerifiedUser` + a `SENSITIVE` entry. Streams `application/zip`. |
| `POST /notes/import` | New. `VerifiedUser` + `SENSITIVE`, multipart zip, returns `{ created, updated, skipped: [{file, reason}] }`. |

This deliberately changes `backend/openapi.json`. Regenerate and commit it in the same change as the frontend update.

### 9.2 Chat (`interface/chat/knowledge.rs`)

- `!note list` uses `excerpt` (no more `content` slicing in the interface).
- Capture paths are unchanged in behaviour: `!note`, `!note add t | c`, quoted, photo OCR and voice STT all call `services.notes.create`. Voice can pass `note_type` later; for now it isn't auto-classified.
- `!note search` shows the lexical-mode notice.
- **Relabel (D10):** the command description ("Create or manage your notes") and the `!note help` header ("📝 <b>Notes:</b>") drop "Second Brain".

### 9.3 LLM tools and MCP (`llm/tools/knowledge.rs`)

- `knowledge_create_note` gains an optional `note_type` enum. `knowledge_search` returns `mode`.
- New `knowledge_get_note(id)` returns the full Markdown, so the model can answer from the authoritative document rather than a 500-char snippet (the minimal step toward Recall). It is read-only and owner-scoped through the service.
- MCP exposes the same registry automatically. `knowledge_search` stays on the "tainted" list (HANDOFF D4), and `knowledge_get_note` joins it (it returns user content that may contain injected instructions).
- **Relabel (D10):** tool descriptions say "your notes" / "your notes, bookmarks, journal entries and footprints" instead of "Second Brain", so the model stops repeating the term back to the user. The tool names don't change (MCP clients depend on them).

### 9.4 Frontend (`frontend/src/modules/knowledge/`)

- `types.ts`: split `NoteSummary` / `Note`. The list renders `excerpt`, and opening a note fetches `GET /notes/:id`.
- The editor holds the loaded `content_hash` and sends it as `expected_content_hash`. On a 409 it shows "This note changed elsewhere — reload" (keeping the user's draft in the textarea).
- Type selector (sentence-case labels). Export/import buttons sit behind the existing step-up flow.
- A subtle notice when the search response has `X-Search-Mode: lexical`.
- **Relabel (D10):** "Second Brain" → "Notes" in [`Sidebar.tsx`](../../../frontend/src/components/Sidebar.tsx) (nav label), [`AppShell.tsx`](../../../frontend/src/components/AppShell.tsx) (page title), [`modules/knowledge/index.tsx`](../../../frontend/src/modules/knowledge/index.tsx) (module name, loading text) and [`KnowledgeMainView.tsx`](../../../frontend/src/modules/knowledge/KnowledgeMainView.tsx) (empty state: "Capture ideas, snippets and research as notes."). Update `e2e/fixtures.ts` and `e2e/knowledge.spec.ts` in the same change. The `knowledge` module id and route don't change.
- `e2e/fake-api.ts` mocks + `e2e/knowledge.spec.ts`: list with excerpt, 409 path, lexical notice.

---

## 10. Export / import (D9)

**Vault layout**

```text
notes-<yyyy-mm-dd>.zip
└── notes/
    └── <slug>-<id>.md        slug: lowercase ASCII/CJK-safe, ≤ 60 chars, derived at export time only
```

**File format** (hand-rendered YAML for a fixed key set, deterministic key order):

```markdown
---
id: 42
type: decision
title: "Use PostgreSQL as the source of truth"
tags: [architecture, postgresql]
source: web
created_at: 2026-10-08T02:00:00Z
updated_at: 2026-10-08T02:30:00Z
---

<note content, byte-for-byte>
```

**Import rules**

- Caps: zip ≤ 20 MB, ≤ 5,000 entries, each entry ≤ 1 MB uncompressed, total uncompressed ≤ 100 MB (zip-bomb guard). Only `*.md` entries are read. Entry names are never used as filesystem paths (in-memory only, so zip-slip can't happen).
- Frontmatter is parsed with `serde_yaml_ng` into a strict struct (`deny_unknown_fields` off, with unknown keys ignored). A missing frontmatter block means title = the first `# ` heading or the file stem.
- If `id` is present **and** owned by the caller: update through `NoteService::update` with `ExpectedHash::Any` (skipped when the content hash is unchanged). If the id is foreign or missing: create. **An id is never trusted across accounts.**
- Batches of 100 per transaction, each note publishing its normal outbox event and embed job.
- Round-trip test: export → import into a fresh account → `content` byte-identical, tags/type/title equal.

**Out of scope:** attachments (no note references binary assets today). When they arrive, they live in `StorageService` (`StorageCategory::Attachments`) and the export adds `attachments/`.

---

## 11. Rollout sequence (each step ships green on its own)

0. **Relabel "Second Brain" → "Notes"** (D10): a copy-only commit covering the UI, chat help, LLM tool descriptions and e2e expectations. It has no dependencies, so it can ship first.
1. **Markdown module + domain split.** `markdown.rs`, `ContentHash`, `NoteType`, `Note`/`NoteDocument`, the RRF function. Pure unit tests. No schema change.
2. **Migrations 0012 + 0013.** NoteService writes the derived columns on every create/update. Reads still work for NULL-derived rows (via COALESCE).
3. **Backfill.** `backfill_derived_fields` runs inside the existing backfill job. Verify there are zero NULL rows in prod (a runbook query in HANDOFF).
4. **Search rewrite** (lexical + vector + RRF + degrade), with `X-Search-Mode`.
5. **Embedding:** skip-on-hash, per-chunk model, the `passage:`/`query:` prefixes. Switch the inference default to `multilingual-e5-small` (the model download goes into the inference runbook) → the backfill re-embeds everything. During the switch the vector list returns only the already-re-embedded items, and lexical covers the rest.
6. **API/contract change + frontend** (D6, D7) in one commit with the regenerated `openapi.json`.
7. **LLM/MCP tools** (`note_type`, `mode`, `knowledge_get_note`).
8. **Export/import** endpoints + UI.
9. **Migration 0014** (NOT NULL + simplified generated column), only after step 3 is confirmed in every environment.
10. **Cleanup:** delete `NoteFormat`, the old `search_hybrid`, `get_item_content_for_embedding`'s note branch, and the dead tests. Update `docs/llm.md` (model, prefixes), `docs/database.md` (pg_trgm, derived columns), the README env table and HANDOFF.

---

## 12. Testing

| Area | Tests |
|---|---|
| `markdown.rs` | headings/emphasis/links/images/code/tables → plain text; CJK preserved; HTML stripped; excerpt char boundary; frontmatter render→parse round trip; missing/malformed frontmatter |
| Domain | `ContentHash` format; `NoteType` parse; content cap; `rrf` ordering and exact tie-breaks; one slot per item |
| Repository (`live_postgres`) | derived columns round-trip; `list` never returns content (type-level: the row struct has no `content`); owner isolation for get/list/update/search; `simple` FTS + trigram finds a Chinese 2–3 char query |
| NoteService | the 409 on a stale hash; `FOR UPDATE` serialises concurrent patches; no embed job when nothing changed; outbox payload has no `content`; mid-transaction failure leaves no event |
| Indexing | skip on the same hash; a stale job doesn't overwrite a newer hash; model switch re-embeds; the full rebuild invariant (§8) |
| Search | lexical-only when the embedder errors or times out (mock), mode reported; a new note found before embedding; archived excluded by default |
| HTTP | contract tests for the new DTOs; `SENSITIVE` covers export/import; import caps, foreign-id → create, zip-bomb rejection |
| Frontend e2e | list excerpt, open document, 409 reload path, lexical notice, export/import buttons gated |

All AGENTS §8 gates apply (`fmt`, `check`, `clippy -D warnings`, lib tests, the architecture test, `live_postgres`, the openapi diff **expected to change once in step 6**, the frontend build and e2e). Regenerate `.sqlx/` after every query change.

---

## 13. Explicitly not doing

- No Markdown files as a store, no `ContentStore` trait, no `LocalMarkdownStore`, no S3/R2/WebDAV content providers, no reconciliation job, no dual-write phase (D1).
- No entities, relations, decisions tables, extraction jobs, RecallService or `brain.*` tools in this refactor (D8 — separate spec).
- No `slug`/`path`/`metadata` columns. No Git integration. No filesystem watcher.
- No new vector dimension. No external search engine. No segmentation extension.

## 14. Completion criteria

- [ ] `notes.format` is Markdown-only and `NoteFormat` is deleted
- [ ] `content_hash`, `search_text` and `excerpt` are populated for every note, then `NOT NULL`
- [ ] FTS uses the indexed `search_vector` over Markdown-stripped text; the trigram finds Chinese substrings
- [ ] Search uses RRF, finds new notes before embedding, and degrades to lexical with `X-Search-Mode`
- [ ] The embed job skips unchanged content; chunks carry `embedding_model`; the e5 prefixes are applied
- [ ] Deleting all `knowledge_embeddings` rows is fully recoverable by the backfill (test passes)
- [ ] List and outbox payloads carry no note body; `GET /notes/:id` returns the document
- [ ] The editor's stale save returns 409; the merge logic lives in `NoteService`
- [ ] `note_type` is settable via REST, LLM and MCP
- [ ] Export/import round-trips byte-identical content; both are step-up gated
- [ ] Chat capture (text/quote/OCR/voice), MCP, the web console, outbox and jobs work end to end
- [ ] No user-facing "Second Brain" text remains (UI, chat help, tool descriptions); the label is "Notes"
- [ ] `openapi.json`, `.sqlx/`, docs and HANDOFF are updated; all gates are green
