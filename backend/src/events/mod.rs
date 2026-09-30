//! Event delivery. Until step 9's Task 10 this still holds the legacy
//! in-process broadcaster; the outbox runtime (listener, SSE hub,
//! dispatcher, purge) joins it from Task 3.

mod broadcaster;

pub use broadcaster::EventBroadcaster;
