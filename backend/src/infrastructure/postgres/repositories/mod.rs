//! PostgreSQL repositories, one file per feature: plain `async fn`s over
//! `impl sqlx::PgExecutor`, compile-checked `query!` SQL, private row structs
//! mapped into domain types before they leave the file.

pub mod accounts;
pub mod bookmarks;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod life;
pub mod llm;
pub mod mcp;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todos;
pub mod webhooks;
