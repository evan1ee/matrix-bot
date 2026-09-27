use serde::Serialize;
use tokio::sync::broadcast;

use crate::domain::settings::UserSettings;
use crate::modules::finance::models::{Category, Transaction};
use crate::modules::portfolio::audit::PortfolioAuditEntry;
use crate::modules::portfolio::models::Holding;
use crate::modules::recurring_bills::models::RecurringBill;
use crate::modules::reminder::models::Reminder;
use crate::modules::todo::models::TodoItem;
use crate::modules::webhook::models::Webhook;

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
    TodoDeleted {
        id: i64,
        room_id: String,
        owner_id: String,
    },
    ReminderCreated(Reminder),
    ReminderUpdated(Reminder),
    ReminderTriggered(Reminder),
    ReminderCancelled {
        id: i64,
        room_id: String,
        owner_id: String,
    },
    ReminderDeleted {
        id: i64,
        room_id: String,
        owner_id: String,
    },
    ReminderHistoryCleared {
        owner_id: String,
    },
    TransactionCreated(Transaction),
    TransactionUpdated(Transaction),
    TransactionDeleted {
        id: i64,
        room_id: String,
        owner_id: String,
    },
    CategoryCreated(Category),
    CategoryDeleted {
        id: i64,
        owner_id: String,
    },
    /// Carries `owner_id` alongside the settings because `UserSettings` holds
    /// only the two preferences — the row is keyed by owner, not identified by
    /// its contents.
    SettingsUpdated {
        owner_id: String,
        settings: UserSettings,
    },
    PortfolioHoldingCreated(Holding),
    PortfolioHoldingUpdated(Holding),
    PortfolioHoldingDeleted {
        id: i64,
        owner_id: String,
    },
    /// Carries `owner_id`/`currency` alongside the balance because a
    /// `CashBalance` for a never-deposited-into currency (`id: 0`) is not a
    /// real row to key off — the event fields are what the SSE stream and the
    /// frontend cache actually key on.
    PortfolioCashUpdated {
        owner_id: String,
        currency: String,
        balance_minor: i64,
    },
    PortfolioAuditLogged(PortfolioAuditEntry),
    PortfolioAlertCreated(crate::modules::portfolio::alerts::PriceAlert),
    PortfolioAlertTriggered(crate::modules::portfolio::alerts::PriceAlert),
    PortfolioAlertDeleted {
        id: i64,
        owner_id: String,
    },
    RecurringBillCreated(RecurringBill),
    RecurringBillUpdated(RecurringBill),
    RecurringBillDeleted {
        id: i64,
        owner_id: String,
    },
    RecurringBillPaid {
        id: i64,
        owner_id: String,
        next_due_at: String,
    },
    WebhookCreated(Webhook),
    WebhookUpdated(Webhook),
    WebhookDeleted {
        id: String,
        owner_id: String,
    },
    WebhookTriggered {
        id: String,
        owner_id: String,
        outbox_id: String,
        room_id: String,
        severity: String,
    },
    AiConversationUpdated {
        id: String,
        owner_id: String,
    },
    AiConversationDeleted {
        id: String,
        owner_id: String,
    },
    AiConfigUpdated {
        owner_id: String,
    },
    FootprintCreated(crate::domain::footprints::Footprint),
    FootprintUpdated(crate::domain::footprints::Footprint),
    FootprintDeleted {
        id: i64,
        owner_id: String,
    },
    HabitCreated(crate::domain::habits::Habit),
    HabitUpdated(crate::domain::habits::Habit),
    HabitDeleted {
        id: i64,
        owner_id: String,
    },
    HabitLogToggled {
        habit_id: i64,
        owner_id: String,
        log_date: String,
        completed: bool,
    },
    LifeProfileUpdated {
        owner_id: String,
        profile: crate::modules::life::models::LifeProfile,
    },
    LifeDayLogged(crate::modules::life::models::LifeDayLog),
    LifeDayCleared {
        owner_id: String,
        date: String,
    },
    BookmarkCreated(crate::domain::bookmarks::Bookmark),
    BookmarkUpdated(crate::domain::bookmarks::Bookmark),
    BookmarkDeleted {
        id: i64,
        owner_id: String,
    },
    MorningBriefingSent {
        owner_id: String,
        delivered_at: String,
        summary: String,
    },
}

