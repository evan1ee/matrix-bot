use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS account_rooms (
            account_id TEXT NOT NULL,
            room_id TEXT NOT NULL,
            name TEXT NOT NULL,
            is_direct BOOLEAN NOT NULL DEFAULT FALSE,
            updated_at TEXT NOT NULL DEFAULT {{NOW_TEXT}},
            PRIMARY KEY (account_id, room_id),
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_account_rooms_account_id ON account_rooms(account_id);"
    )?;

    // Migrate legacy 'web_chat' records to the user's direct 1:1 room (if available) or fallback to 'web'
    let _ = crate::db_ddl!(
        pool,
        r#"
        UPDATE reminders
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = reminders.owner_id AND account_rooms.is_direct = TRUE LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#
    );

    let _ = crate::db_ddl!(
        pool,
        r#"
        UPDATE todos
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = todos.owner_id AND account_rooms.is_direct = TRUE LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#
    );

    let _ = crate::db_ddl!(
        pool,
        r#"
        UPDATE ai_conversations
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = ai_conversations.owner_id AND account_rooms.is_direct = TRUE LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#
    );

    let _ = crate::db_ddl!(
        pool,
        r#"
        UPDATE transactions
        SET room_id = COALESCE(
            (SELECT room_id FROM account_rooms WHERE account_rooms.account_id = transactions.owner_id AND account_rooms.is_direct = TRUE LIMIT 1),
            'web'
        )
        WHERE room_id = 'web_chat';
        "#
    );

    Ok(())
}
