# Note wiki-links and knowledge graph (Blueprint 7)

Spec: `specs/2026-10-31-feature-blueprints.md` §Blueprint 7. Branch `feat/note-wiki-links`.

## Decisions (my calls, deviating from or filling gaps in the spec)

- Migration is `0017_knowledge_note_links.sql` (0016 is the calendar feed; the spec's 0018 numbering is stale).
- Links are derived data: `NoteService` replaces a source note's outgoing rows inside the write transaction (create, update, append). No new `SystemEvent`; `NoteCreated`/`NoteUpdated` already fire.
- Resolution is case-insensitive on title; duplicate titles resolve to the lowest note id. A link whose target is the source note (by id or by own title) is dropped.
- Unresolved links keep `target_note_id = NULL`. Creating or renaming a note re-resolves matching dangling links (`resolve_dangling`); renaming away from a title unresolves links that no longer match (`unresolve_stale`).
- Parser ignores fenced code blocks and inline code, caps at 50 distinct targets, dedups by lowercased title.
- Graph: non-archived notes only, capped at 500 nodes; edges are resolved links between included nodes.
- `knowledge_note_links` goes into `OWNED_TABLES` (it carries `owner_id`).
- Frontend: no new graph dependency; a small SVG force layout. Link rendering in the editor preview, backlinks panel, `[[` autocomplete.

## Steps

1. Domain `domain/knowledge/links.rs`: `extract_links(&str, own_title) -> Vec<WikiLink>`; unit tests first (aliases, code blocks, cap, dedup, self-link).
2. Migration + `OWNED_TABLES`; repository `repositories/knowledge/links.rs` (replace, resolve, dangling, list outgoing/backlinks, graph) with round-trip, cascade and owner-isolation tests.
3. `NoteService`: call link sync in `create`/`apply`; service tests (backlink appears, rename re-resolves, delete leaves dangling, cross-owner isolation, failed write rolls back).
4. REST `GET /notes/{id}/links`, `GET /notes/graph`; OpenAPI; `generate-api`; contract fixture if the crate asserts one.
5. Frontend: types/api, backlinks panel, `[[` autocomplete, graph tab, Playwright spec + fake-api mocks.
6. Gates (AGENTS §8), HANDOFF update with rulings, commit.
