//! Everything that talks to the outside world on the application's behalf:
//! the HTTP API (`http`), the Matrix connection (`channels`) and each
//! feature's chat side (`chat`). Interface code translates transport
//! payloads into application calls and back; it holds
//! no business rules and, outside a shrinking list of interim offenders
//! (`INTERFACE_SQL_ALLOW_LIST` in `tests/architecture.rs`), runs no SQL of
//! its own. A file leaves the list only when its SQL moves into
//! `application`/`infrastructure` — the test fails if a listed file no
//! longer offends, so the list can't go stale.

pub mod channels;
pub mod chat;
pub mod http;
pub mod mcp;
