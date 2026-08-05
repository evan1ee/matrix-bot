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
    pub async fn connect(database_url: &str) -> Result<Self> {
        info!("Connecting to database at {}", database_url);

        let pool = if database_url.starts_with("sqlite://") {
            let options = SqliteConnectOptions::from_str(database_url)?
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_secs(5));

            SqlitePoolOptions::new()
                .max_connections(5)
                .connect_with(options)
                .await
                .context("Failed to connect to SQLite database")?
        } else {
            SqlitePoolOptions::new()
                .max_connections(5)
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
