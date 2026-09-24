//! Keeps a bill's lead-time reminder in step with the bill.
//!
//! The reconcile rules live in [`crate::modules::due_reminder`], shared with
//! todo. This module's job is only to say what a bill wants: when it should
//! fire, and what it should say.
//!
//! A bill differs from a task in one way that matters here. Finishing a task
//! ends it, so completing one tears its reminder down for good. Paying a bill
//! only advances `next_due_at` to the next period, so [`sync`] re-arms at the
//! new date instead — which is the whole point of a *recurring* bill. The
//! reminder is torn down only when the bill stops recurring: its final
//! installment is paid (`completed`), it is paused or cancelled, or it is
//! deleted.

use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};

use crate::events::EventBroadcaster;
use crate::modules::due_reminder::{self, ReminderPlan, SyncOutcome};
use crate::modules::recurring_bills::models::RecurringBill;
use crate::modules::reminder::models::ReminderSource;

/// Parses a stored `next_due_at`, which is TEXT rather than a timestamp.
///
/// The column accepts three shapes across the codebase's own writers, so this
/// mirrors `repository::advance_due_date` rather than assuming one. A value
/// that parses as none of them has no due instant, so no reminder is scheduled
/// — better than guessing `now` and firing something the user never asked for.
pub fn parse_due_at(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(&format!("{raw} 00:00:00"), "%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|ndt| Utc.from_utc_datetime(&ndt))
}

/// Whether a bill in this state still has periods left to remind about.
fn still_recurring(status: &str) -> bool {
    status.eq_ignore_ascii_case("active")
}

/// The instant a reminder should fire for `bill`, if it wants one at all.
pub fn wanted_trigger(bill: &RecurringBill) -> Option<DateTime<Utc>> {
    let minutes = bill.remind_before_minutes?;
    if minutes <= 0 || !still_recurring(&bill.status) {
        return None;
    }
    Some(parse_due_at(&bill.next_due_at)? - Duration::minutes(minutes))
}

/// Where the bill's reminder should be delivered.
///
/// A bill created from the web carries the `web` sentinel rather than a room,
/// so the caller must have named one; without either there is nowhere to send
/// it and no reminder is scheduled.
pub fn reminder_room(bill: &RecurringBill) -> Option<&str> {
    match bill.remind_room_id.as_deref() {
        Some(room) if !room.trim().is_empty() => Some(room),
        _ if bill.room_id != "web" => Some(&bill.room_id),
        _ => None,
    }
}

/// Reconciles the linked reminder against the bill's current state.
pub async fn sync(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    bill: &RecurringBill,
) -> Result<SyncOutcome> {
    let Some(room_id) = reminder_room(bill) else {
        return tear_down(pool, broadcaster, bill.id).await;
    };

    let major = bill.amount_minor as f64 / 10f64.powi(bill.currency_exponent as i32);
    let plan = ReminderPlan {
        source: ReminderSource::RecurringBill,
        source_id: bill.id,
        owner_id: &bill.owner_id,
        owner_name: &bill.sender_name,
        room_id,
        message: format!(
            "Bill #{} is due: {} ({} {:.2})",
            bill.id, bill.name, bill.currency, major
        ),
        trigger_at: wanted_trigger(bill),
    };

    due_reminder::sync(pool, broadcaster, &plan).await
}

/// Cancels the bill's linked reminder if one is pending.
pub async fn tear_down(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    bill_id: i64,
) -> Result<SyncOutcome> {
    due_reminder::tear_down(pool, broadcaster, ReminderSource::RecurringBill, bill_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::recurring_bills::models::CreateBillPayload;
    use crate::modules::recurring_bills::repository::RecurringBillRepository;
    use crate::modules::reminder::repository::ReminderRepository;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    fn due_in(hours: i64) -> String {
        (Utc::now() + Duration::hours(hours))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    async fn bill_with(pool: &DbPool, next_due_at: String, lead: Option<i64>) -> RecurringBill {
        RecurringBillRepository::create(
            pool,
            "!room:a",
            "@alice:a",
            "alice",
            CreateBillPayload {
                name: "Spotify".to_string(),
                category: "subscription".to_string(),
                amount_minor: 999,
                currency: "USD".to_string(),
                currency_exponent: Some(2),
                frequency: Some("monthly".to_string()),
                due_day: None,
                next_due_at,
                total_installments: None,
                auto_pay: Some(false),
                notes: None,
                remind_before_minutes: lead,
                remind_room_id: None,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_schedules_at_due_minus_lead() {
        let pool = setup().await;
        let bill = bill_with(&pool, due_in(4), Some(60)).await;

        let outcome = sync(&pool, None, &bill).await.unwrap();

        let expected = parse_due_at(&bill.next_due_at).unwrap() - Duration::minutes(60);
        assert_eq!(outcome, SyncOutcome::Scheduled(expected));

        let rem = ReminderRepository::find_pending_for_source(
            &pool,
            ReminderSource::RecurringBill,
            bill.id,
        )
        .await
        .unwrap()
        .expect("a linked reminder must exist");
        assert_eq!(rem.source_kind.as_deref(), Some("recurring_bill"));
        assert!(rem.message.contains("Spotify"));
    }

    /// A bill with no lead time wants nothing, and asking twice changes nothing.
    #[tokio::test]
    async fn test_no_lead_time_schedules_nothing() {
        let pool = setup().await;
        let bill = bill_with(&pool, due_in(4), None).await;

        assert_eq!(
            sync(&pool, None, &bill).await.unwrap(),
            SyncOutcome::Nothing
        );
        assert_eq!(
            sync(&pool, None, &bill).await.unwrap(),
            SyncOutcome::Nothing
        );
    }

    /// Paying advances the due date, so the reminder must re-arm rather than
    /// disappear — the difference between a bill and a task.
    #[tokio::test]
    async fn test_paying_rearms_at_the_next_period() {
        let pool = setup().await;
        let bill = bill_with(&pool, due_in(4), Some(60)).await;
        sync(&pool, None, &bill).await.unwrap();

        let (paid, _) = RecurringBillRepository::mark_paid(&pool, bill.id, "@alice:a", None)
            .await
            .unwrap();
        let outcome = sync(&pool, None, &paid).await.unwrap();

        let expected = parse_due_at(&paid.next_due_at).unwrap() - Duration::minutes(60);
        assert_eq!(outcome, SyncOutcome::Scheduled(expected));

        let pending: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM reminders WHERE source_kind = 'recurring_bill' \
             AND source_id = ? AND status = 'pending'",
            bill.id
        )
        .unwrap();
        assert_eq!(pending, 1, "re-arming must replace, not stack");
    }

    /// The last installment ends the bill, and with it the reminder.
    #[tokio::test]
    async fn test_completing_the_final_installment_tears_the_reminder_down() {
        let pool = setup().await;
        let mut bill = bill_with(&pool, due_in(4), Some(60)).await;
        bill.total_installments = Some(1);
        bill = RecurringBillRepository::update(
            &pool,
            bill.id,
            "@alice:a",
            crate::modules::recurring_bills::models::UpdateBillPayload {
                total_installments: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        sync(&pool, None, &bill).await.unwrap();

        let (paid, _) = RecurringBillRepository::mark_paid(&pool, bill.id, "@alice:a", None)
            .await
            .unwrap();
        assert_eq!(paid.status, "completed");

        assert_eq!(
            sync(&pool, None, &paid).await.unwrap(),
            SyncOutcome::Cancelled
        );
        assert!(ReminderRepository::find_pending_for_source(
            &pool,
            ReminderSource::RecurringBill,
            bill.id
        )
        .await
        .unwrap()
        .is_none());
    }

    /// A web-created bill has no room of its own, so it needs one named.
    #[tokio::test]
    async fn test_web_bill_without_a_room_schedules_nothing() {
        let pool = setup().await;
        let bill = RecurringBillRepository::create(
            &pool,
            "web",
            "@alice:a",
            "alice",
            CreateBillPayload {
                name: "Netflix".to_string(),
                category: "subscription".to_string(),
                amount_minor: 1599,
                currency: "USD".to_string(),
                currency_exponent: Some(2),
                frequency: Some("monthly".to_string()),
                due_day: None,
                next_due_at: due_in(4),
                total_installments: None,
                auto_pay: Some(false),
                notes: None,
                remind_before_minutes: Some(60),
                remind_room_id: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(reminder_room(&bill), None);
        assert_eq!(
            sync(&pool, None, &bill).await.unwrap(),
            SyncOutcome::Nothing
        );
    }
}
