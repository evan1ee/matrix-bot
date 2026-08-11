use chrono::{DateTime, Duration, Months, Utc};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use std::time::Duration as StdDuration;
use tracing::{error, info, warn};

use crate::modules::reminder::models::Reminder;
use crate::modules::reminder::repository::{ReminderRepository, MAX_DELIVERY_ATTEMPTS};
use crate::modules::ModuleContext;
use crate::utils::{html_escape, strip_html};

/// How far out a failed delivery is pushed before the next attempt.
///
/// Linear rather than exponential: transient homeserver trouble usually clears
/// in seconds, and `MAX_DELIVERY_ATTEMPTS` bounds the total wait anyway.
fn retry_delay(attempts_so_far: i64) -> Duration {
    Duration::seconds(30 * (attempts_so_far + 1))
}

/// The next time a recurring reminder should fire, strictly after `now`.
///
/// Monthly and yearly step by calendar months rather than fixed 30/365-day
/// offsets, so a reminder set for the 15th stays on the 15th instead of sliding
/// backwards through the year. Returns `None` for non-recurring reminders.
pub fn next_occurrence(
    current: DateTime<Utc>,
    recurrence: &str,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let step = |dt: DateTime<Utc>| -> Option<DateTime<Utc>> {
        match recurrence {
            "daily" => Some(dt + Duration::days(1)),
            "weekly" => Some(dt + Duration::weeks(1)),
            "monthly" => dt.checked_add_months(Months::new(1)),
            "yearly" => dt.checked_add_months(Months::new(12)),
            _ => None,
        }
    };

    let mut next = step(current)?;
    // Catch up past any occurrences missed while the bot was down.
    while next <= now {
        next = step(next)?;
    }
    Some(next)
}

/// Attempts to put the reminder in front of its Matrix room.
///
/// Returns `false` only when the room was addressable but the send failed —
/// that is what earns a retry. Reminders created through the web API carry the
/// `web` sentinel instead of a room ID; for those the SSE broadcast is the
/// delivery, so there is nothing to fail here.
async fn deliver_to_matrix(
    ctx: &ModuleContext,
    reminder: &Reminder,
    content: RoomMessageEventContent,
) -> bool {
    let Ok(room_id) = <&RoomId>::try_from(reminder.room_id.as_str()) else {
        return true;
    };

    if let Some(room) = ctx.client.get_room(room_id) {
        match room.send(content.clone()).await {
            Ok(_) => {
                info!(
                    "Delivered reminder #{} to Matrix room {}",
                    reminder.id, reminder.room_id
                );
                return true;
            }
            Err(err) => {
                warn!(
                    "Failed sending reminder #{} to room {}: {:?}. Retrying via join...",
                    reminder.id, reminder.room_id, err
                );
            }
        }
    } else {
        warn!(
            "Bot store missing room handle for {}. Joining room...",
            reminder.room_id
        );
    }

    match ctx.client.join_room_by_id(room_id).await {
        Ok(joined) => match joined.send(content).await {
            Ok(_) => {
                info!("Delivered reminder #{} after joining room", reminder.id);
                true
            }
            Err(err) => {
                warn!("Reminder #{} send failed after join: {:?}", reminder.id, err);
                false
            }
        },
        Err(err) => {
            warn!("Reminder #{} could not join room: {:?}", reminder.id, err);
            false
        }
    }
}

/// Moves a reminder out of `pending` once it has been delivered.
async fn advance_after_delivery(ctx: &ModuleContext, reminder: &Reminder) {
    let now = Utc::now();
    match next_occurrence(reminder.trigger_at, &reminder.recurrence, now) {
        Some(next_trigger) => {
            if let Err(err) =
                ReminderRepository::reschedule(&ctx.db.pool, reminder.id, next_trigger).await
            {
                error!("Failed rescheduling reminder #{}: {:?}", reminder.id, err);
            }
        }
        None => {
            if let Err(err) = ReminderRepository::mark_triggered(&ctx.db.pool, reminder.id).await {
                error!("Failed marking reminder #{} triggered: {:?}", reminder.id, err);
            }
        }
    }
}

