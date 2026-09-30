//! TEMPORARY (deleted by step 9 Task 10): forwards events still sent on the
//! legacy broadcaster by services not yet converted to the outbox.

use std::sync::Arc;

use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::events::{EventBroadcaster, Hub};

pub(crate) async fn run(broadcaster: EventBroadcaster, hub: Hub, shutdown: CancellationToken) {
    let mut rx = broadcaster.subscribe();
    loop {
        let event = tokio::select! {
            _ = shutdown.cancelled() => return,
            msg = rx.recv() => match msg {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            },
        };
        if let Ok(json) = serde_json::to_string(&event) {
            hub.push(event.owner_id(), Arc::from(json));
        }
    }
}
