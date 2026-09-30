use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;
use crate::domain::finance::{Category, Transaction};
use crate::domain::portfolio::{Holding, PortfolioAuditEntry, PriceAlert};
use crate::domain::recurring_bills::RecurringBill;
use crate::domain::reminders::Reminder;
use crate::domain::settings::UserSettings;
use crate::domain::todos::TodoItem;
use crate::domain::webhooks::Webhook;

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
        owner_id: AccountId,
    },
    ReminderCreated(Reminder),
    ReminderUpdated(Reminder),
    ReminderTriggered(Reminder),
    ReminderCancelled {
        id: i64,
        room_id: String,
        owner_id: AccountId,
    },
    ReminderDeleted {
        id: i64,
        room_id: String,
        owner_id: AccountId,
    },
    ReminderHistoryCleared {
        owner_id: AccountId,
    },
    TransactionCreated(Transaction),
    TransactionUpdated(Transaction),
    TransactionDeleted {
        id: i64,
        room_id: String,
        owner_id: AccountId,
    },
    CategoryCreated(Category),
    CategoryDeleted {
        id: i64,
        owner_id: AccountId,
    },
    /// Carries `owner_id` alongside the settings because `UserSettings` holds
    /// only the two preferences — the row is keyed by owner, not identified by
    /// its contents.
    SettingsUpdated {
        owner_id: AccountId,
        settings: UserSettings,
    },
    PortfolioHoldingCreated(Holding),
    PortfolioHoldingUpdated(Holding),
    PortfolioHoldingDeleted {
        id: i64,
        owner_id: AccountId,
    },
    /// Carries `owner_id`/`currency` alongside the balance because a
    /// `CashBalance` for a never-deposited-into currency (`id: 0`) is not a
    /// real row to key off — the event fields are what the SSE stream and the
    /// frontend cache actually key on.
    PortfolioCashUpdated {
        owner_id: AccountId,
        currency: String,
        balance_minor: i64,
    },
    PortfolioAuditLogged(PortfolioAuditEntry),
    PortfolioAlertCreated(PriceAlert),
    PortfolioAlertTriggered(PriceAlert),
    PortfolioAlertDeleted {
        id: i64,
        owner_id: AccountId,
    },
    RecurringBillCreated(RecurringBill),
    RecurringBillUpdated(RecurringBill),
    RecurringBillDeleted {
        id: i64,
        owner_id: AccountId,
    },
    RecurringBillPaid {
        id: i64,
        owner_id: AccountId,
        next_due_at: String,
    },
    WebhookCreated(Webhook),
    WebhookUpdated(Webhook),
    WebhookDeleted {
        id: String,
        owner_id: AccountId,
    },
    WebhookTriggered(WebhookAlert),
    AiConversationUpdated {
        id: String,
        owner_id: AccountId,
    },
    AiConversationDeleted {
        id: String,
        owner_id: AccountId,
    },
    AiConfigUpdated {
        owner_id: AccountId,
    },
    FootprintCreated(crate::domain::footprints::Footprint),
    FootprintUpdated(crate::domain::footprints::Footprint),
    FootprintDeleted {
        id: i64,
        owner_id: AccountId,
    },
    HabitCreated(crate::domain::habits::Habit),
    HabitUpdated(crate::domain::habits::Habit),
    HabitDeleted {
        id: i64,
        owner_id: AccountId,
    },
    HabitLogToggled {
        habit_id: i64,
        owner_id: AccountId,
        log_date: String,
        completed: bool,
    },
    LifeProfileUpdated {
        owner_id: AccountId,
        profile: crate::domain::life::LifeProfile,
    },
    LifeDayLogged(crate::domain::life::LifeDayLog),
    LifeDayCleared {
        owner_id: AccountId,
        date: String,
    },
    BookmarkCreated(crate::domain::bookmarks::Bookmark),
    BookmarkUpdated(crate::domain::bookmarks::Bookmark),
    BookmarkDeleted {
        id: i64,
        owner_id: AccountId,
    },
    MorningBriefingSent {
        owner_id: AccountId,
        delivered_at: String,
        summary: String,
    },
}

