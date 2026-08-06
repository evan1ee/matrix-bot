use serde::Serialize;
use tokio::sync::broadcast;

use crate::modules::reminder::models::Reminder;
use crate::modules::todo::models::TodoItem;

/// Real-time system events broadcast to SSE clients and internal subscribers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum SystemEvent {
    TodoCreated(TodoItem),
    TodoUpdated(TodoItem),
    TodoDeleted { id: i64, room_id: String },
    ReminderCreated(Reminder),
    ReminderTriggered(Reminder),
    ReminderCancelled { id: i64, room_id: String },
}

impl SystemEvent {
    /// Returns a lowercase event type string for SSE `event:` field discrimination.
    #[allow(dead_code)]
    pub fn event_type(&self) -> &'static str {
        match self {
            SystemEvent::TodoCreated(_) => "todo_created",
            SystemEvent::TodoUpdated(_) => "todo_updated",
            SystemEvent::TodoDeleted { .. } => "todo_deleted",
            SystemEvent::ReminderCreated(_) => "reminder_created",
            SystemEvent::ReminderTriggered(_) => "reminder_triggered",
            SystemEvent::ReminderCancelled { .. } => "reminder_cancelled",
        }
    }
}

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

    #[test]
    fn test_system_event_types() {
        let ev = SystemEvent::TodoDeleted {
            id: 1,
            room_id: "!room:1".to_string(),
        };
        assert_eq!(ev.event_type(), "todo_deleted");
    }

    #[tokio::test]
    async fn test_event_broadcaster_pub_sub() {
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let ev = SystemEvent::ReminderCancelled {
            id: 42,
            room_id: "!room:2".to_string(),
        };
        broadcaster.send(ev);

        let received = rx.recv().await.unwrap();
        assert_eq!(received.event_type(), "reminder_cancelled");
    }
}
