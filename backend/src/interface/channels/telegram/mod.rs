//! The Telegram adapter (spec §8.3, plan C).
//!
//! Bot API updates (messages, `/commands`, mentions, callback-query button
//! presses) are normalized into `IncomingMessage`s for the shared pipeline.
//! `teloxide-core` stays inside this module.
//!
//! - `commands`: `/name[@bot] args` parsing, the advertised command list.
//! - `inbound`: `Update` → `IncomingMessage` (pure).

pub mod commands;
pub mod inbound;
