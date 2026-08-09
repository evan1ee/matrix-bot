use chrono::{Duration as ChronoDuration, Utc};
use sqlx::SqlitePool;
use tracing::error;

use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::SettingsRepository;
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

    // Read after the command is claimed, so an unrelated message never costs a
    // settings query.
    let tz = SettingsRepository::get_or_default(pool, sender_mxid).await.tz();

    // 1. !remind list [page]
    if clean_lower == "remind list"
        || clean_lower == "!remind list"
        || clean_lower.starts_with("remind list ")
        || clean_lower.starts_with("!remind list ")
    {
        let rest = clean_body.split_whitespace().nth(2).unwrap_or("1");
        let page: i64 = rest.parse().unwrap_or(1).max(1);
        let offset = (page - 1) * 25;

        match ReminderRepository::list_user_pending(pool, sender_mxid, 25, offset).await {
            Ok(reminders) => {
                if reminders.is_empty() {
                    if page > 1 {
                        Some(format!("⏰ <b>Pending Reminders (Page {}):</b><br/>No more reminders on this page.", page))
                    } else {
                        Some("⏰ You have no pending reminders.".to_string())
                    }
                } else {
                    let mut html = format!("⏰ <b>Pending Reminders for {} (Page {}):</b><br/>", html_escape(sender_name), page);
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
                            format_datetime(&r.trigger_at, &tz),
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
    else if clean_lower.starts_with("remind cancel") || clean_lower.starts_with("!remind cancel") {
        let id_str = clean_body
            .trim_start_matches("!remind cancel")
            .trim_start_matches("remind cancel")
            .trim();

        if id_str.is_empty() {
            return Some("⚠️ Usage: <code>!remind cancel &lt;id&gt;</code>".to_string());
        }

        if let Ok(id) = id_str.parse::<i64>() {
            match ReminderRepository::cancel(pool, sender_mxid, id).await {
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

        if raw_args.is_empty() {
            return Some(
                "⚠️ Usage: <code>!remind [daily|weekly] &lt;time&gt; &lt;message&gt;</code><br/>\
                 Examples: <code>!remind 10s check oven</code>, <code>!remind daily 8h standup</code>"
                    .to_string(),
            );
        }

        let (recurrence, args_to_parse) = if raw_args.to_lowercase().starts_with("daily ") {
            (RecurrencePattern::Daily, raw_args[6..].trim())
        } else if raw_args.to_lowercase().starts_with("weekly ") {
            (RecurrencePattern::Weekly, raw_args[7..].trim())
        } else {
            (RecurrencePattern::None, raw_args)
        };

        if let Some((dur, msg)) = parse_reminder_args(args_to_parse) {
            // Security: Message length limit (300 chars)
            if msg.len() > 300 {
                return Some("⚠️ Reminder message too long! Maximum limit is 300 characters.".to_string());
            }

            // Quota check: max 100 pending reminders per user
            match ReminderRepository::count_user_pending(pool, sender_mxid).await {
                Ok(count) if count >= 100 => {
                    return Some("⚠️ Reminder limit reached (100 pending reminders). Cancel existing reminders using <code>!remind cancel &lt;id&gt;</code>.".to_string());
                }
                Err(err) => {
                    error!("Failed checking pending reminder count: {:?}", err);
                }
                _ => {}
            }

            let dur_chrono = match ChronoDuration::from_std(dur) {
                Ok(c) => c,
                Err(_) => {
                    return Some("⚠️ Duration is too large or out of range!".to_string());
                }
            };

            let trigger_at = Utc::now() + dur_chrono;

            match ReminderRepository::create(pool, room_id, sender_mxid, sender_name, &msg, trigger_at, recurrence.clone(), None).await {
                Ok(reminder) => {
                    let rec_note = match recurrence {
                        RecurrencePattern::Daily => " 🔄 (Recurring daily)",
                        RecurrencePattern::Weekly => " 🔄 (Recurring weekly)",
                        RecurrencePattern::Monthly => " 🔄 (Recurring monthly)",
                        RecurrencePattern::Yearly => " 🔄 (Recurring yearly)",
                        RecurrencePattern::None => "",
                    };
                    Some(format!(
                        "⏱️ <b>Reminder #{} set!</b> I will remind you in <b>{}</b> (at {}) for: <i>\"{}\"</i>{}",
                        reminder.id,
                        format_duration(dur),
                        format_datetime(&trigger_at, &tz),
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
                "⚠️ <b>Invalid reminder format or time value.</b><br/>\
                 Usage: <code>!remind [daily|weekly] &lt;time&gt; &lt;message&gt;</code><br/>\
                 Examples: <code>!remind 10s check oven</code>, <code>!remind daily 8h morning standup</code>"
                    .to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_handle_reminder_set_list_cancel() {
        let pool = setup_test_db().await;

        let res_set = handle_reminder_command(&pool, "!room:1", "@bob:1", "bob", "!remind 10s Check oven")
            .await
            .unwrap();
        assert!(res_set.contains("Reminder #1 set!"));

        let res_list = handle_reminder_command(&pool, "!room:1", "@bob:1", "bob", "!remind list")
            .await
            .unwrap();
        assert!(res_list.contains("Pending Reminders for bob"));
        assert!(res_list.contains("Check oven"));

        let res_cancel = handle_reminder_command(&pool, "!room:1", "@bob:1", "bob", "!remind cancel 1")
            .await
            .unwrap();
        assert!(res_cancel.contains("Cancelled reminder"));

        let res_empty_args = handle_reminder_command(&pool, "!room:1", "@bob:1", "bob", "!remind")
            .await
            .unwrap();
        assert!(res_empty_args.contains("Usage:"));
    }
}
