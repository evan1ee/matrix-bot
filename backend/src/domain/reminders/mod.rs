//! Reminder domain entities, value objects and invariants.

use chrono::{DateTime, Duration, Months, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

/// Maximum delivery attempts allowed before a reminder is marked failed.
pub const MAX_DELIVERY_ATTEMPTS: i64 = 5;

/// Maximum allowed length for a reminder message.
pub const MAX_MESSAGE_LEN: usize = 500;

/// Longest lead time accepted for due reminders (30 days).
pub const MAX_LEAD_MINUTES: i64 = 60 * 24 * 30;

/// Pattern for recurring reminders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecurrencePattern {
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl RecurrencePattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecurrencePattern::None => "none",
            RecurrencePattern::Daily => "daily",
            RecurrencePattern::Weekly => "weekly",
            RecurrencePattern::Monthly => "monthly",
            RecurrencePattern::Yearly => "yearly",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "daily" | "day" => RecurrencePattern::Daily,
            "weekly" | "week" => RecurrencePattern::Weekly,
            "monthly" | "month" => RecurrencePattern::Monthly,
            "yearly" | "year" => RecurrencePattern::Yearly,
            _ => RecurrencePattern::None,
        }
    }
}

/// Status of a reminder in the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderStatus {
    Pending,
    Triggered,
    Cancelled,
    Failed,
}

impl ReminderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderStatus::Pending => "pending",
            ReminderStatus::Triggered => "triggered",
            ReminderStatus::Cancelled => "cancelled",
            ReminderStatus::Failed => "failed",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "triggered" => ReminderStatus::Triggered,
            "cancelled" => ReminderStatus::Cancelled,
            "failed" => ReminderStatus::Failed,
            _ => ReminderStatus::Pending,
        }
    }
}

/// Which module's due date scheduled a reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderSource {
    Todo,
    RecurringBill,
}

impl ReminderSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderSource::Todo => "todo",
            ReminderSource::RecurringBill => "recurring_bill",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(ReminderSource::Todo),
            "recurring_bill" => Some(ReminderSource::RecurringBill),
            _ => None,
        }
    }
}

/// A reminder entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: i64,
    pub room_id: String,
    pub owner_id: AccountId,
    pub sender_name: String,
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub recurrence: RecurrencePattern,
    pub status: ReminderStatus,
    pub retry_count: i64,
    pub source_kind: Option<ReminderSource>,
    pub source_id: Option<i64>,
    pub created_at: Option<DateTime<Utc>>,
}

impl Reminder {
    /// True while the reminder is still waiting for exactly this trigger: a
    /// job enqueued for an earlier trigger must not fire a snoozed, advanced
    /// or finished reminder.
    pub fn is_pending_at(&self, trigger_at: DateTime<Utc>) -> bool {
        self.status == ReminderStatus::Pending && self.trigger_at == trigger_at
    }

    /// The entity a reaction on the delivered message acts on: the todo the
    /// reminder came from, else the reminder itself.
    pub fn reaction_target(&self) -> (&'static str, i64) {
        match (self.source_kind, self.source_id) {
            (Some(ReminderSource::Todo), Some(id)) => ("todo", id),
            _ => ("reminder", self.id),
        }
    }
}

/// The next time a recurring reminder should fire, strictly after `now`,
/// evaluated in `tz` so DST transitions keep the same local wall-clock time
/// and multi-step catch-ups anchor to `current`'s day-of-month rather than
/// drifting through shorter intermediate months (e.g. Jan 31 -> Feb 28 -> Mar 31).
pub fn next_occurrence_in_tz(
    current: DateTime<Utc>,
    recurrence: RecurrencePattern,
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
) -> Option<DateTime<Utc>> {
    if recurrence == RecurrencePattern::None {
        return None;
    }
    let local_base = current.with_timezone(&tz).naive_local();
    for step_idx in 1u32..=10_000 {
        let next_naive = match recurrence {
            RecurrencePattern::Daily => {
                local_base.checked_add_signed(Duration::days(i64::from(step_idx)))?
            }
            RecurrencePattern::Weekly => {
                local_base.checked_add_signed(Duration::weeks(i64::from(step_idx)))?
            }
            RecurrencePattern::Monthly => local_base.checked_add_months(Months::new(step_idx))?,
            RecurrencePattern::Yearly => {
                local_base.checked_add_months(Months::new(step_idx.checked_mul(12)?))?
            }
            RecurrencePattern::None => return None,
        };
        let next_utc = resolve_local_to_utc(next_naive, tz)?;
        if next_utc > now {
            return Some(next_utc);
        }
    }
    None
}

