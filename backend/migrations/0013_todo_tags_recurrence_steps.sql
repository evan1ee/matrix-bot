ALTER TABLE todos
    ADD COLUMN tags         TEXT[]  NOT NULL DEFAULT '{}',
    ADD COLUMN recurrence   TEXT    NOT NULL DEFAULT 'none'
        CHECK (recurrence IN ('none', 'daily', 'weekly', 'monthly', 'yearly')),
    ADD COLUMN recurrence_anchor TIMESTAMPTZ,
    ADD COLUMN next_todo_id BIGINT  REFERENCES todos(id) ON DELETE SET NULL,
    ADD CONSTRAINT todos_recurrence_needs_due
        CHECK (recurrence = 'none' OR due_at IS NOT NULL);

CREATE INDEX idx_todos_tags ON todos USING GIN (tags);

CREATE TABLE todo_steps (
    id       BIGSERIAL PRIMARY KEY,
    todo_id  BIGINT  NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
    owner_id UUID    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    text     TEXT    NOT NULL,
    done     BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (todo_id, position)
);
CREATE INDEX idx_todo_steps_owner ON todo_steps(owner_id);
