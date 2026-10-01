//! The Matrix adapter: login and verification, the sync loop, inbound events
//! turned into neutral messages, and the outbound path. The chat modules know
//! none of it (`chat_modules_know_no_platform_sdk`).

pub mod adapter;
pub mod attachments;
pub mod client;
pub mod dispatcher;
pub mod handler;
pub mod handles;
pub mod inbound;
pub mod invite;
pub mod linking;
pub mod live_client;
pub mod media;
pub mod modules;
pub mod notify;
pub mod outbound;
pub mod quote;
pub mod reaction;
pub mod reaction_targets;
pub mod reply;
pub mod rooms;
pub mod self_verify;
pub mod stage;
pub mod sync_health;
pub mod sync_loop;
pub mod verification;
pub mod voice_upload;
