-- 0002_account_uuid.sql: migrate accounts.id, owner_id, and account_id from TEXT 'acct_<uuid>' to native UUID.
-- Spec: docs/superpowers/specs/2026-09-24-postgres-foundation-design.md §4.2

-- 1. Safety Guard: abort with clear error if any value does not conform to acct_<uuid> or is orphaned.
DO $$
DECLARE
    tbl TEXT;
    bad_vals TEXT;
    orphans BIGINT;
    owner_tables TEXT[] := ARRAY[
        'user_settings', 'webhooks', 'webhook_outbox', 'ai_user_configs', 'ai_conversations',
        'portfolio_holdings', 'portfolio_cash_balances', 'portfolio_daily_snapshots',
        'portfolio_price_alerts', 'portfolio_audit_log', 'footprints', 'habits', 'habit_logs',
        'life_day_logs', 'life_profiles', 'bookmarks', 'todos', 'reminders', 'finance_categories',
        'transactions', 'finance_shares', 'recurring_bills', 'bot_message_targets'
    ];
    account_tables TEXT[] := ARRAY[
        'matrix_links', 'webauthn_credentials', 'sessions', 'matrix_link_challenges',
        'passkey_enrollment_tokens', 'recovery_codes', 'recovery_requests', 'security_events',
        'mcp_tokens', 'account_rooms'
    ];
BEGIN
    -- Check accounts.id
    EXECUTE 'SELECT string_agg(id, '', '') FROM (SELECT id FROM accounts WHERE id !~ ''^acct_([0-9a-fA-F]{32}|[0-9a-fA-F-]{36})$'' LIMIT 5) s'
    INTO bad_vals;
    IF bad_vals IS NOT NULL AND bad_vals <> '' THEN
        RAISE EXCEPTION 'accounts.id contains non-UUID values: %', bad_vals;
    END IF;

    -- Check owner_id tables format and orphans
    FOREACH tbl IN ARRAY owner_tables LOOP
        EXECUTE format('SELECT string_agg(owner_id, '', '') FROM (SELECT owner_id FROM %I WHERE owner_id !~ ''^acct_([0-9a-fA-F]{32}|[0-9a-fA-F-]{36})$'' LIMIT 5) s', tbl)
        INTO bad_vals;
        IF bad_vals IS NOT NULL AND bad_vals <> '' THEN
            RAISE EXCEPTION 'table % column owner_id contains non-UUID values: %', tbl, bad_vals;
        END IF;

        EXECUTE format('SELECT COUNT(*) FROM %I t WHERE NOT EXISTS (SELECT 1 FROM accounts a WHERE a.id = t.owner_id)', tbl)
        INTO orphans;
        IF orphans > 0 THEN
            RAISE EXCEPTION 'table % column owner_id contains % orphaned rows not present in accounts', tbl, orphans;
        END IF;
    END LOOP;

    -- Check account_id tables format and orphans
    FOREACH tbl IN ARRAY account_tables LOOP
        EXECUTE format('SELECT string_agg(account_id, '', '') FROM (SELECT account_id FROM %I WHERE account_id !~ ''^acct_([0-9a-fA-F]{32}|[0-9a-fA-F-]{36})$'' LIMIT 5) s', tbl)
        INTO bad_vals;
        IF bad_vals IS NOT NULL AND bad_vals <> '' THEN
            RAISE EXCEPTION 'table % column account_id contains non-UUID values: %', tbl, bad_vals;
        END IF;

        EXECUTE format('SELECT COUNT(*) FROM %I t WHERE NOT EXISTS (SELECT 1 FROM accounts a WHERE a.id = t.account_id)', tbl)
        INTO orphans;
        IF orphans > 0 THEN
            RAISE EXCEPTION 'table % column account_id contains % orphaned rows not present in accounts', tbl, orphans;
        END IF;
    END LOOP;

    -- Check web_users (nullable account_id)
    EXECUTE 'SELECT string_agg(account_id, '', '') FROM (SELECT account_id FROM web_users WHERE account_id IS NOT NULL AND account_id !~ ''^acct_([0-9a-fA-F]{32}|[0-9a-fA-F-]{36})$'' LIMIT 5) s'
    INTO bad_vals;
    IF bad_vals IS NOT NULL AND bad_vals <> '' THEN
        RAISE EXCEPTION 'table web_users column account_id contains non-UUID values: %', bad_vals;
    END IF;
    EXECUTE 'SELECT COUNT(*) FROM web_users t WHERE account_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM accounts a WHERE a.id = t.account_id)'
    INTO orphans;
    IF orphans > 0 THEN
        RAISE EXCEPTION 'table web_users column account_id contains % orphaned rows not present in accounts', orphans;
    END IF;
