use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS webhooks (
            id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            token_hash TEXT NOT NULL UNIQUE,
            token_prefix TEXT NOT NULL,
            default_room_id TEXT NOT NULL,
            is_active BIGINT NOT NULL DEFAULT 1,
            total_delivered_count BIGINT NOT NULL DEFAULT 0,
            failure_count BIGINT NOT NULL DEFAULT 0,
            last_triggered_at TIMESTAMP,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (owner_id) REFERENCES accounts(id) ON DELETE CASCADE
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_webhooks_owner ON webhooks(owner_id);"
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_webhooks_token_hash ON webhooks(token_hash);"
    )?;

    db_ddl!(
        pool,
        r#"
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
            next_attempt_at TIMESTAMP NOT NULL,
            last_error TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            delivered_at TIMESTAMP,
            FOREIGN KEY (webhook_id) REFERENCES webhooks(id) ON DELETE CASCADE
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_webhook_outbox_pending \
         ON webhook_outbox(status, next_attempt_at);"
    )?;

    Ok(())
}
