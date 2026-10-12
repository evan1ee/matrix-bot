//! The state the API server and the Matrix bot share.
//!
//! Both halves need the same cells, and they are written from one side and
//! read from the other: the bot's login path fills in its access token and
//! client, which request handlers borrow to serve `/rooms` and the media proxy.
//!
//! Created once in `main`, cloned into both. They used to be created *by* the
//! API server and returned as a bare tuple for `main` to thread onward as
//! positional `Option`s, which made the API server look like their owner when
//! it is only one of two users.

use std::sync::Arc;
use tokio::sync::RwLock;

use matrix_sdk::Client;

use crate::interface::channels::matrix::live_client::MatrixHandle;
use crate::interface::channels::matrix::sync_health::SyncHealth;

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
    /// When the sync loop last completed a sync, read by `/health/ready`.
    pub sync_health: SyncHealth,
    /// This generation's client for the job handlers; empty while disconnected.
    pub matrix: MatrixHandle,
    /// Outbound channel registry for security announcements from HTTP handlers.
    pub outbound: Option<std::sync::Weak<crate::interface::channels::registry::ChannelRegistry>>,
}
