//! Business entities, value objects and invariants — pure Rust.
//!
//! Depends on nothing but std, chrono, uuid, serde and thiserror
//! (`tests/architecture.rs`). Features move here one at a time from
//! `modules/<feature>/models.rs` as the per-feature plan reaches them.

pub mod rooms;
