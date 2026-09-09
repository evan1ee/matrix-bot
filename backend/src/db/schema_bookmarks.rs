use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS bookmarks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            url TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            author TEXT,
            image_url TEXT,
            domain TEXT NOT NULL,
            reader_content TEXT,
            reading_time_minutes INTEGER DEFAULT 1,
            is_favorite BOOLEAN NOT NULL DEFAULT 0,
            is_archived BOOLEAN NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookmarks_owner ON bookmarks(owner_id);")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookmarks_owner_archived ON bookmarks(owner_id, is_archived);")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookmarks_owner_favorite ON bookmarks(owner_id, is_favorite);")
        .execute(pool)
        .await?;

    Ok(())
}
