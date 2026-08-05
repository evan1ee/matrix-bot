use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use tracing::info;

use crate::db::schema;

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self> {
        info!("Connecting to database at {}", database_url);

        let pool = if database_url.starts_with("sqlite://") {
            if !database_url.contains("mode=memory") {
                let raw_path = database_url
                    .trim_start_matches("sqlite://")
                    .split('?')
                    .next()
                    .unwrap_or("data/bot_data.db");
                let path = std::path::Path::new(raw_path);
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        info!("Creating database directory: {:?}", parent);
                        tokio::fs::create_dir_all(parent)
                            .await
                            .context("Failed to create database directory")?;
                    }
                }
            }

            let options = SqliteConnectOptions::from_str(database_url)?
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_secs(5));

            SqlitePoolOptions::new()
                .max_connections(max_connections)
                .connect_with(options)
                .await
                .context("Failed to connect to SQLite database")?
        } else {
            SqlitePoolOptions::new()
                .max_connections(max_connections)
                .connect(database_url)
                .await
                .context("Failed to connect to database")?
        };

        schema::run_migrations(&pool)
            .await
            .context("Failed to execute database migrations")?;

        Ok(Self { pool })
    }
}
