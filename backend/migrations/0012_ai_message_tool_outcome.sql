ALTER TABLE ai_messages
    ADD COLUMN tool_outcome TEXT NULL
    CHECK (tool_outcome IN ('ok', 'failed', 'needs_confirmation'));
