-- Sub-project 2: one identity table for every channel (spec §7.1).
CREATE TABLE platform_identities (
    id               BIGSERIAL PRIMARY KEY,
    account_id       UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    platform         TEXT NOT NULL CHECK (platform IN ('matrix', 'discord', 'telegram', 'email')),
    platform_user_id TEXT NOT NULL,
    display_name     TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (platform, platform_user_id)
);
CREATE INDEX idx_platform_identities_account ON platform_identities(account_id);

CREATE TABLE link_tokens (
    token_hash       TEXT PRIMARY KEY,
    platform         TEXT NOT NULL CHECK (platform IN ('matrix', 'discord', 'telegram', 'email')),
    platform_user_id TEXT NOT NULL,
    display_name     TEXT,
    conversation     TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,
    consumed_at      TIMESTAMPTZ
);
CREATE INDEX idx_link_tokens_identity ON link_tokens(platform, platform_user_id) WHERE consumed_at IS NULL;
