-- Review follow-ups to 0015 (config management). 0015 may already be applied
-- somewhere, so these changes are a new forward-only migration rather than an
-- edit of 0015.

-- 0015 guarded this CHECK by constraint name alone, so with one schema per test
-- only the first schema got it. Recreate it on this schema's table.
ALTER TABLE accounts DROP CONSTRAINT IF EXISTS accounts_role_check;
ALTER TABLE accounts ADD CONSTRAINT accounts_role_check CHECK (role IN ('user', 'admin'));

-- Audit rows are also written for account events (role changes, merges).
ALTER TABLE config_audit_log DROP CONSTRAINT IF EXISTS config_audit_log_target_type_check;
ALTER TABLE config_audit_log ADD CONSTRAINT config_audit_log_target_type_check
    CHECK (target_type IN ('setting', 'secret', 'account', 'system'));

-- Nothing reads these: accounts has two roles, and the settings, secrets and
-- audit tables are small and queried by key or by id.
DROP INDEX IF EXISTS idx_accounts_role;
DROP INDEX IF EXISTS idx_app_settings_updated_at;
DROP INDEX IF EXISTS idx_provider_secrets_provider;
