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

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_account_rooms_account_id ON account_rooms(account_id);",
    )
    .execute(pool)
    .await?;

    // Migrate legacy 'web_chat' records to the user's direct 1:1 room (if available) or fallback to 'web'
    let _ = sqlx::query(
        r#"
        UPDATE reminders
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = reminders.owner_id AND account_rooms.is_direct = 1 LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#,
    )
    .execute(pool)
    .await;

    let _ = sqlx::query(
        r#"
        UPDATE todos
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = todos.owner_id AND account_rooms.is_direct = 1 LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#,
    )
    .execute(pool)
    .await;

    let _ = sqlx::query(
        r#"
        UPDATE ai_conversations
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = ai_conversations.owner_id AND account_rooms.is_direct = 1 LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#,
    )
    .execute(pool)
    .await;

    let _ = sqlx::query(
        r#"
        UPDATE transactions
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = transactions.owner_id AND account_rooms.is_direct = 1 LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#,
    )
    .execute(pool)
    .await;

    Ok(())
}
