//! Chat platforms behind one neutral contract.
//!
//! Every platform is a `ChannelAdapter`: it turns platform events into
//! `IncomingMessage`s for the shared pipeline and delivers `OutgoingMessage`s
//! the application produces. Business rules never live here; an adapter
//! translates, authenticates the sender and formats the reply (spec §3.2).
//!
//! - `message`: the platform-neutral message, reply and content types.
//! - `adapter`: the `ChannelAdapter` trait and its error type.
//! - `registry`: the set of configured adapters; routes outbound sends by the
//!   target's channel and supervises the adapters' `run` loops.
//! - `render`: pure renderers from the module HTML subset to a platform's format.
//! - `matrix`: the Matrix adapter.

pub mod adapter;
#[cfg(test)]
pub(crate) mod fake;
pub mod inbound;
pub mod matrix;
pub mod message;
pub mod registry;
pub mod render;
