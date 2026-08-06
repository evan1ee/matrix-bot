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

                        // 1. Deliver to specific Matrix room if valid room_id
                        if let Ok(room_id) = <&RoomId>::try_from(reminder.room_id.as_str()) {
                            let room_opt = ctx.client.get_room(room_id);
                            if let Some(room) = room_opt {
                                match room.send(alert_content.clone()).await {
                                    Ok(_) => {
                                        info!("Successfully delivered reminder #{} to Matrix room {}", reminder.id, reminder.room_id);
                                    }
                                    Err(err) => {
                                        warn!("Failed sending reminder #{} to room {}: {:?}. Retrying join...", reminder.id, reminder.room_id, err);
                                        if let Ok(joined) = ctx.client.join_room_by_id(room_id).await {
                                            let _ = joined.send(alert_content.clone()).await;
                                        }
                                    }
                                }
                            } else {
                                warn!("Bot store missing room handle for {}. Joining room...", reminder.room_id);
                                if let Ok(joined) = ctx.client.join_room_by_id(room_id).await {
                                    let _ = joined.send(alert_content.clone()).await;
                                }
                            }
                        }

                        // 2. Broadcast SystemEvent over SSE for real-time Web UI notification
                        if let Some(ref broadcaster) = ctx.broadcaster {
                            broadcaster.send(crate::events::SystemEvent::ReminderTriggered(reminder.clone()));
                        }

                        // 3. Mark as triggered or reschedule (never leave stuck in 'pending'/Triggering... state)
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
                            "monthly" => {
                                let now = Utc::now();
                                let mut next_trigger = reminder.trigger_at + Duration::days(30);
                                while next_trigger <= now {
                                    next_trigger = next_trigger + Duration::days(30);
                                }
                                let _ = ReminderRepository::reschedule(&ctx.db.pool, reminder.id, next_trigger).await;
                            }
                            "yearly" => {
                                let now = Utc::now();
                                let mut next_trigger = reminder.trigger_at + Duration::days(365);
                                while next_trigger <= now {
                                    next_trigger = next_trigger + Duration::days(365);
                                }
                                let _ = ReminderRepository::reschedule(&ctx.db.pool, reminder.id, next_trigger).await;
                            }
                            _ => {
                                let _ = ReminderRepository::mark_triggered(&ctx.db.pool, reminder.id).await;
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
