use chrono::{Duration, Utc};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use std::time::Duration as StdDuration;
use tracing::{error, info, warn};

use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::ModuleContext;
use crate::utils::{html_escape, strip_html};

pub fn start_reminder_scheduler(ctx: ModuleContext) {
    tokio::spawn(async move {
        info!("Starting persistent background reminder worker loop...");
        let mut interval = tokio::time::interval(StdDuration::from_secs(5));

        loop {
            interval.tick().await;

            match ReminderRepository::get_due_reminders(&ctx.db.pool).await {
                Ok(due_list) => {
                    for reminder in due_list {
                        info!(
                            "Triggering due reminder #{} for {} in room {}",
                            reminder.id, reminder.sender_name, reminder.room_id
                        );

                        let mut send_success = false;

                        if let Ok(room_id) = <&RoomId>::try_from(reminder.room_id.as_str()) {
                            if let Some(room) = ctx.client.get_room(room_id) {
                                let alert_html = format!(
                                    "⏰ <b>REMINDER for {}!</b><br/>{}",
                                    html_escape(&reminder.sender_name),
                                    html_escape(&reminder.message)
                                );
                                let plain_text = strip_html(&alert_html);
                                let alert_content = RoomMessageEventContent::text_html(&plain_text, &alert_html);

                                match room.send(alert_content).await {
                                    Ok(_) => {
                                        send_success = true;
                                        info!("Successfully delivered reminder notification #{}", reminder.id);
                                    }
                                    Err(err) => {
                                        error!("Failed to send reminder notification #{}: {:?}", reminder.id, err);
                                    }
                                }
                            } else {
                                warn!("Bot is not joined to room {} for reminder #{}", reminder.room_id, reminder.id);
                            }
                        } else {
                            error!("Invalid room_id {} for reminder #{}", reminder.room_id, reminder.id);
                        }

                        if send_success {
                            if let Some(broadcaster) = &ctx.broadcaster {
                                broadcaster.send(crate::events::SystemEvent::ReminderTriggered(reminder.clone()));
                            }

                            // Process recurrence or mark triggered
                            match reminder.recurrence.as_str() {
                                "daily" => {
                                    let now = Utc::now();
                                    let mut next_trigger = reminder.trigger_at + Duration::days(1);
                                    while next_trigger <= now {
                                        next_trigger = next_trigger + Duration::days(1);
                                    }
                                    let _ = ReminderRepository::reschedule(&ctx.db.pool, reminder.id, next_trigger).await;
                                }
                                "weekly" => {
                                    let now = Utc::now();
                                    let mut next_trigger = reminder.trigger_at + Duration::weeks(1);
                                    while next_trigger <= now {
                                        next_trigger = next_trigger + Duration::weeks(1);
                                    }
                                    let _ = ReminderRepository::reschedule(&ctx.db.pool, reminder.id, next_trigger).await;
                                }
                                _ => {
                                    let _ = ReminderRepository::mark_triggered(&ctx.db.pool, reminder.id).await;
                                }
                            }
                        } else {
                            // Increment retry count on delivery failure
                            if reminder.retry_count + 1 >= 5 {
                                error!("Reminder #{} exceeded max retries (5). Marking failed.", reminder.id);
                                let _ = ReminderRepository::mark_failed(&ctx.db.pool, reminder.id).await;
                            } else {
                                let _ = ReminderRepository::increment_retry(&ctx.db.pool, reminder.id).await;
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("Error querying due reminders: {:?}", err);
                }
            }
        }
    });
}
