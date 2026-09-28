//! Business entities, value objects and invariants — pure Rust.
//!
//! Depends on nothing but std, chrono, uuid, serde and thiserror
//! (`tests/architecture.rs`). Features move here one at a time from
//! `modules/<feature>/models.rs` as the per-feature plan reaches them.

pub mod accounts;
pub mod bookmarks;
pub mod chat;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todos;
pub mod webhooks;
