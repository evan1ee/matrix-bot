-- Notes are Markdown only: `plain` is valid Markdown and `quote` rows already
-- store `> ` blockquote Markdown, so the format column no longer discriminates.
-- Data rewrite only; the constraint change is in 0013 (docs/database.md keeps
-- data rewrites and DDL in separate files).

-- Guard: name any row whose format is outside the known set before rewriting.
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM notes WHERE format NOT IN ('markdown','plain','quote')) THEN
    RAISE EXCEPTION 'unexpected notes.format values: %',
      (SELECT array_agg(DISTINCT format) FROM notes WHERE format NOT IN ('markdown','plain','quote'));
  END IF;
END $$;

UPDATE notes SET format = 'markdown' WHERE format <> 'markdown';
