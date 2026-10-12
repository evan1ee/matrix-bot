-- 0011_configuration_and_secrets.sql: Admin settings, encrypted secrets, audit log, and account roles.

-- 1. Account Roles: Add role column to accounts table. Named inline so that
-- every schema (one per test) gets its own constraint.
ALTER TABLE accounts ADD COLUMN role TEXT NOT NULL DEFAULT 'user'
    CONSTRAINT accounts_role_check CHECK (role IN ('user', 'admin'));

-- 2. Runtime Application Settings
CREATE TABLE app_settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_by  UUID REFERENCES accounts(id) ON DELETE SET NULL
);

-- 3. Encrypted Provider Secrets
CREATE TABLE provider_secrets (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider          TEXT NOT NULL,
    name              TEXT NOT NULL DEFAULT 'default',
    secret_ciphertext BYTEA NOT NULL,
    nonce             BYTEA NOT NULL,
    key_version       INT NOT NULL DEFAULT 1,
    status            TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'revoked')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_by        UUID REFERENCES accounts(id) ON DELETE SET NULL,
    last_tested_at    TIMESTAMPTZ,
    last_test_status  TEXT CHECK (last_test_status IN ('healthy', 'unhealthy', 'failed', 'unknown')),
    last_test_error   TEXT,
    metadata          JSONB,
    CONSTRAINT uq_provider_secrets_slot UNIQUE (provider, name)
);

-- 4. Dedicated Configuration Audit Log. Rows are also written for account
-- events (role changes, merges), hence the 'account' target type.
CREATE TABLE config_audit_log (
    id          BIGSERIAL PRIMARY KEY,
    actor_id    UUID REFERENCES accounts(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('setting', 'secret', 'account', 'system')),
    target_key  TEXT NOT NULL,
    outcome     TEXT NOT NULL CHECK (outcome IN ('success', 'failure')),
    detail      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_config_audit_log_created_at ON config_audit_log(created_at DESC);
CREATE INDEX idx_config_audit_log_target ON config_audit_log(target_type, target_key);

-- No index on accounts.role, app_settings.updated_at or provider_secrets.provider:
-- accounts has two roles, and the settings and secrets tables are small and
-- queried by key or by id.
