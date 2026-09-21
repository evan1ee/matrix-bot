use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS bookmarks (
            id {{PK}},
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
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_bookmarks_owner ON bookmarks(owner_id);"
    )?;

    crate::db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_bookmarks_owner_archived ON bookmarks(owner_id, is_archived);")?;

    crate::db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_bookmarks_owner_favorite ON bookmarks(owner_id, is_favorite);")?;

    Ok(())
}