impl SystemEvent {
    /// The owner who is allowed to receive this event over SSE.
    ///
    /// Every event must have an owner: the SSE hub delivers an event only to
    /// the client whose session matches this value, so an event with no owner
    /// would either leak to everyone or never be delivered. Arms are explicit
    /// so a new variant fails to compile until its owner is decided.
    pub fn owner_id(&self) -> &AccountId {
        match self {
            SystemEvent::TodoCreated(x) => &x.owner_id,
            SystemEvent::TodoUpdated(x) => &x.owner_id,
            SystemEvent::TodoDeleted { owner_id, .. } => owner_id,
            SystemEvent::ReminderCreated(x) => &x.owner_id,
            SystemEvent::ReminderUpdated(x) => &x.owner_id,
            SystemEvent::ReminderTriggered(x) => &x.owner_id,
            SystemEvent::ReminderCancelled { owner_id, .. } => owner_id,
            SystemEvent::ReminderDeleted { owner_id, .. } => owner_id,
            SystemEvent::ReminderHistoryCleared { owner_id, .. } => owner_id,
            SystemEvent::TransactionCreated(x) => &x.owner_id,
            SystemEvent::TransactionUpdated(x) => &x.owner_id,
            SystemEvent::TransactionDeleted { owner_id, .. } => owner_id,
            SystemEvent::CategoryCreated(x) => &x.owner_id,
            SystemEvent::CategoryDeleted { owner_id, .. } => owner_id,
            SystemEvent::SettingsUpdated { owner_id, .. } => owner_id,
            SystemEvent::PortfolioHoldingCreated(x) => &x.owner_id,
            SystemEvent::PortfolioHoldingUpdated(x) => &x.owner_id,
            SystemEvent::PortfolioHoldingDeleted { owner_id, .. } => owner_id,
            SystemEvent::PortfolioCashUpdated { owner_id, .. } => owner_id,
            SystemEvent::PortfolioAuditLogged(x) => &x.owner_id,
            SystemEvent::PortfolioAlertCreated(x) => &x.owner_id,
            SystemEvent::PortfolioAlertTriggered(x) => &x.owner_id,
            SystemEvent::PortfolioAlertDeleted { owner_id, .. } => owner_id,
            SystemEvent::RecurringBillCreated(x) => &x.owner_id,
            SystemEvent::RecurringBillUpdated(x) => &x.owner_id,
            SystemEvent::RecurringBillDeleted { owner_id, .. } => owner_id,
            SystemEvent::RecurringBillPaid { owner_id, .. } => owner_id,
            SystemEvent::WebhookCreated(x) => &x.owner_id,
            SystemEvent::WebhookUpdated(x) => &x.owner_id,
            SystemEvent::WebhookDeleted { owner_id, .. } => owner_id,
            SystemEvent::WebhookTriggered(x) => &x.owner_id,
            SystemEvent::AiConversationUpdated { owner_id, .. } => owner_id,
            SystemEvent::AiConversationDeleted { owner_id, .. } => owner_id,
            SystemEvent::AiConfigUpdated { owner_id, .. } => owner_id,
            SystemEvent::FootprintCreated(x) => &x.owner_id,
            SystemEvent::FootprintUpdated(x) => &x.owner_id,
            SystemEvent::FootprintDeleted { owner_id, .. } => owner_id,
            SystemEvent::HabitCreated(x) => &x.owner_id,
            SystemEvent::HabitUpdated(x) => &x.owner_id,
            SystemEvent::HabitDeleted { owner_id, .. } => owner_id,
            SystemEvent::HabitLogToggled { owner_id, .. } => owner_id,
            SystemEvent::LifeProfileUpdated { owner_id, .. } => owner_id,
            SystemEvent::LifeDayLogged(x) => &x.owner_id,
            SystemEvent::LifeDayCleared { owner_id, .. } => owner_id,
            SystemEvent::BookmarkCreated(x) => &x.owner_id,
            SystemEvent::BookmarkUpdated(x) => &x.owner_id,
            SystemEvent::BookmarkDeleted { owner_id, .. } => owner_id,
            SystemEvent::MorningBriefingSent { owner_id, .. } => owner_id,
        }
    }

