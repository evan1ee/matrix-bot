use serde::Serialize;
use tokio::sync::broadcast;

use crate::modules::reminder::models::Reminder;
use crate::modules::todo::models::TodoItem;

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
