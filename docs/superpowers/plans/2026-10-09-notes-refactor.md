# Notes (Second Brain) Refactor — Implementation Plan

> **For agentic workers:** steps use checkboxes (`- [ ]`). Work task by task with strict TDD: failing test → minimal code → green → refactor → run the task's gates. Each task ends in **one commit** that also updates [`HANDOFF.md`](../HANDOFF.md). **Don't start implementation until the user explicitly says so.**

**Status:** Plan ready. Design approved 2026-10-09.
**Branch:** `feat/notes-refactor` (from `master`)
**Spec:** [`specs/2026-10-08-second-brain-refactor-design.md`](../specs/2026-10-08-second-brain-refactor-design.md). §-references below point there.
**Authority:** [`AGENTS.md`](../../../AGENTS.md), [`docs/architecture.md`](../../architecture.md), [`docs/database.md`](../../database.md), [`docs/llm.md`](../../llm.md)

---

## Finding recorded before planning

- **Ruling R6:** the inference `/v1/embeddings` handler ([`inference/src/api/embeddings.rs`](../../../inference/src/api/embeddings.rs) L77-80) is a placeholder. It folds the input bytes into a normalised 384-dim vector and runs no model. Task 6 replaces it with real ONNX inference (`ort` is already a dependency) for `multilingual-e5-small`, using mean pooling + L2 normalisation — without it, "semantic" search is byte-frequency matching and D4/D5 give nothing — the cost is that the inference image grows by about 120 MB (the int8 model is about 118 MB) and CPU time per chunk is about 10–30 ms.
- HANDOFF §5 calls pgvector hybrid search "verified". That's true for the plumbing, not for retrieval quality. Task 6 corrects the HANDOFF wording.

## Gates (run at the end of every task that touches the listed area)

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p matrix-bot --lib
cargo test -p matrix-bot --test architecture
cargo test -p matrix-bot live_postgres -- --ignored          # tasks touching SQL
cargo test -p matrix-bot-inference                              # task 6
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # must PASS except in tasks 7 and 9
npm --prefix frontend run build && npm --prefix frontend run test:e2e           # tasks 0, 7, 9
```

After any query or migration change: regenerate `.sqlx/` against a migrated scratch DB and commit it.

## Dependency graph

```text
T0 relabel ─────────────────────────────────────────────── (independent)
T1 markdown+domain ─► T2 migrations+write path ─► T3 backfill ─┬─► T4 search ─► T5 indexing ─► T6 real e5 inference
                                                                ├─► T7 API/frontend contract
                                                                ├─► T8 LLM/MCP tools
                                                                └─► T9 export/import
T3 confirmed in every env ─► T10 migration 0014 ─► T11 cleanup + docs
```

---

### Task 0: Relabel "Second Brain" → "Notes" (D10)

- [ ] e2e first: change `frontend/e2e/fixtures.ts` (`knowledge: 'Notes'`) and `frontend/e2e/knowledge.spec.ts` (heading `Notes`, describe title). Run → red.
- [ ] `frontend/src/components/Sidebar.tsx` L40, `AppShell.tsx` L27, `modules/knowledge/index.tsx` (name, label, "Loading notes…"), `KnowledgeMainView.tsx` L193 ("Capture ideas, snippets and research as notes.").
- [ ] `backend/src/interface/chat/knowledge.rs`: the command description "Create or manage your notes" and the help header `📝 <b>Notes:</b>`. Adjust the help-text unit test.
- [ ] `backend/src/llm/tools/knowledge.rs`: descriptions → "your notes, bookmarks, journal entries and footprints" / "Save an idea, thought or document as a note". Tool names unchanged.
- [ ] `rg -i "second brain" frontend/src backend/src` returns only `//!` doc comments, which also get updated to "notes / knowledge base".
- [ ] Gates: Rust lib + clippy, frontend build + e2e. openapi diff empty.

### Task 1: Markdown module + domain split (§5.2, §5.3, R1, R2)

Pure code, no schema change, no behaviour change at the interfaces.

