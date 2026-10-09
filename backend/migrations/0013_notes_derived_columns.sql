-- Notes keep Markdown in `content` as the single source of truth. Everything
-- else search needs is derived from it on write and can be rebuilt from it:
-- content_hash, search_text, excerpt, the generated search_vector and the
-- trigram index. indexed_content_hash/indexed_at record which body the
-- embeddings were built from.
--
-- The derived columns are nullable until the backfill job has filled every row;
-- a later migration makes them NOT NULL. Until then FTS falls back to `content`.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

ALTER TABLE notes DROP CONSTRAINT notes_format_check;
ALTER TABLE notes ADD CONSTRAINT notes_format_check CHECK (format = 'markdown');

ALTER TABLE notes
  ADD COLUMN note_type            TEXT NOT NULL DEFAULT 'note'
        CHECK (note_type IN ('note','idea','fact','observation','decision')),
  ADD COLUMN content_hash         TEXT,
  ADD COLUMN search_text          TEXT,
  ADD COLUMN excerpt              TEXT,
  ADD COLUMN indexed_content_hash TEXT,
  ADD COLUMN indexed_at           TIMESTAMPTZ,
  ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', title), 'A') ||
        setweight(to_tsvector('simple', COALESCE(search_text, content)), 'B')
  ) STORED;

DROP INDEX idx_notes_search;
CREATE INDEX idx_notes_search_vector ON notes USING GIN (search_vector);
CREATE INDEX idx_notes_search_trgm
    ON notes USING GIN ((title || ' ' || COALESCE(search_text, content)) gin_trgm_ops);

-- Which model produced each chunk, so a model switch can find stale chunks.
ALTER TABLE knowledge_embeddings
  ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'bge-small-en-v1.5';
CREATE INDEX idx_knowledge_embeddings_model ON knowledge_embeddings (embedding_model);
