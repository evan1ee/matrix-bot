//! The state the API server and the Matrix bot share.
//!
//! Both halves need the same three cells, and they are written from one side
//! and read from the other: the bot's login path fills in its access token and
//! client, which request handlers borrow to serve `/rooms` and the media proxy;
//! `!link` records a pending merge that the web UI later confirms.
//!
//! Created once in `main`, cloned into both. They used to be created *by* the
//! API server and returned as a bare three-tuple for `main` to thread onward as
//! positional `Option`s, which made the API server look like their owner when
//! it is only one of two users.

use std::sync::Arc;
use tokio::sync::RwLock;

use matrix_sdk::Client;

use crate::bot::linking::PendingMerges;

#[derive(Clone, Default)]
pub struct BotHandles {
    /// The **bot's** own Matrix access token.
    ///
    /// Written only by the bot's login path. A request handler must never write
    /// a caller's token here — that would silently repoint every
    /// bot-credentialed endpoint at whoever signed in most recently.
    pub access_token: Arc<RwLock<Option<String>>>,
    /// The bot's Matrix SDK client, under the same ownership rule.
    pub client: Arc<RwLock<Option<Client>>>,
    /// Accounts with a merge awaiting confirmation, `into -> from`.
    ///
    /// In memory rather than a table because it is a handoff between two halves
    /// of one flow: a stale entry should evaporate on restart rather than
    /// linger as a standing authorisation to absorb an account.
    pub pending_merges: Arc<PendingMerges>,
}