- [ ] Add the dependency `pulldown-cmark = { version = "0.13", default-features = false }` to `backend/Cargo.toml` (check the current version and MSRV first).
- [ ] `backend/src/application/knowledge/markdown.rs` (tests first):
  - `to_plain_text(&str) -> String`: keeps text, code contents, link text, image alt and table cells; drops syntax, HTML and URLs; separates blocks with `\n`; CJK preserved.
  - `excerpt(&str, max_chars) -> String`: whitespace collapsed, char-boundary safe, `…` when truncated.
  - `content_hash(&str) -> ContentHash` (sha2, hex).
- [ ] `domain/knowledge/note.rs`:
  - `ContentHash` newtype (`from_hex` validates 64 lowercase hex chars).
  - `NoteType` enum + `FromStr`/`as_str`. An unknown value is an **error** (unlike today's `NoteFormat`, which silently defaults).
  - `MAX_CONTENT_CHARS = 100_000` in `validate_content`, with `KnowledgeError::ContentTooLong`.
  - `NotePatch`, `ExpectedHash`, and `KnowledgeError::Conflict` → `AppError::Conflict`.
  - Introduce `NoteDocument { note, markdown }`. Keep the existing `Note` struct shape for now (the field moves happen in T2, together with the repository rows).
- [ ] `domain/knowledge/search.rs`: `rrf(lexical: &[ItemKey], vector: &[ItemKey], k: u32) -> Vec<(ItemKey, f64)>`, with exact tie-breaks (fused score desc → earlier lexical rank → lower `item_id`) and one entry per item. `SearchMode { Hybrid, Lexical }`.
- [ ] Split `application/knowledge/mod.rs` (491 lines) into `notes.rs`, `search.rs` and `markdown.rs`, with `mod.rs` holding only re-exports and the `From<KnowledgeError>` impl. Behaviour is unchanged and the existing tests pass.
- [ ] Gates: Rust lib, architecture, clippy.

### Task 2: Migrations 0012/0013 + the derived write path (§5.1, §6.1, D2, D3, D6-domain, D7)

- [ ] `backend/migrations/0012_notes_markdown_only.sql`: the guard + `UPDATE` exactly as in §5.1 (data only).
- [ ] `backend/migrations/0013_notes_derived_columns.sql`: as in §5.1 (pg_trgm, format CHECK, `note_type`, `content_hash`, `search_text`, `excerpt`, `indexed_content_hash`, `indexed_at`, the generated `search_vector`, the GIN + trigram indexes, dropping `idx_notes_search`, `knowledge_embeddings.embedding_model`). Confirm the CHECK constraint's actual name with `\d notes` on a migrated scratch DB before writing the `DROP CONSTRAINT`.
- [ ] Verify the pgvector compose image ships `pg_trgm` (contrib). Rehearse `0012`+`0013` on a copy of production data (docs/database.md rule for data-rewriting migrations).
- [ ] Repository `infrastructure/postgres/repositories/knowledge/notes.rs`:
  - Private `NoteRow` (metadata, **no `content` field**) and `NoteDocumentRow` (+ `content`). `list_notes` selects metadata only.
  - `insert_note` / `update_note` take the derived fields.
  - `lock_note_for_update(tx, owner, id) -> Option<NoteDocument>` (`FOR UPDATE`).
  - Tests: round trip of the derived columns, owner isolation, and that the list doesn't touch content.
- [ ] Domain `Note`: remove `content`/`format`, add `note_type`, `excerpt`, `content_hash`, `indexed_at`. Fix every compile error by routing body readers to `NoteDocument`:
  - `interface/http/knowledge.rs`: **keep the HTTP DTO contract unchanged in this task**. `list` keeps returning `content`, by mapping from a temporary `list_documents` service call, which is deleted in T7. The openapi diff must stay empty.
  - `interface/chat/knowledge.rs` `!note list`: use `excerpt`.
  - `SystemEvent::NoteCreated/NoteUpdated(Note)`: now metadata-only. Add/adjust the events test asserting the serialized payload has no `content` key.
- [ ] `NoteService` (§6.1): `create` derives the hash, search_text and excerpt; `update(owner, id, NotePatch)` uses lock → compare the expected hash → merge → re-derive → update → publish → enqueue the embed job **only if the title or content_hash changed**; `append`. Move the merge logic out of the HTTP `update_note` handler (the handler builds a `NotePatch` with `ExpectedHash::Any` until T7).
- [ ] Tests (live_postgres): 409 on a stale hash; two concurrent patches serialise (spawn two tasks, both applied, final hash = second); no job is enqueued on a no-op update; a mid-tx failure leaves no outbox row.
- [ ] Regenerate `.sqlx/`. Gates: everything except frontend. openapi diff empty.

### Task 3: Backfill derived fields (§6.2, rollout step 3)

- [ ] Repository: `list_notes_missing_derived(limit)` and `set_derived_fields(id, owner, hash, search_text, excerpt)` (guarded `WHERE content_hash IS NULL`, so it's idempotent and never overwrites a fresh write).
- [ ] `NoteService::backfill_derived_fields(batch = 500) -> usize`: one tx per batch. Called at the start of `KnowledgeBackfillEmbeddings` handling in `interface/jobs/knowledge.rs`.
- [ ] Tests: NULL rows get filled; a second run is a no-op; a concurrently updated row isn't clobbered.
- [ ] HANDOFF runbook: `SELECT count(*) FROM notes WHERE content_hash IS NULL OR search_text IS NULL OR excerpt IS NULL;` must be 0 in every environment before T10.
- [ ] Gates: Rust + live_postgres.

### Task 4: Search rewrite — lexical + vector + RRF + degrade (§6.3, D5, R3, R4)

- [ ] Repository `knowledge/search.rs`:
  - `search_lexical(owner, q, types, include_archived, limit)`: notes via `search_vector @@ websearch_to_tsquery('simple', $q)` ranked by `ts_rank_cd`, UNION trigram (`word_similarity($q, title || ' ' || search_text) > 0.3`), plus `knowledge_embeddings.content_snippet` FTS for bookmark/journal/footprint. Returns `(item_type, item_id, rank)`.
  - `search_vector(owner, embedding, model, types, include_archived, limit)`: `DISTINCT ON (item_type, item_id)` keeping the best chunk, filtered by `embedding_model = $model`, returning the snippet.
  - `hydrate(owner, keys)`: title, source_ref and excerpt for the result keys (one query per type or a single LEFT JOIN, as today).
- [ ] `SearchService::search(owner, SearchQuery) -> SearchOutcome { items, mode }`: run lexical; embed `"query: " + q` with a 3 s `tokio::time::timeout`; on error or timeout set `mode = Lexical` and `warn!(operation="knowledge.search", outcome="degraded", error_kind, elapsed_ms)`; fuse with `rrf`; hydrate; snippet = best chunk, else excerpt. Archived items are excluded unless `include_archived`.
- [ ] HTTP: set the `X-Search-Mode` header, add the optional `include_archived` to the request DTO, and document the header in the utoipa path (an additive openapi change is acceptable here — note it in the commit). Chat: append "(keyword results only)" when lexical. LLM tool: include `"mode"`.
- [ ] Tests: a new note found before any embedding exists; a failing mock embedder → lexical + mode; a Chinese 2–3 char query finds a note via trigram; one result per note even with several chunks; owner isolation; archived excluded by default.
- [ ] Delete the old `search_hybrid`. Gates: everything except frontend (openapi: additive diff, committed).

### Task 5: Indexing — skip-on-hash, model tagging, rebuild invariant (§6.2, §8, D4)

- [ ] Config: `EMBEDDING_MODEL` (single env name, default `multilingual-e5-small`) flows into `LocalOnnxEmbeddingClient` and into `SearchService`/indexing as the "configured model". Update the README env table.
- [ ] `embed_item` for notes: load `(title, search_text, content_hash, indexed_content_hash)`; skip when the hashes match and every chunk's model equals the configured one; chunk `title + "\n\n" + search_text` and prefix `"passage: "`; embed outside the tx; in one tx delete + insert chunks with `embedding_model`, then `UPDATE notes SET indexed_content_hash=$h, indexed_at=now() WHERE id=$id AND content_hash=$h`. If 0 rows are updated, roll back (a newer write won).
- [ ] Other item types: same `"passage: "` prefix and model tagging. No hash skip.
- [ ] `list_unindexed_items` → `list_items_needing_index(model, limit)`: missing chunks, OR notes with `indexed_content_hash IS DISTINCT FROM content_hash`, OR any item with a chunk whose `embedding_model <> $model`.
- [ ] Tests (live_postgres, `MockEmbeddingProvider`): skip on an unchanged hash; a stale job loses to a newer write; **the rebuild invariant** (embed → `DELETE FROM knowledge_embeddings` + null `indexed_content_hash` → backfill + handle jobs → chunks equal the first run); a model switch re-tags every chunk.
- [ ] Gates: Rust + live_postgres.

### Task 6: Real e5 inference in `matrix-bot-inference` (R6)

- [ ] Add `tokenizers` (check its compile time and that it works with `default-features = false, features = ["onig"]` or the `fancy-regex` variant; pick whichever builds on the Windows + Linux CI) to `inference/Cargo.toml`.
- [ ] `inference/src/embeddings/`: load `model.onnx` (int8 export of `intfloat/multilingual-e5-small`) + `tokenizer.json` once into an `ort::Session` behind the existing app state; truncate to 512 tokens; batch; run on `spawn_blocking`; mean-pool over the attention mask; L2-normalise; assert dim 384. Bound concurrency with a semaphore (size = physical cores / 2, min 1).
- [ ] `models_present` checks `model.onnx` + `tokenizer.json`. The `/v1/embeddings` handler returns the real model name and rejects requests asking for a different `model` with 400.
- [ ] Provisioning: add the download URLs + SHA-256 for the two files to the existing `download.rs` flow, and document `EMBEDDING_MODEL_DIR` in the README and the inference runbook.
- [ ] Tests: the handler contract test (503 when missing, unchanged); `#[ignore]` model test — "猫坐在垫子上" vs "The cat sits on the mat" cosine > "The cat sits on the mat" vs "Quarterly tax filing" cosine.
- [ ] Remove the byte-folding placeholder. Correct HANDOFF §5's pgvector bullet. Gates: inference crate tests + clippy.
- [ ] Operator step (HANDOFF Track 1, user-gated): deploy inference with the model, then the backend with `EMBEDDING_MODEL=multilingual-e5-small`; the backfill re-embeds everything; lexical covers the gap meanwhile.

### Task 7: API contract change + frontend (§9.1, §9.4, D6, D7)

- [ ] DTOs: `NoteSummary` (list), `Note` (= summary + `content` + `format: "markdown"`), `CreateNoteInput.note_type?`, `UpdateNoteInput.expected_content_hash?` + `note_type?`. `format` on input is accepted and ignored. 409 body: `{"error":"note changed since it was loaded","code":409}`.
- [ ] Delete the temporary `list_documents` from T2. The handler maps the DTO → `NotePatch` (`ExpectedHash::Exactly` when the hash is supplied).
- [ ] HTTP tests: list has no `content`; 409 on a stale hash; `note_type` round trip; invalid `note_type` → 400.
- [ ] Frontend `modules/knowledge/`: `types.ts` split; list renders `excerpt`; opening a note fetches `GET /notes/:id`; the editor sends `expected_content_hash` and on a 409 shows "This note changed elsewhere — reload", keeping the draft; type selector; lexical notice from `X-Search-Mode` (read the header in `api.ts` via `apiClient`).
- [ ] `e2e/fake-api.ts`: summary vs document responses, the 409 path, a header toggle. `e2e/knowledge.spec.ts`: excerpt list, open document, conflict reload, lexical notice.
- [ ] Regenerate `backend/openapi.json` (an **expected** diff, committed together with the frontend change). All gates.

### Task 8: LLM / MCP tools (§9.3)

- [ ] `knowledge_create_note`: optional `note_type` enum (OpenAI `type: "function"` schema).
- [ ] New `knowledge_get_note { id }` → `services.notes.get(owner, id)`. The owner comes from the turn, never from the model. Returns title, type, tags and markdown (truncated to the tool-output cap with a note when cut).
- [ ] Add `knowledge_get_note` to the tainted-output set next to `knowledge_search` (HANDOFF D4). Add it to `docs/llm.md`.
- [ ] Tests: the tool registry lists it; owner scoping (another account's id → not found); taint classification; MCP exposes it.
- [ ] Gates: Rust lib + architecture + clippy.

### Task 9: Markdown export / import (§10, D9)

- [ ] Dependencies: `zip` (deflate only, `default-features = false, features = ["deflate"]`) and `serde_yaml_ng`. Check the current versions and maintenance first.
- [ ] `markdown.rs`: `render_note_file(&NoteDocument) -> String` (hand-written YAML, fixed key order, quoted strings) and `parse_note_file(&str) -> ParsedNote` (frontmatter optional; title falls back to the first `# ` heading, then the file stem). `export_slug(title, id)`. Tests: render → parse round trip byte-identical body, CJK titles, malformed frontmatter → falls back without failing.
- [ ] `NoteService::export(owner) -> Vec<(String, String)>` (path, file), capped at 10k notes, and `NoteService::import(owner, Vec<ParsedNote>) -> ImportReport`: same-owner id → `update` with `ExpectedHash::Any` (skipped when the hash is unchanged), otherwise `create`; batches of 100 per tx; normal events and jobs.
- [ ] HTTP: `GET /api/v1/notes/export` (zip built in `spawn_blocking`, `Content-Disposition: attachment; filename="notes-<date>.zip"`) and `POST /api/v1/notes/import` (multipart; caps 20 MB body, ≤ 5,000 entries, ≤ 1 MB per entry, ≤ 100 MB total uncompressed, measured while reading, never trusting the headers; only `*.md`; names never touch the filesystem). Both use `VerifiedUser`. Add both to `SENSITIVE` in `interface/http/mod.rs`. Register before `/notes/:id`.
- [ ] Tests: export → import into a fresh account gives identical content; a foreign id → create; zip bomb / oversize entry / too many entries → 400 with the cap named; a non-step-up session → 403.
- [ ] Frontend: Export/Import buttons on the Notes page using the existing step-up flow; import result summary. e2e with fake-api.
- [ ] Regenerate openapi (an expected diff). All gates.

### Task 10: Migration 0014 — NOT NULL + simplified index (§5.1)

**Precondition:** the T3 runbook query returns 0 in every environment (user-confirmed for production).

- [ ] `backend/migrations/0014_notes_derived_not_null.sql`: a guard naming the offending ids → `SET NOT NULL` on `content_hash`, `search_text` and `excerpt` → drop and re-add `search_vector` over `search_text` only → recreate the trigram index over `title || ' ' || search_text`.
- [ ] Remove the `COALESCE(search_text, content)` handling and the `Option` fields in the rows. Regenerate `.sqlx/`.
- [ ] Gates: Rust + live_postgres.

### Task 11: Cleanup + docs

- [ ] Delete `NoteFormat`, the dead tests, the note branch of `get_item_content_for_embedding` if it's superseded, and any leftover temporary helpers. `rg -n "NoteFormat|search_hybrid|list_documents"` returns nothing.
- [ ] Check that no `backend/src` or `inference/src` file exceeds 500 production lines.
- [ ] Docs: `docs/llm.md` (e5 model, `query:`/`passage:` prefixes, `knowledge_get_note`, the search `mode`); `docs/database.md` (pg_trgm, derived columns, the rebuild invariant); `docs/api.md` (`X-Search-Mode`, 409 semantics, export/import caps); README env table (`EMBEDDING_MODEL`, `EMBEDDING_MODEL_DIR`).
- [ ] Spec status → Implemented. HANDOFF: status row, rulings R1–R6, operator steps.
- [ ] Full gate suite, with the actual results reported.

---

## Risks

| Risk | Mitigation |
|---|---|
| The `tokenizers` crate is heavy, or its regex backend fails to build on Windows CI | Try `fancy-regex` first. Fallback: pre-tokenise with a pure-Rust SentencePiece (`sentencepiece` model) loader. Decide in T6 and record it as a ruling. |
| Trigram with a 0.3 threshold is noisy for very short Latin queries | Use trigram only when the query contains CJK or FTS returns < 3 hits. Tune in T4 with fixtures. |
| Re-embedding everything after the model switch takes a while | Priority −1 jobs, and lexical covers it meanwhile. Personal-scale corpus. |
| `0012` rewrites user data | Guarded, rehearsed on a production copy, and the change is semantically lossless (plain/quote are valid Markdown). |
