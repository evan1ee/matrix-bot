use chrono::{Duration as ChronoDuration, Utc};
use sqlx::SqlitePool;
use tracing::error;

use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::utils::{format_datetime, format_duration, html_escape, parse_reminder_args};

pub async fn handle_reminder_command(
    pool: &SqlitePool,
    room_id: &str,
    sender_mxid: &str,
    sender_name: &str,
    clean_body: &str,
) -> Option<String> {
    let clean_lower = clean_body.to_lowercase();

    if !(clean_lower.starts_with("remind") || clean_lower.starts_with("!remind")) {
        return None;
    }

    // 1. !remind list
    if clean_lower == "remind list" || clean_lower == "!remind list" {
        match ReminderRepository::list_user_pending(pool, room_id, sender_mxid).await {
            Ok(reminders) => {
                if reminders.is_empty() {
                    Some("⏰ You have no pending reminders in this room.".to_string())
                } else {
                    let mut html = format!("⏰ <b>Pending Reminders for {}:</b><br/>", html_escape(sender_name));
                    for r in reminders {
                        let rec_str = if r.recurrence != "none" {
                            format!(" 🔄 <i>({})</i>", r.recurrence)
                        } else {
                            String::new()
                        };
                        html.push_str(&format!(
                            "• [<b>#{}</b>] <i>\"{}\"</i> at <b>{}</b>{}<br/>",
                            r.id,
                            html_escape(&r.message),
                            format_datetime(&r.trigger_at),
                            rec_str
                        ));
                    }
                    html.push_str("<br/><i>(Use <code>!remind cancel &lt;id&gt;</code> to cancel a reminder)</i>");
                    Some(html)
                }
            }
            Err(err) => {
                error!("Failed to fetch pending reminders: {:?}", err);
                Some("❌ Database error loading reminders.".to_string())
            }
        }
    }
    // 2. !remind cancel <id>
    else if clean_lower.starts_with("remind cancel ") || clean_lower.starts_with("!remind cancel ") {
        let id_str = clean_body
            .trim_start_matches("!remind cancel ")
            .trim_start_matches("remind cancel ")
            .trim();

        if let Ok(id) = id_str.parse::<i64>() {
            match ReminderRepository::cancel(pool, room_id, sender_mxid, id).await {
                Ok(true) => Some(format!("🗑️ Cancelled reminder <b>#{}</b>.", id)),
                Ok(false) => Some(format!("⚠️ Pending reminder <b>#{}</b> not found.", id)),
                Err(err) => {
                    error!("Failed to cancel reminder: {:?}", err);
                    Some("❌ Database error cancelling reminder.".to_string())
                }
            }
        } else {
            Some("⚠️ Usage: <code>!remind cancel &lt;id&gt;</code>".to_string())
        }
    }
    // 3. !remind [daily|weekly] <time> <message>
    else {
        let raw_args = clean_body
            .trim_start_matches("!remindme ")
            .trim_start_matches("remindme ")
            .trim_start_matches("!remind ")
            .trim_start_matches("remind ")
            .trim();

        let (recurrence, args_to_parse) = if raw_args.to_lowercase().starts_with("daily ") {
            (RecurrencePattern::Daily, raw_args[6..].trim())
        } else if raw_args.to_lowercase().starts_with("weekly ") {
            (RecurrencePattern::Weekly, raw_args[7..].trim())
        } else {
            (RecurrencePattern::None, raw_args)
        };

        if let Some((dur, msg)) = parse_reminder_args(args_to_parse) {
            let trigger_at = Utc::now() + ChronoDuration::from_std(dur).unwrap_or(ChronoDuration::seconds(10));

            match ReminderRepository::create(pool, room_id, sender_mxid, sender_name, &msg, trigger_at, recurrence.clone()).await {
                Ok(reminder) => {
                    let rec_note = match recurrence {
                        RecurrencePattern::Daily => " 🔄 (Recurring daily)",
                        RecurrencePattern::Weekly => " 🔄 (Recurring weekly)",
                        RecurrencePattern::None => "",
                    };
                    Some(format!(
                        "⏱️ <b>Reminder #{} set!</b> I will remind you in <b>{}</b> (at {}) for: <i>\"{}\"</i>{}",
                        reminder.id,
                        format_duration(dur),
                        format_datetime(&trigger_at),
                        html_escape(&msg),
                        rec_note
                    ))
                }
                Err(err) => {
                    error!("Failed to create reminder: {:?}", err);
                    Some("❌ Database error creating reminder.".to_string())
                }
            }
        } else {
            Some(
                "⚠️ <b>Invalid reminder format.</b><br/>\
                 Usage: <code>!remind [daily|weekly] &lt;time&gt; &lt;message&gt;</code><br/>\
                 Examples: <code>!remind 10s check oven</code>, <code>!remind daily 8h morning standup</code>"
                    .to_string(),
            )
        }
    }
}
