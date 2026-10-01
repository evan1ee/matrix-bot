//! PostgreSQL repositories, one file per feature: plain `async fn`s over
//! `impl sqlx::PgExecutor`, compile-checked `query!` SQL, private row structs
//! mapped into domain types before they leave the file.

pub mod accounts;
pub mod bookmarks;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod identities;
pub mod jobs;
pub mod life;
pub mod link_tokens;
pub mod llm;
pub mod mcp;
pub mod outbox;
pub mod portfolio;
pub mod reaction_targets;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todos;
pub mod webhooks;
