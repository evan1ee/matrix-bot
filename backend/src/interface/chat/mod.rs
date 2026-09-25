//! Chat front door, one file per feature: each feature's Matrix side — its
//! `BotModule` command handler, and any Matrix-specific lookups it needs —
//! calling that feature's application service. Holds no SQL
//! (`tests/architecture.rs`).

pub mod rooms;
