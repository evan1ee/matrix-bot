//! Background delivery scheduler for reminders.

use chrono::{DateTime, Duration, Months, Utc};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use std::time::Duration as StdDuration;
use tracing::{error, info, warn};

use crate::domain::reminders::Reminder;
use crate::infrastructure::postgres::repositories::reminders::MAX_DELIVERY_ATTEMPTS;
use crate::interface::chat::ModuleContext;
use crate::utils::{html_escape, strip_html};

/// How far out a failed delivery is pushed before the next attempt.
fn retry_delay(attempts_so_far: i64) -> Duration {
    Duration::seconds(30 * (attempts_so_far + 1))
}

/// The next time a recurring reminder should fire, strictly after `now`.
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
    while next <= now {
        next = step(next)?;
    }
    Some(next)
}

/// Attempts to deliver the reminder to Matrix.
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
            Ok(resp) => {
                info!(
                    "Delivered reminder #{} to Matrix room {}",
                    reminder.id, reminder.room_id
                );
                let (entity_type, entity_id) = match (reminder.source_kind, reminder.source_id) {
                    (Some(k), Some(id)) if k.as_str() == "todo" => ("todo", id),
                    _ => ("reminder", reminder.id),
                };
                let _ = crate::interface::channels::matrix::reaction_targets::record_target(
                    &ctx.db.pool,
                    resp.response.event_id.as_str(),
                    reminder.room_id.as_str(),
                    entity_type,
                    entity_id,
                    reminder.owner_id.as_str(),
                    &reminder.message,
                )
                .await;
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
            Ok(resp) => {
                info!("Delivered reminder #{} after joining room", reminder.id);
                let (entity_type, entity_id) = match (reminder.source_kind, reminder.source_id) {
                    (Some(k), Some(id)) if k.as_str() == "todo" => ("todo", id),
                    _ => ("reminder", reminder.id),
                };
                let _ = crate::interface::channels::matrix::reaction_targets::record_target(
                    &ctx.db.pool,
                    resp.response.event_id.as_str(),
                    reminder.room_id.as_str(),
                    entity_type,
                    entity_id,
                    reminder.owner_id.as_str(),
                    &reminder.message,
                )
                .await;
                true
            }
            Err(err) => {
                warn!(
                    "Reminder #{} send failed after join: {:?}",
                    reminder.id, err
                );
                false
            }
        },
        Err(err) => {
            warn!("Reminder #{} could not join room: {:?}", reminder.id, err);
            false
        }
    }
}

async fn advance_after_delivery(ctx: &ModuleContext, reminder: &Reminder) {
    let now = Utc::now();
    match next_occurrence(reminder.trigger_at, reminder.recurrence.as_str(), now) {
        Some(next_trigger) => {
            if let Err(err) = ctx
                .services
                .reminders
                .reschedule_delivery(reminder.id, next_trigger)
                .await
            {
                error!("Failed rescheduling reminder #{}: {:?}", reminder.id, err);
            }
        }
        None => {
            if let Err(err) = ctx.services.reminders.mark_delivered(reminder.id).await {
                error!(
                    "Failed marking reminder #{} delivered: {:?}",
                    reminder.id, err
                );
            }
        }
    }
}

async fn handle_failed_delivery(ctx: &ModuleContext, reminder: &Reminder) {
    let attempts = reminder.retry_count + 1;

    if attempts >= MAX_DELIVERY_ATTEMPTS {
        error!(
            "Reminder #{} exhausted {} delivery attempts; marking failed",
            reminder.id, MAX_DELIVERY_ATTEMPTS
        );
        if let Err(err) = ctx.services.reminders.mark_failed(reminder.id).await {
            error!("Failed marking reminder #{} failed: {:?}", reminder.id, err);
        }
        return;
    }

    let next_attempt = Utc::now() + retry_delay(reminder.retry_count);
    warn!(
        "Reminder #{} delivery failed (attempt {}/{}); retrying at {}",
        reminder.id, attempts, MAX_DELIVERY_ATTEMPTS, next_attempt
    );
    if let Err(err) = ctx
        .services
        .reminders
        .increment_retry(reminder.id, next_attempt)
        .await
    {
        error!(
            "Failed recording retry for reminder #{}: {:?}",
            reminder.id, err
        );
    }
}

