use serde::Serialize;
use tokio::sync::broadcast;

use crate::modules::finance::models::{Category, Transaction};
use crate::modules::reminder::models::Reminder;
use crate::modules::todo::models::TodoItem;

/// Real-time system events broadcast to SSE clients and internal subscribers.
///
/// Serialized as `{"type": "<VariantName>", "payload": …}` and sent as unnamed
/// SSE messages, so browser clients read them from `EventSource.onmessage` and
/// discriminate on `type`. Note this is the variant name verbatim
/// (`TodoCreated`), not a snake_case event name.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum SystemEvent {
    TodoCreated(TodoItem),
    TodoUpdated(TodoItem),
    TodoDeleted { id: i64, room_id: String, owner_id: String },
    ReminderCreated(Reminder),
    ReminderTriggered(Reminder),
    ReminderCancelled { id: i64, room_id: String, owner_id: String },
    TransactionCreated(Transaction),
    TransactionUpdated(Transaction),
    TransactionDeleted { id: i64, room_id: String, owner_id: String },
    CategoryCreated(Category),
    CategoryDeleted { id: i64, owner_id: String },
}

impl SystemEvent {
    /// The owner of the row this event concerns.
    ///
    /// The SSE stream delivers an event only to that owner, so this must agree
    /// exactly with the `sender_mxid` ownership key the REST handlers use. The
    /// variants carrying a full row read it from the payload; the deletion
    /// variants carry it explicitly because the row is already gone.
    pub fn owner_id(&self) -> &str {
        match self {
            SystemEvent::TodoCreated(item) | SystemEvent::TodoUpdated(item) => &item.sender_mxid,
            SystemEvent::ReminderCreated(rem) | SystemEvent::ReminderTriggered(rem) => {
                &rem.sender_mxid
            }
            SystemEvent::TodoDeleted { owner_id, .. } => owner_id,
            SystemEvent::ReminderCancelled { owner_id, .. } => owner_id,
            SystemEvent::TransactionCreated(tx) | SystemEvent::TransactionUpdated(tx) => {
                &tx.sender_mxid
            }
            SystemEvent::TransactionDeleted { owner_id, .. } => owner_id,
            SystemEvent::CategoryCreated(cat) => &cat.owner_id,
            SystemEvent::CategoryDeleted { owner_id, .. } => owner_id,
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

    /// Pins the SSE wire format. The web client discriminates on `type` inside
    /// the JSON body, so the variant names are a published contract.
    #[test]
    fn test_wire_format_is_tagged_by_variant_name() {
        let ev = SystemEvent::TodoDeleted {
            id: 1,
            room_id: "!room:1".to_string(),
            owner_id: "@alice:a".to_string(),
        };

        let json: serde_json::Value = serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert_eq!(json["type"], "TodoDeleted");
        assert_eq!(json["payload"]["id"], 1);
        assert_eq!(json["payload"]["owner_id"], "@alice:a");
    }

    fn todo_owned_by(owner: &str) -> TodoItem {
        TodoItem {
            id: 1,
            room_id: "!room:1".to_string(),
            sender_mxid: owner.to_string(),
            sender_name: "shared-display-name".to_string(),
            text: "task".to_string(),
            priority: "medium".to_string(),
            due_at: None,
            remind_before_minutes: None,
            completed: false,
            created_at: None,
        }
    }

    #[test]
    fn test_owner_id_reads_the_ownership_key_not_the_display_name() {
        // Both users share `sender_name`; only `sender_mxid` decides delivery.
        let mine = SystemEvent::TodoCreated(todo_owned_by("@web-evan:api"));
        let theirs = SystemEvent::TodoUpdated(todo_owned_by("@evan:mbm.one"));

        assert_eq!(mine.owner_id(), "@web-evan:api");
        assert_eq!(theirs.owner_id(), "@evan:mbm.one");
        assert_ne!(mine.owner_id(), theirs.owner_id());

        let deleted = SystemEvent::TodoDeleted {
            id: 7,
            room_id: "!room:1".to_string(),
            owner_id: "@evan:mbm.one".to_string(),
        };
        assert_eq!(deleted.owner_id(), "@evan:mbm.one");

        let cancelled = SystemEvent::ReminderCancelled {
            id: 8,
            room_id: "web".to_string(),
            owner_id: "@web-evan:api".to_string(),
        };
        assert_eq!(cancelled.owner_id(), "@web-evan:api");
    }

    #[tokio::test]
    async fn test_event_broadcaster_pub_sub() {
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let ev = SystemEvent::ReminderCancelled {
            id: 42,
            room_id: "!room:2".to_string(),
            owner_id: "@alice:a".to_string(),
        };
        broadcaster.send(ev);

        let received = rx.recv().await.unwrap();
        assert!(matches!(
            received,
            SystemEvent::ReminderCancelled { id: 42, .. }
        ));
    }
}
