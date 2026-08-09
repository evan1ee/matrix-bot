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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_connect_runs_migrations() {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM todos")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "migrations must have created the table");
    }

    /// `DATABASE_URL` points at `data/bot_data.db` by default, and that
    /// directory does not exist on a fresh checkout or in a fresh container.
    #[tokio::test]
    async fn test_creates_the_database_directory_if_missing() {
        let dir = std::env::temp_dir().join(format!("matrix-bot-pool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let nested = dir.join("nested");
        let url = format!("sqlite://{}/bot.db?mode=rwc", nested.display());

        let db = Database::connect(&url, 1).await.unwrap();
        assert!(nested.exists(), "the parent directory must be created");

        // Usable, not merely present.
        sqlx::query("INSERT INTO web_users (username, password_hash) VALUES ('a', 'h')")
            .execute(&db.pool)
            .await
            .unwrap();

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_rejects_a_non_sqlite_url() {
        // Postgres is not wired up: the driver is not even compiled in, so this
        // must fail loudly rather than appear to work.
        let result = Database::connect("postgres://localhost/whatever", 1).await;
        assert!(result.is_err(), "a non-sqlite URL must not silently succeed");
    }
}