END $$;

-- 2. Drop existing foreign key constraints on accounts(id)
ALTER TABLE webhooks DROP CONSTRAINT IF EXISTS webhooks_owner_id_fkey;
ALTER TABLE ai_user_configs DROP CONSTRAINT IF EXISTS ai_user_configs_owner_id_fkey;
ALTER TABLE ai_conversations DROP CONSTRAINT IF EXISTS ai_conversations_owner_id_fkey;
ALTER TABLE mcp_tokens DROP CONSTRAINT IF EXISTS mcp_tokens_account_id_fkey;
ALTER TABLE account_rooms DROP CONSTRAINT IF EXISTS account_rooms_account_id_fkey;

-- 3. Alter accounts.id to UUID
ALTER TABLE accounts ALTER COLUMN id TYPE UUID USING substr(id, 6)::uuid;

-- 4. Alter all 23 owner_id columns to UUID
ALTER TABLE user_settings ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE webhooks ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE webhook_outbox ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE ai_user_configs ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE ai_conversations ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE portfolio_holdings ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE portfolio_cash_balances ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE portfolio_daily_snapshots ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE portfolio_price_alerts ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE portfolio_audit_log ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE footprints ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE habits ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE habit_logs ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE life_day_logs ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE life_profiles ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE bookmarks ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE todos ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE reminders ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE finance_categories ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE transactions ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE finance_shares ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE recurring_bills ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;
ALTER TABLE bot_message_targets ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid;

-- 5. Alter all 11 account_id columns to UUID
ALTER TABLE matrix_links ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE webauthn_credentials ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE sessions ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE matrix_link_challenges ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE passkey_enrollment_tokens ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE recovery_codes ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE recovery_requests ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE security_events ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE web_users ALTER COLUMN account_id TYPE UUID USING CASE WHEN account_id IS NOT NULL THEN substr(account_id, 6)::uuid ELSE NULL END;
ALTER TABLE mcp_tokens ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;
ALTER TABLE account_rooms ALTER COLUMN account_id TYPE UUID USING substr(account_id, 6)::uuid;

-- 6. Add Foreign Key constraints REFERENCES accounts(id) ON DELETE CASCADE
ALTER TABLE user_settings ADD CONSTRAINT user_settings_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE webhooks ADD CONSTRAINT webhooks_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE webhook_outbox ADD CONSTRAINT webhook_outbox_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE ai_user_configs ADD CONSTRAINT ai_user_configs_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE ai_conversations ADD CONSTRAINT ai_conversations_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE portfolio_holdings ADD CONSTRAINT portfolio_holdings_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE portfolio_cash_balances ADD CONSTRAINT portfolio_cash_balances_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE portfolio_daily_snapshots ADD CONSTRAINT portfolio_daily_snapshots_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE portfolio_price_alerts ADD CONSTRAINT portfolio_price_alerts_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE portfolio_audit_log ADD CONSTRAINT portfolio_audit_log_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE footprints ADD CONSTRAINT footprints_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE habits ADD CONSTRAINT habits_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE habit_logs ADD CONSTRAINT habit_logs_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE life_day_logs ADD CONSTRAINT life_day_logs_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE life_profiles ADD CONSTRAINT life_profiles_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE bookmarks ADD CONSTRAINT bookmarks_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE todos ADD CONSTRAINT todos_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE reminders ADD CONSTRAINT reminders_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE finance_categories ADD CONSTRAINT finance_categories_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE transactions ADD CONSTRAINT transactions_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE finance_shares ADD CONSTRAINT finance_shares_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE recurring_bills ADD CONSTRAINT recurring_bills_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE bot_message_targets ADD CONSTRAINT bot_message_targets_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE;

ALTER TABLE matrix_links ADD CONSTRAINT matrix_links_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE webauthn_credentials ADD CONSTRAINT webauthn_credentials_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE sessions ADD CONSTRAINT sessions_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE matrix_link_challenges ADD CONSTRAINT matrix_link_challenges_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE passkey_enrollment_tokens ADD CONSTRAINT passkey_enrollment_tokens_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE recovery_codes ADD CONSTRAINT recovery_codes_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE recovery_requests ADD CONSTRAINT recovery_requests_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE security_events ADD CONSTRAINT security_events_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE web_users ADD CONSTRAINT web_users_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE mcp_tokens ADD CONSTRAINT mcp_tokens_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
ALTER TABLE account_rooms ADD CONSTRAINT account_rooms_account_id_fkey FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
