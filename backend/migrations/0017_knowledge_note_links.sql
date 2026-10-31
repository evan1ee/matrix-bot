-- Outgoing `[[Title]]` links of a note, derived from its Markdown on every
-- write. A link to a title with no note keeps `target_note_id` NULL so the
-- link resolves as soon as such a note exists ("ghost" links).
CREATE TABLE knowledge_note_links (
    id             BIGSERIAL PRIMARY KEY,
    owner_id       UUID   NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    source_note_id BIGINT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    target_note_id BIGINT REFERENCES notes(id) ON DELETE SET NULL,
    target_title   TEXT   NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_note_links_source_title
    ON knowledge_note_links (source_note_id, LOWER(target_title));
CREATE INDEX idx_note_links_owner_target
    ON knowledge_note_links (owner_id, target_note_id);
CREATE INDEX idx_note_links_owner_title
    ON knowledge_note_links (owner_id, LOWER(target_title));
