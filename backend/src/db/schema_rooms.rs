use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS account_rooms (
            account_id TEXT NOT NULL,
            room_id TEXT NOT NULL,
            name TEXT NOT NULL,
            is_direct BOOLEAN NOT NULL DEFAULT 0,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (account_id, room_id),
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_account_rooms_account_id ON account_rooms(account_id);")
        .execute(pool)
        .await?;

    Ok(())
}
