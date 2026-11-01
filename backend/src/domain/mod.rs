//! Business entities, value objects and invariants — pure Rust.
//!
//! Depends on nothing but std, chrono, uuid, serde and thiserror
//! (`tests/architecture.rs`).

pub mod accounts;
pub mod agent;
pub mod bookmarks;
pub mod briefing;
pub mod calendar;
pub mod channels;
pub mod chat;
pub mod config;
pub mod events;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod jobs;
pub mod knowledge;
pub mod life;
pub mod llm;
pub mod mcp;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod secrets;
pub mod settings;
pub mod storage;
pub mod tags;
pub mod todos;
pub mod webhooks;
