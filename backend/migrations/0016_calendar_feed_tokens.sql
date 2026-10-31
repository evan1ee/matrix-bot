-- One private calendar-subscription token per account. Only the SHA-256 of the
-- token is stored; minting again replaces the row and so revokes the old link.
CREATE TABLE account_calendar_tokens (
    account_id       UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    token_hash       TEXT NOT NULL UNIQUE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_accessed_at TIMESTAMPTZ
);
