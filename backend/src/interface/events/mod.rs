//! Delivery sinks for the outbox dispatcher (`crate::events`): the only part
//! of the events runtime that knows channels.

mod channel_sink;

pub use channel_sink::ChannelSink;
