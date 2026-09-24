use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::modules::reminder::models::{RecurrencePattern, Reminder, ReminderSource};

/// Delivery attempts allowed before a reminder is marked `failed`.
///
/// Shared with `get_due_reminders` so the query and the scheduler cannot
/// disagree about when to stop retrying.
pub const MAX_DELIVERY_ATTEMPTS: i64 = 5;

/// The `Reminder` column list. Shared by every read so a new column cannot be
/// missed by one query.
const COLUMNS: &str = "id, room_id, owner_id, sender_name, message, trigger_at, \
                       recurrence, status, retry_count, source_kind, source_id, created_at";

pub struct ReminderRepository;

impl ReminderRepository {
    // One parameter per column, which is what an INSERT is. Clippy's threshold is 7
    // and this is a row; collapsing them into a struct would spell the row type a
    // second time, minus the fields the database fills in, and every caller would
    // build that struct inline at the call site instead. No clarity is gained.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        pool: &DbPool,
        room_id: &str,
        owner_id: &str,
        sender_name: &str,
        message: &str,
        trigger_at: DateTime<Utc>,
        recurrence: RecurrencePattern,
        source: Option<(ReminderSource, i64)>,
    ) -> Result<Reminder> {
        let rec_str = recurrence.as_str();

        let res = crate::db_scalar_one!(
            pool,
            i64,
            r#"
            INSERT INTO reminders (room_id, owner_id, sender_name, message, trigger_at, recurrence, status, retry_count, source_kind, source_id)
            VALUES (?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?)
            RETURNING id
            "#,
            room_id,
            owner_id,
            sender_name,
            message,
            trigger_at,
            rec_str,
            source.map(|(kind, _)| kind.as_str()),
            source.map(|(_, id)| id)
        )?;

        let id = res;

        let reminder = crate::db_fetch_one!(
            pool,
            Reminder,
            &format!("SELECT {COLUMNS} FROM reminders WHERE id = ?"),
            id
        )?;

        Ok(reminder)
    }

    /// The pending reminder scheduled from this source row, if there is one.
    ///
    /// At most one exists at a time: `sync` reschedules rather than stacking a
    /// second row.
    pub async fn find_pending_for_source(
        pool: &DbPool,
        source: ReminderSource,
        source_id: i64,
    ) -> Result<Option<Reminder>> {
        let reminder = crate::db_fetch_optional!(
            pool,
            Reminder,
            &format!(
                "SELECT {COLUMNS} FROM reminders WHERE source_kind = ? AND source_id = ? \
             AND status = 'pending' ORDER BY id DESC LIMIT 1"
            ),
            source.as_str(),
            source_id
        )?;

        Ok(reminder)
    }

    /// Cancels the pending reminder linked to this source row, returning it so
    /// the caller can broadcast the cancellation.
    ///
    /// Ownership is not re-checked here: the caller reached the source row
    /// through an owner-scoped lookup, and the link columns are not
    /// user-supplied.
    pub async fn cancel_for_source(
        pool: &DbPool,
        source: ReminderSource,
        source_id: i64,
    ) -> Result<Option<Reminder>> {
        let Some(reminder) = Self::find_pending_for_source(pool, source, source_id).await? else {
            return Ok(None);
        };

        crate::db_execute!(
            pool,
            "UPDATE reminders SET status = 'cancelled' WHERE id = ?",
            reminder.id
        )?;

        Ok(Some(reminder))
    }

    pub async fn count_user_pending(pool: &DbPool, owner_id: &str) -> Result<i64> {
        let count = crate::db_scalar_one!(
            pool,
            i64,
            "SELECT COUNT(*) FROM reminders WHERE owner_id = ? AND status = 'pending'",
            owner_id
        )?;

        Ok(count)
    }

    pub async fn get_due_reminders(pool: &DbPool) -> Result<Vec<Reminder>> {
        let now = Utc::now();
        let reminders = crate::db_fetch_all!(pool, Reminder, &format!(
            "SELECT {COLUMNS} FROM reminders WHERE status = 'pending' AND trigger_at <= ? AND retry_count < ?"
        ), now, MAX_DELIVERY_ATTEMPTS)?;

        Ok(reminders)
    }

    pub async fn mark_triggered(pool: &DbPool, id: i64) -> Result<()> {
        crate::db_execute!(
            pool,
            "UPDATE reminders SET status = 'triggered' WHERE id = ?",
            id
        )?;
        Ok(())
    }

    pub async fn mark_failed(pool: &DbPool, id: i64) -> Result<()> {
        crate::db_execute!(
            pool,
            "UPDATE reminders SET status = 'failed' WHERE id = ?",
            id
        )?;
        Ok(())
    }

    pub async fn increment_retry(
        pool: &DbPool,
        id: i64,
        new_trigger_at: DateTime<Utc>,
    ) -> Result<()> {
        crate::db_execute!(
            pool,
            "UPDATE reminders SET retry_count = retry_count + 1, trigger_at = ? WHERE id = ?",
            new_trigger_at,
            id
        )?;
        Ok(())
    }

    pub async fn reschedule(pool: &DbPool, id: i64, new_trigger_at: DateTime<Utc>) -> Result<()> {
        crate::db_execute!(
            pool,
            "UPDATE reminders SET trigger_at = ?, status = 'pending', retry_count = 0 WHERE id = ?",
            new_trigger_at,
            id
        )?;
        Ok(())
    }

    pub async fn list_user_pending(
        pool: &DbPool,
        owner_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Reminder>> {
        let reminders = crate::db_fetch_all!(
            pool,
            Reminder,
            &format!(
                "SELECT {COLUMNS} FROM reminders WHERE owner_id = ? AND status = 'pending' \
             ORDER BY trigger_at ASC LIMIT ? OFFSET ?"
            ),
            owner_id,
            limit,
            offset
        )?;

        Ok(reminders)
    }

    /// Looks up a reminder that `owner_id` owns.
    pub async fn find_owned(pool: &DbPool, owner_id: &str, id: i64) -> Result<Option<Reminder>> {
        let reminder = crate::db_fetch_optional!(
            pool,
            Reminder,
            &format!("SELECT {COLUMNS} FROM reminders WHERE id = ? AND owner_id = ?"),
            id,
            owner_id
        )?;

        Ok(reminder)
    }

    /// Edits a *pending* reminder's message, fire time, and recurrence.
    ///
    /// Restricted to `status = 'pending'` for the same reason `cancel` is: once
    /// a reminder has fired or failed, its row is a delivery record, and moving
    /// its `trigger_at` would either resurrect it in the scheduler's next sweep
    /// or silently rewrite history. `retry_count` resets because a re-timed
    /// reminder is a fresh delivery, not a continuation of the failed one.
    pub async fn update_pending(
        pool: &DbPool,
        owner_id: &str,
        id: i64,
        message: &str,
        trigger_at: DateTime<Utc>,
        recurrence: RecurrencePattern,
    ) -> Result<Option<Reminder>> {
        let res = crate::db_execute!(
            pool,
            "UPDATE reminders SET message = ?, trigger_at = ?, recurrence = ?, retry_count = 0 \
             WHERE id = ? AND owner_id = ? AND status = 'pending'",
            message,
            trigger_at,
            recurrence.as_str(),
            id,
            owner_id
        )?;

        if res == 0 {
            return Ok(None);
        }

        Self::find_owned(pool, owner_id, id).await
    }

    pub async fn list_user_reminders(
        pool: &DbPool,
        owner_id: &str,
        status: Option<&str>,
        room_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Reminder>> {
        let (status_filter, order_clause) = match status {
            Some("history") => (
                "AND status != 'pending'",
                "ORDER BY trigger_at DESC, id DESC",
            ),
            Some("all") => ("", "ORDER BY trigger_at DESC, id DESC"),
            Some("triggered") => (
                "AND status = 'triggered'",
                "ORDER BY trigger_at DESC, id DESC",
            ),
            Some("cancelled") => (
                "AND status = 'cancelled'",
                "ORDER BY trigger_at DESC, id DESC",
            ),
            Some("failed") => ("AND status = 'failed'", "ORDER BY trigger_at DESC, id DESC"),
            Some("pending") | None => ("AND status = 'pending'", "ORDER BY trigger_at ASC"),
            Some(_) => ("AND status = 'pending'", "ORDER BY trigger_at ASC"),
        };

        let reminders = if let Some(room) = room_id {
            let sql = format!(
                "SELECT {COLUMNS} FROM reminders WHERE owner_id = ? AND room_id = ? {status_filter} {order_clause} LIMIT ? OFFSET ?"
            );
            crate::db_fetch_all!(pool, Reminder, &sql, owner_id, room, limit, offset)?
        } else {
            let sql = format!(
                "SELECT {COLUMNS} FROM reminders WHERE owner_id = ? {status_filter} {order_clause} LIMIT ? OFFSET ?"
            );
            crate::db_fetch_all!(pool, Reminder, &sql, owner_id, limit, offset)?
        };

        Ok(reminders)
    }

    pub async fn count_user_reminders(
        pool: &DbPool,
        owner_id: &str,
        status: Option<&str>,
        room_id: Option<&str>,
    ) -> Result<i64> {
        let status_filter = match status {
            Some("history") => "AND status != 'pending'",
            Some("all") => "",
            Some("triggered") => "AND status = 'triggered'",
            Some("cancelled") => "AND status = 'cancelled'",
            Some("failed") => "AND status = 'failed'",
            Some("pending") | None => "AND status = 'pending'",
            Some(_) => "AND status = 'pending'",
        };

        let total = if let Some(room) = room_id {
            let sql = format!(
                "SELECT COUNT(*) FROM reminders WHERE owner_id = ? AND room_id = ? {status_filter}"
            );
            crate::db_scalar_one!(pool, i64, &sql, owner_id, room)?
        } else {
            let sql = format!("SELECT COUNT(*) FROM reminders WHERE owner_id = ? {status_filter}");
            crate::db_scalar_one!(pool, i64, &sql, owner_id)?
        };

        Ok(total)
    }

    pub async fn delete_history_item(pool: &DbPool, owner_id: &str, id: i64) -> Result<bool> {
        let res = crate::db_execute!(
            pool,
            "DELETE FROM reminders WHERE id = ? AND owner_id = ? AND status != 'pending'",
            id,
            owner_id
        )?;

        Ok(res > 0)
    }

    pub async fn clear_user_history(pool: &DbPool, owner_id: &str) -> Result<u64> {
        let res = crate::db_execute!(
            pool,
            "DELETE FROM reminders WHERE owner_id = ? AND status != 'pending'",
            owner_id
        )?;

        Ok(res)
    }

    pub async fn cancel(pool: &DbPool, owner_id: &str, id: i64) -> Result<bool> {
        let res = crate::db_execute!(pool, "UPDATE reminders SET status = 'cancelled' WHERE id = ? AND owner_id = ? AND status = 'pending'", id, owner_id)?;

        Ok(res > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    async fn setup_test_db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_reminder_crud_flow() {
        let pool = setup_test_db().await;
        let past_time = Utc::now() - Duration::seconds(10);

        let rem = ReminderRepository::create(
            &pool,
            "!room:a",
            "@bob:a",
            "bob",
            "Check oven",
            past_time,
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(rem.message, "Check oven");

        let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, rem.id);

        ReminderRepository::mark_triggered(&pool, rem.id)
            .await
            .unwrap();

        let due_after = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert!(due_after.is_empty());
    }

    /// An edit must be owner-scoped and must not reach an already-fired row.
    #[tokio::test]
    async fn test_update_pending_is_owner_scoped_and_pending_only() {
        let pool = setup_test_db().await;
        let later = Utc::now() + Duration::hours(1);

        let rem = ReminderRepository::create(
            &pool,
            "!room:a",
            "@bob:a",
            "bob",
            "Check oven",
            later,
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        // Someone else's edit finds nothing to update.
        let stranger = ReminderRepository::update_pending(
            &pool,
            "@mallory:a",
            rem.id,
            "Hijacked",
            later,
            RecurrencePattern::None,
        )
        .await
        .unwrap();
        assert!(stranger.is_none(), "a non-owner must not be able to edit");

        // The owner's edit lands, and resets the retry budget.
        let new_time = Utc::now() + Duration::hours(3);
        let updated = ReminderRepository::update_pending(
            &pool,
            "@bob:a",
            rem.id,
            "Check oven again",
            new_time,
            RecurrencePattern::Daily,
        )
        .await
        .unwrap()
        .expect("the owner's edit should apply");

        assert_eq!(updated.message, "Check oven again");
        assert_eq!(updated.recurrence, RecurrencePattern::Daily.as_str());
        assert_eq!(updated.retry_count, 0);

        // Once it has fired, it is a delivery record, not an editable plan.
        ReminderRepository::mark_triggered(&pool, rem.id)
            .await
            .unwrap();
        let after_fire = ReminderRepository::update_pending(
            &pool,
            "@bob:a",
            rem.id,
            "Too late",
            new_time,
            RecurrencePattern::None,
        )
        .await
        .unwrap();
        assert!(
            after_fire.is_none(),
            "a triggered reminder must not be editable"
        );
    }

    /// A failed delivery must neither be dropped nor retried forever.
    #[tokio::test]
    async fn test_retry_budget_is_bounded_and_never_silently_drops() {
        let pool = setup_test_db().await;
        let past = Utc::now() - Duration::seconds(10);

        let rem = ReminderRepository::create(
            &pool,
            "!room:a",
            "@bob:a",
            "bob",
            "Check oven",
            past,
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        // Burn through the budget; each retry keeps it pending and due.
        for attempt in 0..MAX_DELIVERY_ATTEMPTS {
            let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
            assert_eq!(due.len(), 1, "still owed a delivery on attempt {}", attempt);
            assert_eq!(due[0].retry_count, attempt);

            ReminderRepository::increment_retry(&pool, rem.id, Utc::now() - Duration::seconds(1))
                .await
                .unwrap();
        }

        // Budget spent: the query stops offering it even though it is `pending`,
        // so it can never spin forever.
        let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert!(due.is_empty(), "exhausted reminder must stop being retried");

        // And the scheduler records the outcome rather than leaving it dangling.
        ReminderRepository::mark_failed(&pool, rem.id)
            .await
            .unwrap();
        let status: (String,) = crate::db_fetch_one!(
            &pool,
            _,
            "SELECT status FROM reminders WHERE id = ?",
            rem.id
        )
        .unwrap();
        assert_eq!(status.0, "failed");
    }

    /// A successful delivery after retries clears the counter, so a later
    /// occurrence starts from a full budget.
    #[tokio::test]
    async fn test_reschedule_resets_the_retry_counter() {
        let pool = setup_test_db().await;
        let past = Utc::now() - Duration::seconds(10);

        let rem = ReminderRepository::create(
            &pool,
            "!room:a",
            "@bob:a",
            "bob",
            "Standup",
            past,
            RecurrencePattern::Daily,
            None,
        )
        .await
        .unwrap();

        ReminderRepository::increment_retry(&pool, rem.id, Utc::now() - Duration::seconds(1))
            .await
            .unwrap();
        let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert_eq!(due[0].retry_count, 1);

        ReminderRepository::reschedule(&pool, rem.id, Utc::now() + Duration::days(1))
            .await
            .unwrap();

        let row: (i64, String) = crate::db_fetch_one!(
            &pool,
            _,
            "SELECT retry_count, status FROM reminders WHERE id = ?",
            rem.id
        )
        .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, "pending");
    }

    #[tokio::test]
    async fn test_reminder_history_flow() {
        let pool = setup_test_db().await;
        let past = Utc::now() - Duration::hours(2);

        let rem1 = ReminderRepository::create(
            &pool,
            "!room:1",
            "@alice:m",
            "Alice",
            "Reminder 1",
            past,
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        let rem2 = ReminderRepository::create(
            &pool,
            "!room:1",
            "@alice:m",
            "Alice",
            "Reminder 2",
            past + Duration::minutes(10),
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        let rem3 = ReminderRepository::create(
            &pool,
            "!room:1",
            "@alice:m",
            "Alice",
            "Reminder 3",
            past + Duration::hours(1),
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        // Mark rem1 triggered, rem2 cancelled
        ReminderRepository::mark_triggered(&pool, rem1.id)
            .await
            .unwrap();
        ReminderRepository::cancel(&pool, "@alice:m", rem2.id)
            .await
            .unwrap();

        // Pending count should be 1 (rem3)
        let pending = ReminderRepository::list_user_reminders(
            &pool,
            "@alice:m",
            Some("pending"),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, rem3.id);

        // History count should be 2 (rem1, rem2)
        let history = ReminderRepository::list_user_reminders(
            &pool,
            "@alice:m",
            Some("history"),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(history.len(), 2);
        let total_hist =
            ReminderRepository::count_user_reminders(&pool, "@alice:m", Some("history"), None)
                .await
                .unwrap();
        assert_eq!(total_hist, 2);

        // Filter by triggered
        let triggered = ReminderRepository::list_user_reminders(
            &pool,
            "@alice:m",
            Some("triggered"),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0].id, rem1.id);

        // Delete history item
        let deleted = ReminderRepository::delete_history_item(&pool, "@alice:m", rem1.id)
            .await
            .unwrap();
        assert!(deleted);
        let history_after_del = ReminderRepository::list_user_reminders(
            &pool,
            "@alice:m",
            Some("history"),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(history_after_del.len(), 1);

        // Clear user history
        let cleared = ReminderRepository::clear_user_history(&pool, "@alice:m")
            .await
            .unwrap();
        assert_eq!(cleared, 1);
        let history_empty = ReminderRepository::list_user_reminders(
            &pool,
            "@alice:m",
            Some("history"),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(history_empty.len(), 0);

        // rem3 is still pending and untouched
        let pending_after = ReminderRepository::list_user_reminders(
            &pool,
            "@alice:m",
            Some("pending"),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(pending_after.len(), 1);
    }
}
