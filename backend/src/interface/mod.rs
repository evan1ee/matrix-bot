//! Everything that talks to the outside world on the application's behalf:
//! the HTTP API (`http`), chat channels (`channels`). Interface code
//! translates transport payloads into application calls and back; it holds
//! no business rules and runs no SQL of its own (see `tests/architecture.rs`).

pub mod channels;
pub mod http;
