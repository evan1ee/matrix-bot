//! Inbound webhooks and webhook account management API endpoints.

pub mod dto;
pub mod inbound;
pub mod manage;

pub use crate::interface::http::webhooks_telegram::{
    telegram_send_message_get, telegram_send_message_post,
};
pub use dto::*;
pub use inbound::*;
pub use manage::*;
