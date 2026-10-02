//! Inbound webhooks and webhook account management API endpoints.

pub mod dto;
pub mod inbound;
pub mod manage;
pub mod telegram_compat;

pub use dto::*;
pub use inbound::*;
pub use manage::*;
pub use telegram_compat::{telegram_send_message_get, telegram_send_message_post};