fn resolve_local_to_utc(naive: chrono::NaiveDateTime, tz: chrono_tz::Tz) -> Option<DateTime<Utc>> {
    use chrono::TimeZone;
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest.with_timezone(&Utc)),
        chrono::LocalResult::None => {
            // Spring-forward gap: shift forward by 1 hour into valid local time.
            let shifted = naive + Duration::hours(1);
            tz.from_local_datetime(&shifted)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc))
        }
    }
}

/// The next time a recurring reminder should fire, strictly after `now` (in UTC).
pub fn next_occurrence(
    current: DateTime<Utc>,
    recurrence: RecurrencePattern,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    next_occurrence_in_tz(current, recurrence, now, chrono_tz::UTC)
}

/// Input data for creating a new reminder.
#[derive(Debug, Clone)]
pub struct NewReminder {
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub room_id: String,
    pub owner_name: String,
    pub recurrence: RecurrencePattern,
    pub source: Option<(ReminderSource, i64)>,
}

impl NewReminder {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), &'static str> {
        validate_message(&self.message)?;
        validate_trigger_at(self.trigger_at, now)?;
        Ok(())
    }
}

/// What [`ReminderService::sync_due_reminder`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    Nothing,
    Scheduled(DateTime<Utc>),
    Cancelled,
    LeadTimeInPast(DateTime<Utc>),
}

/// What an external feature wants reminding about.
#[derive(Debug, Clone)]
pub struct ReminderPlan {
    pub source: ReminderSource,
    pub source_id: i64,
    pub owner_id: AccountId,
    pub owner_name: String,
    pub room_id: String,
    pub message: String,
    pub trigger_at: Option<DateTime<Utc>>,
}

pub fn validate_message(message: &str) -> Result<(), &'static str> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err("Message required");
    }
    if message.len() > MAX_MESSAGE_LEN {
        return Err("Message exceeds character limit");
    }
    Ok(())
}

