use tokio::sync::broadcast;

use crate::domain::events::SystemEvent;

/// Broadcasts `SystemEvent`s to all connected SSE clients via `tokio::sync::broadcast`.
#[derive(Clone)]
pub struct EventBroadcaster {
    sender: broadcast::Sender<SystemEvent>,
}

impl EventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn send(&self, event: SystemEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SystemEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;

    #[tokio::test]
    async fn test_event_broadcaster_pub_sub() {
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let ev = SystemEvent::ReminderCancelled {
            id: 42,
            room_id: "!room:2".to_string(),
            owner_id: AccountId::from_seed("@alice:a"),
        };
        broadcaster.send(ev);

        let received = rx.recv().await.unwrap();
        assert!(matches!(
            received,
            SystemEvent::ReminderCancelled { id: 42, .. }
        ));
    }
}
