//! Delivery sinks for the outbox dispatcher (`crate::events`). The only
//! Matrix-aware part of the events runtime.

mod matrix_room;

pub use matrix_room::MatrixRoomSink;