/// Schedules another attempt, or gives up once the budget is spent.
///
/// Either way the reminder leaves `pending`-and-due, so a failing delivery can
/// never be retried forever or silently dropped.
async fn handle_failed_delivery(ctx: &ModuleContext, reminder: &Reminder) {
    let attempts = reminder.retry_count + 1;

    if attempts >= MAX_DELIVERY_ATTEMPTS {
        error!(
            "Reminder #{} exhausted {} delivery attempts; marking failed",
            reminder.id, MAX_DELIVERY_ATTEMPTS
        );
        if let Err(err) = ReminderRepository::mark_failed(&ctx.db.pool, reminder.id).await {
            error!("Failed marking reminder #{} failed: {:?}", reminder.id, err);
        }
        return;
    }

    let next_attempt = Utc::now() + retry_delay(reminder.retry_count);
    warn!(
        "Reminder #{} delivery failed (attempt {}/{}); retrying at {}",
        reminder.id, attempts, MAX_DELIVERY_ATTEMPTS, next_attempt
    );
    if let Err(err) =
        ReminderRepository::increment_retry(&ctx.db.pool, reminder.id, next_attempt).await
    {
        error!("Failed recording retry for reminder #{}: {:?}", reminder.id, err);
    }
}

/// Starts the 5s tick loop, returning its handle so the caller controls its
/// lifetime — see [`crate::modules::BackgroundWorkers`].
pub fn start_reminder_scheduler(ctx: ModuleContext) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting persistent background reminder worker loop...");
        let mut interval = tokio::time::interval(StdDuration::from_secs(5));

        loop {
            interval.tick().await;

            let due_list = match ReminderRepository::get_due_reminders(&ctx.db.pool).await {
                Ok(list) => list,
                Err(err) => {
                    error!("Error querying due reminders: {:?}", err);
                    continue;
                }
            };

            for reminder in due_list {
                info!(
                    "Triggering due reminder #{} for {} (target room: {})",
                    reminder.id, reminder.sender_name, reminder.room_id
                );

                let alert_html = format!(
                    "⏰ <b>REMINDER for {}!</b><br/>{}",
                    html_escape(&reminder.sender_name),
                    html_escape(&reminder.message)
                );
                let plain_text = strip_html(&alert_html);
                let alert_content = RoomMessageEventContent::text_html(&plain_text, &alert_html);

                let delivered = deliver_to_matrix(&ctx, &reminder, alert_content).await;

                if delivered {
                    if let Some(ref broadcaster) = ctx.broadcaster {
                        broadcaster
                            .send(crate::events::SystemEvent::ReminderTriggered(reminder.clone()));
                    }
                    advance_after_delivery(&ctx, &reminder).await;
                } else {
                    handle_failed_delivery(&ctx, &reminder).await;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn test_non_recurring_has_no_next_occurrence() {
        let t = utc(2026, 1, 10, 9);
        assert_eq!(next_occurrence(t, "none", t), None);
        assert_eq!(next_occurrence(t, "", t), None);
    }

    #[test]
    fn test_daily_and_weekly_step() {
        let t = utc(2026, 1, 10, 9);
        assert_eq!(next_occurrence(t, "daily", t).unwrap(), utc(2026, 1, 11, 9));
        assert_eq!(next_occurrence(t, "weekly", t).unwrap(), utc(2026, 1, 17, 9));
    }

    #[test]
    fn test_monthly_follows_the_calendar_not_30_days() {
        // A 30-day step would land on 2026-01-31, walking the reminder backwards
        // through the month; calendar months keep it on the 1st.
        let t = utc(2026, 1, 1, 9);
        assert_eq!(next_occurrence(t, "monthly", t).unwrap(), utc(2026, 2, 1, 9));

        // February clamps a 31st to the last valid day rather than overflowing.
        let end_of_jan = utc(2026, 1, 31, 9);
        assert_eq!(
            next_occurrence(end_of_jan, "monthly", end_of_jan).unwrap(),
            utc(2026, 2, 28, 9)
        );
    }

    #[test]
    fn test_yearly_follows_the_calendar_not_365_days() {
        // 2028 is a leap year, so a 365-day step would drift to Feb 29.
        let t = utc(2027, 3, 1, 9);
        assert_eq!(next_occurrence(t, "yearly", t).unwrap(), utc(2028, 3, 1, 9));
    }

    #[test]
    fn test_catches_up_past_missed_occurrences_while_down() {
        // Bot offline for three days: the next fire is in the future, not a
        // backlog of three separate sends.
        let scheduled = utc(2026, 1, 10, 9);
        let now = utc(2026, 1, 13, 10);
        let next = next_occurrence(scheduled, "daily", now).unwrap();
        assert!(next > now);
        assert_eq!(next, utc(2026, 1, 14, 9));
    }

    #[test]
    fn test_retry_delay_grows_with_attempts() {
        assert_eq!(retry_delay(0), Duration::seconds(30));
        assert_eq!(retry_delay(1), Duration::seconds(60));
        assert_eq!(retry_delay(4), Duration::seconds(150));
    }
}