    /// The variant name: the serde `type` tag and `outbox_events.event_type`.
    /// No wildcard arm, so a new variant must be named here.
    pub fn event_type(&self) -> &'static str {
        match self {
            SystemEvent::TodoCreated(_) => "TodoCreated",
            SystemEvent::TodoUpdated(_) => "TodoUpdated",
            SystemEvent::TodoDeleted { .. } => "TodoDeleted",
            SystemEvent::ReminderCreated(_) => "ReminderCreated",
            SystemEvent::ReminderUpdated(_) => "ReminderUpdated",
            SystemEvent::ReminderTriggered(_) => "ReminderTriggered",
            SystemEvent::ReminderCancelled { .. } => "ReminderCancelled",
            SystemEvent::ReminderDeleted { .. } => "ReminderDeleted",
            SystemEvent::ReminderHistoryCleared { .. } => "ReminderHistoryCleared",
            SystemEvent::TransactionCreated(_) => "TransactionCreated",
            SystemEvent::TransactionUpdated(_) => "TransactionUpdated",
            SystemEvent::TransactionDeleted { .. } => "TransactionDeleted",
            SystemEvent::CategoryCreated(_) => "CategoryCreated",
            SystemEvent::CategoryDeleted { .. } => "CategoryDeleted",
            SystemEvent::SettingsUpdated { .. } => "SettingsUpdated",
            SystemEvent::PortfolioHoldingCreated(_) => "PortfolioHoldingCreated",
            SystemEvent::PortfolioHoldingUpdated(_) => "PortfolioHoldingUpdated",
            SystemEvent::PortfolioHoldingDeleted { .. } => "PortfolioHoldingDeleted",
            SystemEvent::PortfolioCashUpdated { .. } => "PortfolioCashUpdated",
            SystemEvent::PortfolioAuditLogged(_) => "PortfolioAuditLogged",
            SystemEvent::PortfolioAlertCreated(_) => "PortfolioAlertCreated",
            SystemEvent::PortfolioAlertTriggered(_) => "PortfolioAlertTriggered",
            SystemEvent::PortfolioAlertDeleted { .. } => "PortfolioAlertDeleted",
            SystemEvent::RecurringBillCreated(_) => "RecurringBillCreated",
            SystemEvent::RecurringBillUpdated(_) => "RecurringBillUpdated",
            SystemEvent::RecurringBillDeleted { .. } => "RecurringBillDeleted",
            SystemEvent::RecurringBillPaid { .. } => "RecurringBillPaid",
            SystemEvent::WebhookCreated(_) => "WebhookCreated",
            SystemEvent::WebhookUpdated(_) => "WebhookUpdated",
            SystemEvent::WebhookDeleted { .. } => "WebhookDeleted",
            SystemEvent::WebhookTriggered(_) => "WebhookTriggered",
            SystemEvent::AiConversationUpdated { .. } => "AiConversationUpdated",
            SystemEvent::AiConversationDeleted { .. } => "AiConversationDeleted",
            SystemEvent::AiConfigUpdated { .. } => "AiConfigUpdated",
            SystemEvent::FootprintCreated(_) => "FootprintCreated",
            SystemEvent::FootprintUpdated(_) => "FootprintUpdated",
            SystemEvent::FootprintDeleted { .. } => "FootprintDeleted",
            SystemEvent::HabitCreated(_) => "HabitCreated",
            SystemEvent::HabitUpdated(_) => "HabitUpdated",
            SystemEvent::HabitDeleted { .. } => "HabitDeleted",
            SystemEvent::HabitLogToggled { .. } => "HabitLogToggled",
            SystemEvent::LifeProfileUpdated { .. } => "LifeProfileUpdated",
            SystemEvent::LifeDayLogged(_) => "LifeDayLogged",
            SystemEvent::LifeDayCleared { .. } => "LifeDayCleared",
            SystemEvent::BookmarkCreated(_) => "BookmarkCreated",
            SystemEvent::BookmarkUpdated(_) => "BookmarkUpdated",
            SystemEvent::BookmarkDeleted { .. } => "BookmarkDeleted",
            SystemEvent::MorningBriefingSent { .. } => "MorningBriefingSent",
        }
    }

    /// Durable destinations this event must be delivered to.
    pub fn destinations(&self) -> Vec<Destination> {
        match self {
            SystemEvent::WebhookTriggered(alert) => {
                vec![Destination::MatrixRoom(alert.room_id.clone())]
            }
            _ => Vec::new(),
        }
    }
}

