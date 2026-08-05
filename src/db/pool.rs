use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use tracing::info;

use crate::db::schema;

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self> {
        info!("Connecting to SQLite database at {}", database_url);

        // Ensure database file creation for sqlite
        if database_url.starts_with("sqlite://") && !database_url.contains("mode=memory") {
            let path = database_url
                .trim_start_matches("sqlite://")
                .split('?')
                .next()
                .unwrap_or("bot_data.db");
            if !std::path::Path::new(path).exists() {
                info!("Creating sqlite database file at {}", path);
                tokio::fs::File::create(path).await.context("Failed to create sqlite DB file")?;
            }
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("Failed to connect to SQLite database")?;

        schema::run_migrations(&pool)
            .await
            .context("Failed to execute database migrations")?;

        Ok(Self { pool })
    }
}
