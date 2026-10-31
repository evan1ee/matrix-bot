//! Email channel adapter: IMAP inbound, DMARC-gated, SMTP outbound via lettre.

pub mod adapter;
pub mod attachments;
pub mod imap;
pub mod inbound;
pub mod outbound;

pub use adapter::EmailAdapter;
