-- Every note now carries its derived columns: new writes derive them in the
-- service and the backfill job filled the legacy rows (runbook query in
-- docs/superpowers/HANDOFF.md must have returned 0 before this runs). With no
-- NULLs left the COALESCE(search_text, content) fallbacks go away and the
-- search indexes are defined over search_text alone.

-- Guard: name the rows the backfill missed instead of failing on SET NOT NULL.
DO $$
DECLARE
  missing BIGINT;
  sample  BIGINT[];
BEGIN
  SELECT count(*) INTO missing
    FROM notes WHERE content_hash IS NULL OR search_text IS NULL OR excerpt IS NULL;
  IF missing > 0 THEN
    SELECT array_agg(id) INTO sample FROM (
      SELECT id FROM notes
       WHERE content_hash IS NULL OR search_text IS NULL OR excerpt IS NULL
       ORDER BY id LIMIT 20
    ) s;
    RAISE EXCEPTION
      '% notes lack derived columns (first ids: %); run the knowledge.backfill_embeddings job on the previous release first',
      missing, sample;
  END IF;
END $$;

ALTER TABLE notes
  ALTER COLUMN content_hash SET NOT NULL,
  ALTER COLUMN search_text  SET NOT NULL,
  ALTER COLUMN excerpt      SET NOT NULL;

-- A generated column's expression cannot be altered; dropping it also drops
-- its GIN index.
DROP INDEX idx_notes_search_trgm;
ALTER TABLE notes DROP COLUMN search_vector;
ALTER TABLE notes
  ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', title), 'A') ||
        setweight(to_tsvector('simple', search_text), 'B')
  ) STORED;

CREATE INDEX idx_notes_search_vector ON notes USING GIN (search_vector);
CREATE INDEX idx_notes_search_trgm
    ON notes USING GIN ((title || ' ' || search_text) gin_trgm_ops);
