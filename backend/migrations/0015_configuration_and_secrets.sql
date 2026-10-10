-- 0015_configuration_and_secrets.sql: Admin settings, encrypted secrets, audit log, and account roles.

-- 1. Account Roles: Add role column to accounts table
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS role TEXT NOT NULL DEFAULT 'user';
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'accounts_role_check'
    ) THEN
        ALTER TABLE accounts ADD CONSTRAINT accounts_role_check CHECK (role IN ('user', 'admin'));
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS idx_accounts_role ON accounts(role);

-- 2. Runtime Application Settings
CREATE TABLE IF NOT EXISTS app_settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_by  UUID REFERENCES accounts(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_app_settings_updated_at ON app_settings(updated_at);

-- 3. Encrypted Provider Secrets
CREATE TABLE IF NOT EXISTS provider_secrets (
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

CREATE INDEX IF NOT EXISTS idx_provider_secrets_provider ON provider_secrets(provider);

-- 4. Dedicated Configuration Audit Log
CREATE TABLE IF NOT EXISTS config_audit_log (
    id          BIGSERIAL PRIMARY KEY,
    actor_id    UUID REFERENCES accounts(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('setting', 'secret', 'system')),
    target_key  TEXT NOT NULL,
    outcome     TEXT NOT NULL CHECK (outcome IN ('success', 'failure')),
    detail      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_config_audit_log_created_at ON config_audit_log(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_config_audit_log_target ON config_audit_log(target_type, target_key);