/// The content of one accepted inbound webhook alert — the single record of
/// it, from which the Matrix delivery and the webhook logs are both read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookAlert {
    /// The webhook's id (`whk_…`).
    pub id: String,
    pub owner_id: AccountId,
    pub room_id: String,
    pub severity: String,
    pub title: Option<String>,
    pub html_body: String,
    pub plain_body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeliveryId(pub i64);

impl std::fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a durable copy of an event must be delivered. SSE is not a
/// destination: every instance pushes committed events to its own clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    MatrixRoom(String),
}

impl Destination {
    const MATRIX: &'static str = "matrix:";

    /// The `outbox_deliveries.destination` value.
    pub fn as_key(&self) -> String {
        match self {
            Self::MatrixRoom(room) => format!("{}{room}", Self::MATRIX),
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        let room = key.strip_prefix(Self::MATRIX)?;
        (!room.is_empty()).then(|| Self::MatrixRoom(room.to_string()))
    }
}

/// What a failed delivery attempt leads to. `attempts` counts the attempt
/// that just failed (it is incremented when the delivery is claimed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStep {
    /// Retry after `domain::webhooks::retry_delay(n)`.
    RetryAfterAttempts(i64),
    Fail,
}

pub fn next_delivery_step(attempts: i32, max_attempts: i32) -> DeliveryStep {
    if attempts >= max_attempts {
        DeliveryStep::Fail
    } else {
        DeliveryStep::RetryAfterAttempts(i64::from(attempts - 1).max(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;

    fn alert(room: &str) -> WebhookAlert {
        WebhookAlert {
            id: "whk_1".to_string(),
            owner_id: AccountId::from_seed("acct_alice"),
            room_id: room.to_string(),
            severity: "info".to_string(),
            title: Some("t".to_string()),
            html_body: "<b>x</b>".to_string(),
            plain_body: "x".to_string(),
        }
    }

    /// Pins the SSE wire format: the web client discriminates on `type`.
    #[test]
    fn wire_format_is_tagged_by_variant_name_with_an_account_owner() {
        let owner = AccountId::from_seed("acct_alice");
        let ev = SystemEvent::TodoDeleted {
            id: 1,
            room_id: "!room:1".to_string(),
            owner_id: owner,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "TodoDeleted");
        assert_eq!(json["payload"]["id"], 1);
        assert_eq!(json["payload"]["owner_id"], owner.to_string());
    }

    #[test]
    fn webhook_triggered_serialises_its_alert_as_the_payload_object() {
        let json = serde_json::to_value(SystemEvent::WebhookTriggered(alert("!r:1"))).unwrap();
        assert_eq!(json["type"], "WebhookTriggered");
        assert_eq!(json["payload"]["id"], "whk_1");
        assert_eq!(json["payload"]["room_id"], "!r:1");
        assert_eq!(json["payload"]["plain_body"], "x");
        let back: WebhookAlert = serde_json::from_value(json["payload"].clone()).unwrap();
        assert_eq!(back, alert("!r:1"));
    }

    fn sample_todo(owner: AccountId) -> TodoItem {
        TodoItem {
            id: 1,
            room_id: "!room:1".to_string(),
            owner_id: owner,
            sender_name: "shared-display-name".to_string(),
            text: "task".to_string(),
            priority: crate::domain::todos::TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            completed: false,
            created_at: None,
        }
    }

    /// One value of every variant, so a typo in any `event_type` arm fails.
    fn one_of_each(o: AccountId) -> Vec<SystemEvent> {
        use crate::domain::{
            bookmarks::Bookmark,
            finance::{Category, Direction, Transaction},
            footprints::Footprint,
            habits::Habit,
            life::{LifeDayLog, LifeProfile},
            portfolio::{Holding, PortfolioAuditEntry, PriceAlert},
            recurring_bills::RecurringBill,
            reminders::{RecurrencePattern, Reminder, ReminderStatus},
        };
        let now = chrono::Utc::now();
        let s = || "x".to_string();
        let reminder = || Reminder {
            id: 1,
            room_id: s(),
            owner_id: o,
            sender_name: s(),
            message: s(),
            trigger_at: now,
            recurrence: RecurrencePattern::None,
            status: ReminderStatus::Pending,
            retry_count: 0,
            source_kind: None,
            source_id: None,
            created_at: None,
        };
        let transaction = || Transaction {
            id: 1,
            room_id: s(),
            owner_id: o,
            sender_name: s(),
            direction: Direction::Expense,
            amount_minor: 1,
            currency: s(),
            currency_exponent: 2,
            category_id: None,
            category_name: None,
            note: None,
            source_kind: None,
            source_id: None,
            occurred_at: now,
            created_at: None,
        };
        let holding = || Holding {
            id: 1,
            owner_id: o,
            symbol: s(),
            quantity_minor: 1,
            avg_cost_minor: 1,
            currency: s(),
            currency_exponent: 2,
            current_price_minor: None,
            price_updated_at: None,
            note: None,
            created_at: None,
            updated_at: None,
        };
        let price_alert = || PriceAlert {
            id: 1,
            owner_id: o,
            symbol: s(),
            target_price_minor: 1,
            currency: s(),
            currency_exponent: 2,
            condition: s(),
            is_triggered: false,
            triggered_at: None,
            created_at: now,
        };
        let bill = || RecurringBill {
            id: 1,
            room_id: s(),
            owner_id: o,
            sender_name: s(),
            name: s(),
            category: s(),
            amount_minor: 1,
            currency: s(),
            currency_exponent: 2,
            frequency: s(),
            due_day: None,
            next_due_at: s(),
            total_installments: None,
            paid_installments: 0,
            auto_pay: false,
            status: s(),
            notes: None,
            remind_before_minutes: None,
            remind_room_id: None,
            created_at: None,
            updated_at: None,
        };
        let webhook = || Webhook {
            id: s(),
            owner_id: o,
            name: s(),
            token_prefix: s(),
            default_room_id: s(),
            is_active: true,
            total_delivered_count: 0,
            failure_count: 0,
            last_triggered_at: None,
            created_at: now,
        };
        let footprint = || Footprint {
            id: 1,
            owner_id: o,
            title: s(),
            latitude: 0.0,
            longitude: 0.0,
            country: None,
            city: None,
            address: None,
            arrival_date: s(),
            departure_date: None,
            stay_duration_days: 1,
            spend_amount_minor: 0,
            spend_currency: s(),
            spend_currency_exponent: 2,
            rating: 3,
            category: s(),
            note: None,
            created_at: now,
            updated_at: now,
        };
        let habit = || Habit {
            id: 1,
            owner_id: o,
            name: s(),
            description: None,
            frequency: s(),
            category: s(),
            target_days_per_week: 7,
            archived: false,
            created_at: None,
            updated_at: None,
        };
        let bookmark = || Bookmark {
            id: 1,
            owner_id: o,
            url: s(),
            title: s(),
            description: None,
            author: None,
            image_url: None,
            domain: s(),
            reader_content: None,
            reading_time_minutes: 1,
            is_favorite: false,
            is_archived: false,
            tags: s(),
            created_at: s(),
            updated_at: s(),
        };
        vec![
            SystemEvent::TodoCreated(sample_todo(o)),
            SystemEvent::TodoUpdated(sample_todo(o)),
            SystemEvent::TodoDeleted {
                id: 1,
                room_id: s(),
                owner_id: o,
            },
            SystemEvent::ReminderCreated(reminder()),
            SystemEvent::ReminderUpdated(reminder()),
            SystemEvent::ReminderTriggered(reminder()),
            SystemEvent::ReminderCancelled {
                id: 1,
                room_id: s(),
                owner_id: o,
            },
            SystemEvent::ReminderDeleted {
                id: 1,
                room_id: s(),
                owner_id: o,
            },
            SystemEvent::ReminderHistoryCleared { owner_id: o },
            SystemEvent::TransactionCreated(transaction()),
            SystemEvent::TransactionUpdated(transaction()),
            SystemEvent::TransactionDeleted {
                id: 1,
                room_id: s(),
                owner_id: o,
            },
            SystemEvent::CategoryCreated(Category {
                id: 1,
                owner_id: o,
                name: s(),
                created_at: None,
            }),
            SystemEvent::CategoryDeleted { id: 1, owner_id: o },
            SystemEvent::SettingsUpdated {
                owner_id: o,
                settings: UserSettings::default(),
            },
            SystemEvent::PortfolioHoldingCreated(holding()),
            SystemEvent::PortfolioHoldingUpdated(holding()),
            SystemEvent::PortfolioHoldingDeleted { id: 1, owner_id: o },
            SystemEvent::PortfolioCashUpdated {
                owner_id: o,
                currency: s(),
                balance_minor: 1,
            },
            SystemEvent::PortfolioAuditLogged(PortfolioAuditEntry {
                id: 1,
                owner_id: o,
                action: s(),
                symbol: None,
                quantity_minor: None,
                price_minor: None,
                amount_minor: 1,
                currency: s(),
                currency_exponent: 2,
                realized_pl_minor: None,
                note: None,
                prev_hash: s(),
                entry_hash: s(),
                created_at: None,
            }),
            SystemEvent::PortfolioAlertCreated(price_alert()),
            SystemEvent::PortfolioAlertTriggered(price_alert()),
            SystemEvent::PortfolioAlertDeleted { id: 1, owner_id: o },
            SystemEvent::RecurringBillCreated(bill()),
            SystemEvent::RecurringBillUpdated(bill()),
            SystemEvent::RecurringBillDeleted { id: 1, owner_id: o },
            SystemEvent::RecurringBillPaid {
                id: 1,
                owner_id: o,
                next_due_at: s(),
            },
            SystemEvent::WebhookCreated(webhook()),
            SystemEvent::WebhookUpdated(webhook()),
            SystemEvent::WebhookDeleted {
                id: s(),
                owner_id: o,
            },
            SystemEvent::WebhookTriggered(alert("!r:1")),
            SystemEvent::AiConversationUpdated {
                id: s(),
                owner_id: o,
            },
            SystemEvent::AiConversationDeleted {
                id: s(),
                owner_id: o,
            },
            SystemEvent::AiConfigUpdated { owner_id: o },
            SystemEvent::FootprintCreated(footprint()),
            SystemEvent::FootprintUpdated(footprint()),
            SystemEvent::FootprintDeleted { id: 1, owner_id: o },
            SystemEvent::HabitCreated(habit()),
            SystemEvent::HabitUpdated(habit()),
            SystemEvent::HabitDeleted { id: 1, owner_id: o },
            SystemEvent::HabitLogToggled {
                habit_id: 1,
                owner_id: o,
                log_date: s(),
                completed: true,
            },
            SystemEvent::LifeProfileUpdated {
                owner_id: o,
                profile: LifeProfile {
                    owner_id: o,
                    birth_date: s(),
                    expected_lifespan_years: 80,
                    created_at: s(),
                    updated_at: s(),
                },
            },
            SystemEvent::LifeDayLogged(LifeDayLog {
                id: 1,
                owner_id: o,
                log_date: s(),
                mood: s(),
                weather: None,
                activities: None,
                note: None,
                time_allocation: None,
                created_at: s(),
                updated_at: s(),
            }),
            SystemEvent::LifeDayCleared {
                owner_id: o,
                date: s(),
            },
            SystemEvent::BookmarkCreated(bookmark()),
            SystemEvent::BookmarkUpdated(bookmark()),
            SystemEvent::BookmarkDeleted { id: 1, owner_id: o },
            SystemEvent::MorningBriefingSent {
                owner_id: o,
                delivered_at: s(),
                summary: s(),
            },
        ]
    }

    /// Bump when a variant is added; `event_type` and `owner_id` have no
    /// wildcard, so the compiler already forces their arms.
    const VARIANT_COUNT: usize = 48;

    #[test]
    fn event_type_matches_the_serde_tag_for_every_variant() {
        let owner = AccountId::from_seed("acct_alice");
        let samples = one_of_each(owner);
        let mut seen = std::collections::HashSet::new();
        for ev in &samples {
            let json = serde_json::to_value(ev).unwrap();
            assert_eq!(json["type"], ev.event_type());
            assert_eq!(ev.owner_id(), &owner);
            seen.insert(ev.event_type());
        }
        assert_eq!(seen.len(), samples.len(), "one sample per variant");
        assert_eq!(samples.len(), VARIANT_COUNT);
    }

    #[test]
    fn owner_id_reads_the_ownership_key_not_the_display_name() {
        // Both todos share `sender_name`; only `owner_id` decides delivery.
        let a = AccountId::from_seed("@web-evan:api");
        let b = AccountId::from_seed("@evan:mbm.one");
        let mine = SystemEvent::TodoCreated(sample_todo(a));
        let theirs = SystemEvent::TodoUpdated(sample_todo(b));
        assert_eq!(mine.owner_id(), &a);
        assert_eq!(theirs.owner_id(), &b);
        assert_ne!(mine.owner_id(), theirs.owner_id());

        let deleted = SystemEvent::TodoDeleted {
            id: 7,
            room_id: "!room:1".into(),
            owner_id: b,
        };
        assert_eq!(deleted.owner_id(), &b);
        let cancelled = SystemEvent::ReminderCancelled {
            id: 8,
            room_id: "web".into(),
            owner_id: a,
        };
        assert_eq!(cancelled.owner_id(), &a);
    }

    #[test]
    fn only_webhook_alerts_have_durable_destinations() {
        let owner = AccountId::from_seed("acct_alice");
        assert!(SystemEvent::AiConfigUpdated { owner_id: owner }
            .destinations()
            .is_empty());
        assert_eq!(
            SystemEvent::WebhookTriggered(alert("!r:1")).destinations(),
            vec![Destination::MatrixRoom("!r:1".to_string())]
        );
    }

    #[test]
    fn destination_keys_round_trip() {
        let d = Destination::MatrixRoom("!abc:example.org".to_string());
        assert_eq!(d.as_key(), "matrix:!abc:example.org");
        assert_eq!(Destination::parse(&d.as_key()), Some(d));
        assert_eq!(Destination::parse("discord:1"), None);
        assert_eq!(Destination::parse("matrix:"), None);
    }

    #[test]
    fn a_failed_attempt_retries_until_the_last_one() {
        // `attempts` already counts the claim that just failed (C7).
        assert_eq!(
            next_delivery_step(1, 10),
            DeliveryStep::RetryAfterAttempts(0)
        );
        assert_eq!(
            next_delivery_step(9, 10),
            DeliveryStep::RetryAfterAttempts(8)
        );
        assert_eq!(next_delivery_step(10, 10), DeliveryStep::Fail);
        assert_eq!(next_delivery_step(11, 10), DeliveryStep::Fail);
    }
}
