-- Web assistant threads: a conversation can carry a title (NULL until the
-- first turn names it or the user renames it).
ALTER TABLE ai_conversations ADD COLUMN title TEXT;

CREATE INDEX idx_ai_conversations_owner_updated
    ON ai_conversations (owner_id, updated_at DESC);
