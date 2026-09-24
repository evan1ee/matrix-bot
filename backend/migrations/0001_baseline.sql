-- Baseline: the schema as built by the pre-migration schema*.rs runner (master @ 07897ab). Idempotent so a database built by that runner adopts it in place. Never edit; add a new migration.

-- schema_auth.rs

CREATE TABLE IF NOT EXISTS accounts (
    id            TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active',
    created_at    TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS matrix_links (
    mxid        TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL,
    linked_at   TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_matrix_links_account ON matrix_links(account_id);

CREATE TABLE IF NOT EXISTS webauthn_credentials (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL,
    passkey_json  TEXT NOT NULL,
    label         TEXT NOT NULL,
    created_at    TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    last_used_at  TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_webauthn_credentials_account
 ON webauthn_credentials(account_id);

CREATE TABLE IF NOT EXISTS sessions (
    id_hash          TEXT PRIMARY KEY,
    account_id       TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    last_seen_at     TIMESTAMPTZ NOT NULL,
    last_verified_at TIMESTAMPTZ NOT NULL,
    restricted_until TIMESTAMPTZ,
    user_agent       TEXT,
    ip               TEXT,
    revoked_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_sessions_account ON sessions(account_id);

CREATE TABLE IF NOT EXISTS matrix_link_challenges (
    code        TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS passkey_enrollment_tokens (
    token       TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_enrollment_tokens_account
 ON passkey_enrollment_tokens(account_id);

CREATE TABLE IF NOT EXISTS recovery_codes (
    id          BIGSERIAL PRIMARY KEY,
    account_id  TEXT NOT NULL,
    code_hash   TEXT NOT NULL,
    created_at  TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    used_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_recovery_codes_account ON recovery_codes(account_id);

CREATE TABLE IF NOT EXISTS recovery_requests (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL,
    code_hash     TEXT NOT NULL,
    requested_at  TIMESTAMPTZ NOT NULL,
    valid_after   TIMESTAMPTZ NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    cancelled_at  TIMESTAMPTZ,
    consumed_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_recovery_requests_account
 ON recovery_requests(account_id);

CREATE TABLE IF NOT EXISTS security_events (
    id          BIGSERIAL PRIMARY KEY,
    account_id  TEXT NOT NULL,
    kind        TEXT NOT NULL,
    detail      TEXT,
    created_at  TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_security_events_account
 ON security_events(account_id, created_at DESC);

CREATE TABLE IF NOT EXISTS web_users (
    id BIGSERIAL PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE web_users ADD COLUMN IF NOT EXISTS account_id TEXT;

-- schema_settings.rs

CREATE TABLE IF NOT EXISTS user_settings (
    owner_id TEXT PRIMARY KEY,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    currency TEXT NOT NULL DEFAULT 'USD',
    dev_room_id TEXT,
    max_history_turns BIGINT NOT NULL DEFAULT 10,
    briefing_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    briefing_time TEXT NOT NULL DEFAULT '07:00',
    last_briefing_date TEXT,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS dev_room_id TEXT;
ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS max_history_turns BIGINT NOT NULL DEFAULT 10;
ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS briefing_enabled BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS briefing_time TEXT NOT NULL DEFAULT '07:00';
ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS last_briefing_date TEXT;
ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS tts_voice TEXT;
ALTER TABLE user_settings DROP COLUMN IF EXISTS tts_speaker_id;

-- schema_webhooks.rs

CREATE TABLE IF NOT EXISTS webhooks (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    token_prefix TEXT NOT NULL,
    default_room_id TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    total_delivered_count BIGINT NOT NULL DEFAULT 0,
    failure_count BIGINT NOT NULL DEFAULT 0,
    last_triggered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_webhooks_owner ON webhooks(owner_id);

CREATE INDEX IF NOT EXISTS idx_webhooks_token_hash ON webhooks(token_hash);

CREATE TABLE IF NOT EXISTS webhook_outbox (
    id TEXT PRIMARY KEY,
    webhook_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'info',
    title TEXT,
    html_body TEXT NOT NULL,
    plain_body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts BIGINT NOT NULL DEFAULT 0,
    max_attempts BIGINT NOT NULL DEFAULT 10,
    next_attempt_at TIMESTAMPTZ NOT NULL,
    last_error TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    delivered_at TIMESTAMPTZ,
    FOREIGN KEY (webhook_id) REFERENCES webhooks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_webhook_outbox_pending
 ON webhook_outbox(status, next_attempt_at);

-- schema_ai.rs

CREATE TABLE IF NOT EXISTS ai_user_configs (
    owner_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    api_base_url TEXT,
    api_key TEXT,
    model_name TEXT,
    system_prompt TEXT,
    temperature DOUBLE PRECISION DEFAULT 0.7,
    max_tokens BIGINT DEFAULT 2048,
    updated_at TEXT NOT NULL DEFAULT (NOW()::text)
);

CREATE TABLE IF NOT EXISTS ai_conversations (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    room_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (NOW()::text),
    updated_at TEXT NOT NULL DEFAULT (NOW()::text)
);

CREATE INDEX IF NOT EXISTS idx_ai_conversations_owner_room
ON ai_conversations(owner_id, room_id, updated_at);

CREATE TABLE IF NOT EXISTS ai_messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool')),
    content TEXT NOT NULL,
    tool_call_id TEXT,
    tool_name TEXT,
    created_at TEXT NOT NULL DEFAULT (NOW()::text)
);

CREATE INDEX IF NOT EXISTS idx_ai_messages_conv_created
ON ai_messages(conversation_id, created_at);

-- schema_portfolio.rs

CREATE TABLE IF NOT EXISTS portfolio_holdings (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    symbol TEXT NOT NULL,
    quantity_minor BIGINT NOT NULL,
    avg_cost_minor BIGINT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    current_price_minor BIGINT,
    price_updated_at TIMESTAMPTZ,
    note TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_holdings_owner_symbol
 ON portfolio_holdings(owner_id, symbol);

CREATE TABLE IF NOT EXISTS portfolio_cash_balances (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    balance_minor BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_cash_owner_currency
 ON portfolio_cash_balances(owner_id, currency);

CREATE TABLE IF NOT EXISTS portfolio_audit_log (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    action TEXT NOT NULL,
    symbol TEXT,
    quantity_minor BIGINT,
    price_minor BIGINT,
    amount_minor BIGINT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    realized_pl_minor BIGINT,
    note TEXT,
    prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000',
    entry_hash TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_portfolio_audit_log_owner
 ON portfolio_audit_log(owner_id, created_at DESC);

-- Audit-log immutability guards (ddl::audit_log_immutability()). PostgreSQL
-- has no CREATE TRIGGER IF NOT EXISTS, so each trigger is dropped first.

CREATE OR REPLACE FUNCTION prevent_portfolio_audit_update() RETURNS trigger AS $fn$ BEGIN RAISE EXCEPTION 'Audit log entries are immutable and cannot be updated'; END; $fn$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS prevent_portfolio_audit_update ON portfolio_audit_log;
CREATE TRIGGER prevent_portfolio_audit_update BEFORE UPDATE ON portfolio_audit_log FOR EACH ROW EXECUTE FUNCTION prevent_portfolio_audit_update();

CREATE OR REPLACE FUNCTION prevent_portfolio_audit_delete() RETURNS trigger AS $fn$ BEGIN RAISE EXCEPTION 'Audit log entries cannot be deleted'; END; $fn$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS prevent_portfolio_audit_delete ON portfolio_audit_log;
CREATE TRIGGER prevent_portfolio_audit_delete BEFORE DELETE ON portfolio_audit_log FOR EACH ROW EXECUTE FUNCTION prevent_portfolio_audit_delete();

ALTER TABLE portfolio_audit_log ADD COLUMN IF NOT EXISTS prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';
ALTER TABLE portfolio_audit_log ADD COLUMN IF NOT EXISTS entry_hash TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS portfolio_price_alerts (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    symbol TEXT NOT NULL,
    target_price_minor BIGINT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    condition TEXT NOT NULL,
    is_triggered BOOLEAN NOT NULL DEFAULT FALSE,
    triggered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_price_alerts_owner
 ON portfolio_price_alerts(owner_id, is_triggered);

CREATE INDEX IF NOT EXISTS idx_price_alerts_symbol
 ON portfolio_price_alerts(symbol, is_triggered);

CREATE TABLE IF NOT EXISTS portfolio_daily_snapshots (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    snapshot_date TEXT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    holdings_value_minor BIGINT NOT NULL,
    cash_minor BIGINT NOT NULL,
    total_net_worth_minor BIGINT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(owner_id, snapshot_date, currency)
);

CREATE INDEX IF NOT EXISTS idx_portfolio_snapshots_lookup
 ON portfolio_daily_snapshots(owner_id, currency, snapshot_date ASC);

-- schema_footprints.rs

CREATE TABLE IF NOT EXISTS footprints (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    title TEXT NOT NULL,
    latitude DOUBLE PRECISION NOT NULL,
    longitude DOUBLE PRECISION NOT NULL,
    country TEXT,
    city TEXT,
    address TEXT,
    arrival_date TEXT NOT NULL,
    departure_date TEXT,
    stay_duration_days BIGINT NOT NULL DEFAULT 1,
    spend_amount_minor BIGINT NOT NULL DEFAULT 0,
    spend_currency TEXT NOT NULL DEFAULT 'USD',
    spend_currency_exponent BIGINT NOT NULL DEFAULT 2,
    rating BIGINT NOT NULL DEFAULT 5,
    category TEXT NOT NULL DEFAULT 'travel',
    note TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_footprints_owner_arrival
 ON footprints(owner_id, arrival_date DESC);

CREATE INDEX IF NOT EXISTS idx_footprints_country
 ON footprints(owner_id, country);

-- schema_habits.rs

CREATE TABLE IF NOT EXISTS habits (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    frequency TEXT NOT NULL DEFAULT 'daily',
    category TEXT NOT NULL DEFAULT 'wellness',
    target_days_per_week BIGINT NOT NULL DEFAULT 7,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT DEFAULT (NOW()::text),
    updated_at TEXT DEFAULT (NOW()::text)
);
CREATE INDEX IF NOT EXISTS idx_habits_owner ON habits(owner_id);

CREATE TABLE IF NOT EXISTS habit_logs (
    id BIGSERIAL PRIMARY KEY,
    habit_id BIGINT NOT NULL REFERENCES habits(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL,
    log_date TEXT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT TRUE,
    note TEXT,
    created_at TEXT DEFAULT (NOW()::text),
    UNIQUE(habit_id, log_date)
);
CREATE INDEX IF NOT EXISTS idx_habit_logs_owner_date ON habit_logs(owner_id, log_date);
CREATE INDEX IF NOT EXISTS idx_habit_logs_habit_date ON habit_logs(habit_id, log_date);

-- schema_mcp.rs

CREATE TABLE IF NOT EXISTS mcp_tokens (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL,
    name          TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    prefix        TEXT NOT NULL,
    created_at    TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    last_used_at  TIMESTAMPTZ,
    expires_at    TIMESTAMPTZ,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_mcp_tokens_hash ON mcp_tokens(token_hash);

CREATE INDEX IF NOT EXISTS idx_mcp_tokens_account ON mcp_tokens(account_id);

-- schema_life.rs

CREATE TABLE IF NOT EXISTS life_profiles (
    owner_id TEXT PRIMARY KEY,
    birth_date TEXT NOT NULL,
    expected_lifespan_years BIGINT NOT NULL DEFAULT 80,
    created_at TEXT NOT NULL DEFAULT (NOW()::text),
    updated_at TEXT NOT NULL DEFAULT (NOW()::text)
);

CREATE TABLE IF NOT EXISTS life_day_logs (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    log_date TEXT NOT NULL,
    mood TEXT NOT NULL,
    weather TEXT,
    activities TEXT,
    note TEXT,
    time_allocation TEXT,
    created_at TEXT NOT NULL DEFAULT (NOW()::text),
    updated_at TEXT NOT NULL DEFAULT (NOW()::text),
    UNIQUE(owner_id, log_date)
);

CREATE INDEX IF NOT EXISTS idx_life_day_logs_owner_date ON life_day_logs(owner_id, log_date);
CREATE INDEX IF NOT EXISTS idx_life_day_logs_owner_mood ON life_day_logs(owner_id, mood);

ALTER TABLE life_day_logs ADD COLUMN IF NOT EXISTS time_allocation TEXT;

-- schema_bookmarks.rs

CREATE TABLE IF NOT EXISTS bookmarks (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    author TEXT,
    image_url TEXT,
    domain TEXT NOT NULL,
    reader_content TEXT,
    reading_time_minutes BIGINT DEFAULT 1,
    is_favorite BOOLEAN NOT NULL DEFAULT FALSE,
    is_archived BOOLEAN NOT NULL DEFAULT FALSE,
    tags TEXT NOT NULL DEFAULT '',
    created_at TEXT DEFAULT (NOW()::text),
    updated_at TEXT DEFAULT (NOW()::text)
);

CREATE INDEX IF NOT EXISTS idx_bookmarks_owner ON bookmarks(owner_id);

CREATE INDEX IF NOT EXISTS idx_bookmarks_owner_archived ON bookmarks(owner_id, is_archived);

CREATE INDEX IF NOT EXISTS idx_bookmarks_owner_favorite ON bookmarks(owner_id, is_favorite);

-- schema_rooms.rs

CREATE TABLE IF NOT EXISTS account_rooms (
    account_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    name TEXT NOT NULL,
    is_direct BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TEXT NOT NULL DEFAULT (NOW()::text),
    PRIMARY KEY (account_id, room_id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_account_rooms_account_id ON account_rooms(account_id);

-- schema_todos.rs

CREATE TABLE IF NOT EXISTS todos (
    id BIGSERIAL PRIMARY KEY,
    room_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    text TEXT NOT NULL,
    priority TEXT NOT NULL DEFAULT 'medium',
    due_at TIMESTAMPTZ,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(owner_id, room_id);

ALTER TABLE todos ADD COLUMN IF NOT EXISTS remind_before_minutes BIGINT;

-- schema_reminders.rs

CREATE TABLE IF NOT EXISTS reminders (
    id BIGSERIAL PRIMARY KEY,
    room_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    message TEXT NOT NULL,
    trigger_at TIMESTAMPTZ NOT NULL,
    recurrence TEXT NOT NULL DEFAULT 'none',
    status TEXT NOT NULL DEFAULT 'pending',
    retry_count BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);

ALTER TABLE reminders ADD COLUMN IF NOT EXISTS retry_count BIGINT NOT NULL DEFAULT 0;

ALTER TABLE reminders ADD COLUMN IF NOT EXISTS todo_id BIGINT;
ALTER TABLE reminders ADD COLUMN IF NOT EXISTS source_kind TEXT;
ALTER TABLE reminders ADD COLUMN IF NOT EXISTS source_id BIGINT;

DROP INDEX IF EXISTS idx_reminders_todo;

CREATE INDEX IF NOT EXISTS idx_reminders_source ON reminders(source_kind, source_id);

CREATE INDEX IF NOT EXISTS idx_reminders_owner_status_trigger ON reminders(owner_id, status, trigger_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_reminders_owner_room_status ON reminders(owner_id, room_id, status, trigger_at DESC);

-- schema_finance.rs

CREATE TABLE IF NOT EXISTS finance_categories (
    id BIGSERIAL PRIMARY KEY,
    owner_id TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_finance_categories_owner_name
 ON finance_categories(owner_id, name);

CREATE TABLE IF NOT EXISTS transactions (
    id BIGSERIAL PRIMARY KEY,
    room_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    direction TEXT NOT NULL,
    amount_minor BIGINT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    category_id BIGINT,
    note TEXT,
    source_kind TEXT,
    source_id BIGINT,
    occurred_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_transactions_owner_occurred
 ON transactions(owner_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);

ALTER TABLE transactions ADD COLUMN IF NOT EXISTS source_kind TEXT;
ALTER TABLE transactions ADD COLUMN IF NOT EXISTS source_id BIGINT;
CREATE INDEX IF NOT EXISTS idx_transactions_source ON transactions(source_kind, source_id);

CREATE TABLE IF NOT EXISTS finance_shares (
    owner_id TEXT PRIMARY KEY,
    token TEXT NOT NULL UNIQUE,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    title TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_finance_shares_token ON finance_shares(token);

-- schema_bills.rs

CREATE TABLE IF NOT EXISTS recurring_bills (
    id BIGSERIAL PRIMARY KEY,
    room_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    amount_minor BIGINT NOT NULL,
    currency TEXT NOT NULL,
    currency_exponent BIGINT NOT NULL DEFAULT 2,
    frequency TEXT NOT NULL DEFAULT 'monthly',
    due_day INTEGER,
    next_due_at TEXT NOT NULL,
    total_installments INTEGER,
    paid_installments INTEGER NOT NULL DEFAULT 0,
    auto_pay BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active',
    notes TEXT,
    remind_before_minutes BIGINT,
    remind_room_id TEXT,
    created_at TEXT DEFAULT (NOW()::text),
    updated_at TEXT DEFAULT (NOW()::text)
);

CREATE INDEX IF NOT EXISTS idx_recurring_bills_owner ON recurring_bills(owner_id, status);

CREATE INDEX IF NOT EXISTS idx_recurring_bills_due ON recurring_bills(owner_id, next_due_at);

ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS currency_exponent BIGINT NOT NULL DEFAULT 2;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS frequency TEXT NOT NULL DEFAULT 'monthly';
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS due_day INTEGER;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS next_due_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS total_installments INTEGER;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS remind_before_minutes BIGINT;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS remind_room_id TEXT;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS paid_installments INTEGER NOT NULL DEFAULT 0;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS auto_pay BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE recurring_bills ADD COLUMN IF NOT EXISTS notes TEXT;

-- schema_reactions.rs

CREATE TABLE IF NOT EXISTS bot_message_targets (
    event_id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id BIGINT NOT NULL,
    owner_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_bot_msg_targets_entity ON bot_message_targets(entity_type, entity_id);

-- Omitted data statements (rule 8: a fresh database has no rows to touch;
-- the live database already ran them).
--
-- schema.rs (create_schema, before schema_auth): for each of
-- todos, reminders, transactions, recurring_bills:
--   ALTER TABLE <table> RENAME COLUMN sender_mxid TO owner_id;
-- (also rule 5: every database this project ever built has already been
-- renamed, and a migration cannot swallow the error on a column that no
-- longer exists.)
--
-- schema_todos.rs:
--   UPDATE todos SET due_at = strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', ''))
--     WHERE due_at LIKE '% UTC' AND strftime(...) IS NOT NULL;
--   UPDATE todos SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';
--
-- schema_reminders.rs:
--   UPDATE reminders SET source_kind = 'todo', source_id = todo_id
--     WHERE todo_id IS NOT NULL AND source_id IS NULL;
--   UPDATE reminders SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';
--
-- schema_rooms.rs:
--   UPDATE reminders SET room_id = COALESCE((SELECT room_id FROM account_rooms
--     WHERE account_rooms.account_id = reminders.owner_id AND account_rooms.is_direct = TRUE LIMIT 1), 'web')
--     WHERE room_id = 'web_chat';
--   UPDATE todos SET room_id = COALESCE((SELECT room_id FROM account_rooms
--     WHERE account_rooms.account_id = todos.owner_id AND account_rooms.is_direct = TRUE LIMIT 1), 'web')
--     WHERE room_id = 'web_chat';
--   UPDATE ai_conversations SET room_id = COALESCE((SELECT room_id FROM account_rooms
--     WHERE account_rooms.account_id = ai_conversations.owner_id AND account_rooms.is_direct = TRUE LIMIT 1), 'web')
--     WHERE room_id = 'web_chat';
--   UPDATE transactions SET room_id = COALESCE((SELECT room_id FROM account_rooms
--     WHERE account_rooms.account_id = transactions.owner_id AND account_rooms.is_direct = TRUE LIMIT 1), 'web')
--     WHERE room_id = 'web_chat';
