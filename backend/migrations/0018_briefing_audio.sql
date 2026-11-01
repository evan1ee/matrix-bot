-- The spoken version of a day's morning briefing. One row per owner and local
-- day; the object lives in storage, the row says which one. Rows older than 30
-- days are removed by the owner's next briefing job.
CREATE TABLE briefing_audio (
    id          BIGSERIAL PRIMARY KEY,
    owner_id    UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    local_date  DATE NOT NULL,
    storage_key TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_id, local_date)
);