pub fn validate_trigger_at(
    trigger_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), &'static str> {
    if trigger_at <= now {
        return Err("Trigger time must be in the future");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recurrence_pattern_conversion() {
        assert_eq!(
            RecurrencePattern::from_str_or_default("daily"),
            RecurrencePattern::Daily
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("day"),
            RecurrencePattern::Daily
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("weekly"),
            RecurrencePattern::Weekly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("week"),
            RecurrencePattern::Weekly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("monthly"),
            RecurrencePattern::Monthly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("month"),
            RecurrencePattern::Monthly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("yearly"),
            RecurrencePattern::Yearly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("year"),
            RecurrencePattern::Yearly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("none"),
            RecurrencePattern::None
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("other"),
            RecurrencePattern::None
        );

        assert_eq!(RecurrencePattern::Daily.as_str(), "daily");
        assert_eq!(RecurrencePattern::Weekly.as_str(), "weekly");
        assert_eq!(RecurrencePattern::Monthly.as_str(), "monthly");
        assert_eq!(RecurrencePattern::Yearly.as_str(), "yearly");
        assert_eq!(RecurrencePattern::None.as_str(), "none");
    }

    #[test]
    fn test_reminder_status_conversion() {
        assert_eq!(
            ReminderStatus::from_str_or_default("pending"),
            ReminderStatus::Pending
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("triggered"),
            ReminderStatus::Triggered
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("cancelled"),
            ReminderStatus::Cancelled
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("failed"),
            ReminderStatus::Failed
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("unknown"),
            ReminderStatus::Pending
        );

        assert_eq!(ReminderStatus::Pending.as_str(), "pending");
        assert_eq!(ReminderStatus::Triggered.as_str(), "triggered");
        assert_eq!(ReminderStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(ReminderStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn test_reminder_source_conversion() {
        assert_eq!(
            ReminderSource::from_str_opt("todo"),
            Some(ReminderSource::Todo)
        );
        assert_eq!(
            ReminderSource::from_str_opt("recurring_bill"),
            Some(ReminderSource::RecurringBill)
        );
        assert_eq!(ReminderSource::from_str_opt("unknown"), None);

        assert_eq!(ReminderSource::Todo.as_str(), "todo");
        assert_eq!(ReminderSource::RecurringBill.as_str(), "recurring_bill");
    }

    #[test]
    fn test_validation() {
        let now = Utc::now();
        assert!(validate_message("").is_err());
        assert!(validate_message("   ").is_err());
        assert!(validate_message(&"a".repeat(501)).is_err());
        assert!(validate_message("Valid reminder message").is_ok());

        assert!(validate_trigger_at(now - chrono::Duration::seconds(1), now).is_err());
        assert!(validate_trigger_at(now + chrono::Duration::seconds(10), now).is_ok());
    }

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn test_non_recurring_has_no_next_occurrence() {
        let t = utc(2026, 1, 10, 9);
        assert_eq!(next_occurrence(t, RecurrencePattern::None, t), None);
    }

    #[test]
    fn test_daily_and_weekly_step() {
        let t = utc(2026, 1, 10, 9);
        assert_eq!(
            next_occurrence(t, RecurrencePattern::Daily, t).unwrap(),
            utc(2026, 1, 11, 9)
        );
        assert_eq!(
            next_occurrence(t, RecurrencePattern::Weekly, t).unwrap(),
            utc(2026, 1, 17, 9)
        );
    }

    #[test]
    fn test_monthly_follows_the_calendar_not_30_days() {
        let t = utc(2026, 1, 1, 9);
        assert_eq!(
            next_occurrence(t, RecurrencePattern::Monthly, t).unwrap(),
            utc(2026, 2, 1, 9)
        );

        let end_of_jan = utc(2026, 1, 31, 9);
        assert_eq!(
            next_occurrence(end_of_jan, RecurrencePattern::Monthly, end_of_jan).unwrap(),
            utc(2026, 2, 28, 9)
        );
    }

    #[test]
    fn test_yearly_follows_the_calendar_not_365_days() {
        let t = utc(2027, 3, 1, 9);
        assert_eq!(
            next_occurrence(t, RecurrencePattern::Yearly, t).unwrap(),
            utc(2028, 3, 1, 9)
        );
    }

    #[test]
    fn test_catches_up_past_missed_occurrences_while_down() {
        let scheduled = utc(2026, 1, 10, 9);
        let now = utc(2026, 1, 13, 10);
        let next = next_occurrence(scheduled, RecurrencePattern::Daily, now).unwrap();
        assert!(next > now);
        assert_eq!(next, utc(2026, 1, 14, 9));

        // M1: Jan 31 monthly reminder catching up in March lands on Mar 31, not Mar 28
        let jan31 = utc(2026, 1, 31, 9);
        let mar1 = utc(2026, 3, 1, 9);
        assert_eq!(
            next_occurrence(jan31, RecurrencePattern::Monthly, mar1).unwrap(),
            utc(2026, 3, 31, 9)
        );
    }

    #[test]
    fn test_next_occurrence_in_tz_preserves_local_hour_across_dst() {
        // 2026-03-07 09:00 EST is 14:00 UTC; after US spring-forward on 2026-03-08,
        // 09:00 EDT is 13:00 UTC.
        let before_dst = utc(2026, 3, 7, 14);
        let next = next_occurrence_in_tz(
            before_dst,
            RecurrencePattern::Daily,
            before_dst,
            chrono_tz::America::New_York,
        )
        .unwrap();
        assert_eq!(next, utc(2026, 3, 8, 13));
    }

    fn fixture(trigger_at: DateTime<Utc>) -> Reminder {
        Reminder {
            id: 7,
            room_id: "!room:test".to_string(),
            owner_id: AccountId::from_seed("acct_alice"),
            sender_name: "alice".to_string(),
            message: "stretch".to_string(),
            trigger_at,
            recurrence: RecurrencePattern::None,
            status: ReminderStatus::Pending,
            retry_count: 0,
            source_kind: Some(ReminderSource::Todo),
            source_id: Some(3),
            created_at: None,
        }
    }

    #[test]
    fn a_reminder_is_pending_only_at_its_own_trigger() {
        let t = utc(2026, 1, 10, 9);
        let r = fixture(t);
        assert!(r.is_pending_at(t));
        assert!(
            !r.is_pending_at(t + Duration::minutes(5)),
            "snoozed/advanced"
        );
        let cancelled = Reminder {
            status: ReminderStatus::Cancelled,
            ..r
        };
        assert!(!cancelled.is_pending_at(t));
    }

    #[test]
    fn a_todo_reminder_reacts_on_its_todo() {
        let r = fixture(utc(2026, 1, 10, 9));
        assert_eq!(r.reaction_target(), ("todo", 3));
        let plain = Reminder {
            source_kind: None,
            source_id: None,
            ..r
        };
        assert_eq!(plain.reaction_target(), ("reminder", 7));
    }
}