pub fn format_reminder_alert(
    message: &str,
    trigger_at: &DateTime<Utc>,
    tz: &chrono_tz::Tz,
    recurrence: &str,
) -> String {
    let local_time = trigger_at
        .with_timezone(tz)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let mut html = format!(
        "⏰ <b>提醒时间到了</b><br/>• <b>事项:</b> {}<br/>• <b>设定时间:</b> {}",
        html_escape(message),
        local_time
    );
    let rec_label = match recurrence {
        "daily" => Some("每天重复"),
        "weekly" => Some("每周重复"),
        "monthly" => Some("每月重复"),
        "yearly" => Some("每年重复"),
        _ => None,
    };
    if let Some(rec) = rec_label {
        html.push_str(&format!("<br/>• <b>周期:</b> {}", rec));
    }
    html.push_str("<br/><font color=\"#888888\" size=\"-1\">React: 👍️ Done · 👎️ Delete · 😝 +1 Day · 🎉 Celebrate</font>");
    html
}

pub fn start_reminder_scheduler(ctx: ModuleContext) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting persistent background reminder worker loop...");
        let mut interval = tokio::time::interval(StdDuration::from_secs(5));

        loop {
            interval.tick().await;

            let due_list = match ctx.services.reminders.get_due_reminders(Utc::now()).await {
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

                let tz = ctx
                    .services
                    .settings
                    .get_or_default(&reminder.owner_id)
                    .await
                    .tz();

                let alert_html = format_reminder_alert(
                    &reminder.message,
                    &reminder.trigger_at,
                    &tz,
                    reminder.recurrence.as_str(),
                );
                let plain_text = strip_html(&alert_html);
                let alert_content = RoomMessageEventContent::text_html(&plain_text, &alert_html);

                let delivered = deliver_to_matrix(&ctx, &reminder, alert_content).await;

                if delivered {
                    if let Some(ref broadcaster) = ctx.broadcaster {
                        broadcaster.send(crate::events::SystemEvent::ReminderTriggered(
                            reminder.clone(),
                        ));
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
        assert_eq!(
            next_occurrence(t, "weekly", t).unwrap(),
            utc(2026, 1, 17, 9)
        );
    }

    #[test]
    fn test_monthly_follows_the_calendar_not_30_days() {
        let t = utc(2026, 1, 1, 9);
        assert_eq!(
            next_occurrence(t, "monthly", t).unwrap(),
            utc(2026, 2, 1, 9)
        );

        let end_of_jan = utc(2026, 1, 31, 9);
        assert_eq!(
            next_occurrence(end_of_jan, "monthly", end_of_jan).unwrap(),
            utc(2026, 2, 28, 9)
        );
    }

    #[test]
    fn test_yearly_follows_the_calendar_not_365_days() {
        let t = utc(2027, 3, 1, 9);
        assert_eq!(next_occurrence(t, "yearly", t).unwrap(), utc(2028, 3, 1, 9));
    }

    #[test]
    fn test_catches_up_past_missed_occurrences_while_down() {
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

    #[test]
    fn test_format_reminder_alert_non_recurring() {
        let t = utc(2026, 9, 17, 12);
        let tz = chrono_tz::Asia::Shanghai;
        let html = format_reminder_alert("去跑步", &t, &tz, "none");
        assert!(html.contains("⏰ <b>提醒时间到了</b>"));
        assert!(html.contains("• <b>事项:</b> 去跑步"));
        assert!(html.contains("• <b>设定时间:</b> 2026-09-17 20:00"));
        assert!(!html.contains("周期"));
    }

    #[test]
    fn test_format_reminder_alert_recurring() {
        let t = utc(2026, 9, 17, 12);
        let tz = chrono_tz::Asia::Shanghai;
        let html = format_reminder_alert("去跑步", &t, &tz, "daily");
        assert!(html.contains("• <b>周期:</b> 每天重复"));
    }
}
