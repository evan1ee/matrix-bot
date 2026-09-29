//! The Matrix client of the current sync generation, for code that is not
//! part of a sync session (the job handlers).
//!
//! The sync loop publishes each generation's client when that generation
//! starts and the guard clears it when `MatrixBot::run` returns by any path,
//! so a handler never sends through a dead generation's client: it sees
//! `None` and defers its job instead.

use std::sync::Arc;

use matrix_sdk::Client;
use tokio::sync::watch;

#[derive(Clone)]
pub struct MatrixHandle {
    cell: Arc<watch::Sender<Option<Client>>>,
}

impl Default for MatrixHandle {
    fn default() -> Self {
        Self {
            cell: Arc::new(watch::channel(None).0),
        }
    }
}

impl MatrixHandle {
    pub fn current(&self) -> Option<Client> {
        self.cell.borrow().clone()
    }

    /// Publishes this sync generation's client until the guard drops.
    #[must_use = "dropping the guard unpublishes the client"]
    pub fn publish(&self, client: Client) -> PublishedClient {
        self.cell.send_replace(Some(client));
        PublishedClient {
            cell: self.cell.clone(),
        }
    }
}

/// Clears the handle on drop.
pub struct PublishedClient {
    cell: Arc<watch::Sender<Option<Client>>>,
}

impl Drop for PublishedClient {
    fn drop(&mut self) {
        self.cell.send_replace(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Building a `Client` makes no request; never call a method that talks to it.
    async fn offline_client() -> Client {
        Client::builder()
            .homeserver_url("http://localhost:1")
            .build()
            .await
            .unwrap()
    }

    /// Given a published client, When the guard drops, Then the handle is empty.
    #[tokio::test]
    async fn the_client_is_visible_only_while_its_guard_lives() {
        let handle = MatrixHandle::default();
        assert!(handle.current().is_none());
        let guard = handle.publish(offline_client().await);
        assert!(handle.clone().current().is_some(), "clones share the cell");
        drop(guard);
        assert!(handle.current().is_none());
    }
}
