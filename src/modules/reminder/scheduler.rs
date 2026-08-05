use chrono::Duration;
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use std::time::Duration as StdDuration;
use tracing::{error, info};

use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::ModuleContext;
use crate::utils::html_escape;

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

                        if let Ok(room_id) = <&RoomId>::try_from(reminder.room_id.as_str()) {
                            if let Some(room) = ctx.client.get_room(room_id) {
                                let alert_html = format!(
                                    "⏰ <b>REMINDER for {}!</b><br/>{}",
                                    html_escape(&reminder.sender_name),
                                    html_escape(&reminder.message)
                                );
                                let alert_content = RoomMessageEventContent::text_html(&alert_html, &alert_html);

                                if let Err(err) = room.send(alert_content).await {
                                    error!("Failed to send reminder notification: {:?}", err);
                                }
                            }
                        }

                        // Process recurrence or mark triggered
                        match reminder.recurrence.as_str() {
                            "daily" => {
                                let next_trigger = reminder.trigger_at + Duration::days(1);
                                let _ = ReminderRepository::reschedule(&ctx.db.pool, reminder.id, next_trigger).await;
                            }
                            "weekly" => {
                                let next_trigger = reminder.trigger_at + Duration::weeks(1);
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