impl SystemEvent {
    /// The owner who is allowed to receive this event over SSE.
    ///
    /// Every event must have an owner: the channel is process-wide and the SSE
    /// handler delivers an event only to the client whose session matches this
    /// value, so an event with no owner would either leak to everyone or never
    /// be delivered.
    pub fn owner_id(&self) -> &str {
        match self {
            SystemEvent::TodoCreated(item) | SystemEvent::TodoUpdated(item) => &item.owner_id,
            SystemEvent::ReminderCreated(rem)
            | SystemEvent::ReminderUpdated(rem)
            | SystemEvent::ReminderTriggered(rem) => &rem.owner_id,
            SystemEvent::TodoDeleted { owner_id, .. } => owner_id,
            SystemEvent::ReminderCancelled { owner_id, .. }
            | SystemEvent::ReminderDeleted { owner_id, .. }
            | SystemEvent::ReminderHistoryCleared { owner_id } => owner_id,
            SystemEvent::TransactionCreated(tx) | SystemEvent::TransactionUpdated(tx) => {
                &tx.owner_id
            }
            SystemEvent::TransactionDeleted { owner_id, .. } => owner_id,
            SystemEvent::CategoryCreated(cat) => &cat.owner_id,
            SystemEvent::CategoryDeleted { owner_id, .. } => owner_id,
            SystemEvent::SettingsUpdated { owner_id, .. } => owner_id,
            SystemEvent::PortfolioHoldingCreated(h) | SystemEvent::PortfolioHoldingUpdated(h) => {
                &h.owner_id
            }
            SystemEvent::PortfolioHoldingDeleted { owner_id, .. } => owner_id,
            SystemEvent::PortfolioCashUpdated { owner_id, .. } => owner_id,
            SystemEvent::PortfolioAuditLogged(entry) => &entry.owner_id,
            SystemEvent::PortfolioAlertCreated(a) | SystemEvent::PortfolioAlertTriggered(a) => {
                &a.owner_id
            }
            SystemEvent::PortfolioAlertDeleted { owner_id, .. } => owner_id,
            SystemEvent::RecurringBillCreated(b) | SystemEvent::RecurringBillUpdated(b) => {
                &b.owner_id
            }
            SystemEvent::RecurringBillDeleted { owner_id, .. }
            | SystemEvent::RecurringBillPaid { owner_id, .. } => owner_id,
            SystemEvent::WebhookCreated(w) | SystemEvent::WebhookUpdated(w) => &w.owner_id,
            SystemEvent::WebhookDeleted { owner_id, .. }
            | SystemEvent::WebhookTriggered { owner_id, .. } => owner_id,
            SystemEvent::AiConversationUpdated { owner_id, .. }
            | SystemEvent::AiConversationDeleted { owner_id, .. }
            | SystemEvent::AiConfigUpdated { owner_id } => owner_id,
            SystemEvent::FootprintCreated(f) | SystemEvent::FootprintUpdated(f) => &f.owner_id,
            SystemEvent::FootprintDeleted { owner_id, .. } => owner_id,
            SystemEvent::HabitCreated(h) | SystemEvent::HabitUpdated(h) => &h.owner_id,
            SystemEvent::HabitDeleted { owner_id, .. }
            | SystemEvent::HabitLogToggled { owner_id, .. } => owner_id,
            SystemEvent::LifeProfileUpdated { owner_id, .. }
            | SystemEvent::LifeDayCleared { owner_id, .. } => owner_id,
            SystemEvent::LifeDayLogged(l) => &l.owner_id,
            SystemEvent::BookmarkCreated(b) | SystemEvent::BookmarkUpdated(b) => &b.owner_id,
            SystemEvent::BookmarkDeleted { owner_id, .. } => owner_id,
            SystemEvent::MorningBriefingSent { owner_id, .. } => owner_id,
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

        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&ev).unwrap()).unwrap();
        assert_eq!(json["type"], "TodoDeleted");
        assert_eq!(json["payload"]["id"], 1);
        assert_eq!(json["payload"]["owner_id"], "@alice:a");
    }

    fn todo_owned_by(owner: &str) -> TodoItem {
        TodoItem {
            id: 1,
            room_id: "!room:1".to_string(),
            owner_id: owner.to_string(),
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
        // Both users share `sender_name`; only `owner_id` decides delivery.
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
