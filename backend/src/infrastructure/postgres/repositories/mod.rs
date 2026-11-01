//! PostgreSQL repositories, one file per feature: plain `async fn`s over
//! `impl sqlx::PgExecutor`, compile-checked `query!` SQL, private row structs
//! mapped into domain types before they leave the file.

pub mod accounts;
pub mod agent;
pub mod app_settings;
pub mod bookmarks;
pub mod briefing_audio;
pub mod calendar;
pub mod config_audit;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod identities;
pub mod jobs;
pub mod knowledge;
pub mod life;
pub mod link_tokens;
pub mod llm;
pub mod mcp;
pub mod outbox;
pub mod portfolio;
pub mod provider_secrets;
pub mod reaction_targets;
pub mod recurring_bills;
pub mod reminder_sources;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todo_steps;
pub mod todos;
pub mod webhooks;
