use crate::db::DbPool;
use chrono::{Duration as ChronoDuration, Utc};
use tracing::error;

use crate::application::settings::SettingsService;
use crate::events::EventBroadcaster;
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::reminder::service::{self, NewReminder, ReminderError};
use crate::modules::CommandContext;
use crate::utils::{format_datetime, format_duration, html_escape, parse_reminder_args};

// `MAX_MESSAGE_LEN` lives in `service` now — this module owns the reply
// wording, not the limit.

pub async fn handle_reminder_command(
    pool: &DbPool,
    settings: &SettingsService,
    broadcaster: Option<&EventBroadcaster>,
    ctx: &CommandContext,
) -> Option<String> {
    let room_id = ctx.room_id.as_str();
    let owner_id = ctx.owner_id.as_str();
    let sender_name = ctx.sender_name.as_str();
    let clean_body = ctx.full_body.as_str();
    let quoted = ctx.quoted.as_ref();

    let clean_lower = clean_body.to_lowercase();

    if !(clean_lower == "remind"
        || clean_lower == "!remind"
        || clean_lower == "reminders"
        || clean_lower == "!reminders"
        || clean_lower.starts_with("remind ")
        || clean_lower.starts_with("!remind ")
        || clean_lower.starts_with("reminders ")
        || clean_lower.starts_with("!reminders "))
    {
        return None;
    }

    // Read after the command is claimed, so an unrelated message never costs a
    // settings query.
    let tz = settings.get_or_default(owner_id).await.tz();

    let is_history = clean_lower == "remind history"
        || clean_lower == "!remind history"
        || clean_lower == "reminders history"
        || clean_lower == "!reminders history"
        || clean_lower.starts_with("remind history ")
        || clean_lower.starts_with("!remind history ")
        || clean_lower.starts_with("reminders history ")
        || clean_lower.starts_with("!reminders history ");

    let is_list = !is_history
        && (clean_lower == "remind"
            || clean_lower == "!remind"
            || clean_lower == "reminders"
            || clean_lower == "!reminders"
            || clean_lower == "remind list"
            || clean_lower == "!remind list"
            || clean_lower == "reminders list"
            || clean_lower == "!reminders list"
            || clean_lower.starts_with("remind list ")
            || clean_lower.starts_with("!remind list ")
            || clean_lower.starts_with("reminders list ")
            || clean_lower.starts_with("!reminders list "));

    // 1. !remind history [page]
    if is_history {
        let rest = clean_body.split_whitespace().last().unwrap_or("1");
        let page: i64 = rest.parse().unwrap_or(1).max(1);
        let offset = (page - 1) * 25;

        match ReminderRepository::list_user_reminders(
            pool,
            owner_id,
            Some("history"),
            None,
            25,
            offset,
        )
        .await
        {
            Ok(reminders) => {
                if reminders.is_empty() {
                    if page > 1 {
                        Some(format!("📜 <b>Reminder History (Page {}):</b><br/>No more history on this page.", page))
                    } else {
                        Some(format!("📜 <b>Reminder History for {}:</b><br/>You have no reminder history yet.", html_escape(sender_name)))
                    }
                } else {
                    let mut lines = vec![format!(
                        "📜 <b>Reminder History for {} (Page {}):</b>",
                        html_escape(sender_name),
                        page
                    )];
                    for r in &reminders {
                        let formatted_time = format_datetime(&r.trigger_at, &tz);
                        let status_label = match r.status.as_str() {
                            "triggered" => " <font color=\"#10b981\">[Triggered ✅]</font>",
                            "cancelled" => " <font color=\"#6b7280\">[Cancelled 🗑️]</font>",
                            "failed" => " <font color=\"#ef4444\">[Failed ❌]</font>",
                            _ => "",
                        };
                        lines.push(format!(
                            "• [<b>#{}</b>] <code>{}</code> — {}{}",
                            r.id,
                            formatted_time,
                            html_escape(&r.message),
                            status_label
                        ));
                    }
                    Some(lines.join("<br/>"))
                }
            }
            Err(err) => {
                error!("Failed listing reminder history: {:?}", err);
                Some("❌ Database error retrieving reminder history.".to_string())
            }
        }
    }
    // 2. !remind list [page] / bare !remind / !reminders
    else if is_list {
        let rest = clean_body.split_whitespace().last().unwrap_or("1");
        let page: i64 = rest.parse().unwrap_or(1).max(1);
        let offset = (page - 1) * 25;

        match ReminderRepository::list_user_pending(pool, owner_id, 25, offset).await {
            Ok(reminders) => {
                if reminders.is_empty() {
                    if page > 1 {
                        Some(format!("⏰ <b>Pending Reminders (Page {}):</b><br/>No more reminders on this page.", page))
                    } else {
                        Some(format!("⏰ <b>Pending Reminders for {}:</b><br/>You have no pending reminders. Use <code>!remind &lt;time&gt; &lt;msg&gt;</code> to set one.", html_escape(sender_name)))
                    }
                } else {
                    let mut lines = vec![format!(
                        "⏰ <b>Pending Reminders for {} (Page {}):</b>",
                        html_escape(sender_name),
                        page
                    )];
                    for r in &reminders {
                        let formatted_time = format_datetime(&r.trigger_at, &tz);
                        let recurrence_label = match r.recurrence.as_str() {
                            "daily" => " 🔄 <i>(daily)</i>",
                            "weekly" => " 🔄 <i>(weekly)</i>",
                            _ => "",
                        };
                        lines.push(format!(
                            "• [<b>#{}</b>] <code>{}</code> — {}{}",
                            r.id,
                            formatted_time,
                            html_escape(&r.message),
                            recurrence_label
                        ));
                    }
                    Some(lines.join("<br/>"))
                }
            }
            Err(err) => {
                error!("Failed listing reminders: {:?}", err);
                Some("❌ Database error retrieving reminders.".to_string())
            }
        }
    }
    // 2. !remind cancel <id> / !remind rm <id>
    else if clean_lower.starts_with("remind cancel ")
        || clean_lower.starts_with("!remind cancel ")
        || clean_lower.starts_with("remind rm ")
        || clean_lower.starts_with("!remind rm ")
    {
        let id_str = clean_body
            .trim_start_matches("!remind cancel ")
            .trim_start_matches("remind cancel ")
            .trim_start_matches("!remind rm ")
            .trim_start_matches("remind rm ")
            .trim();

        if id_str.is_empty() {
            return Some("⚠️ Usage: <code>!remind cancel &lt;id&gt;</code> (or <code>!remind rm &lt;id&gt;</code>)".to_string());
        }

        if let Ok(id) = id_str.parse::<i64>() {
            match ReminderRepository::cancel(pool, owner_id, id).await {
                Ok(true) => Some(format!("🗑️ Cancelled reminder <b>#{}</b>.", id)),
                Ok(false) => Some(format!("⚠️ Pending reminder <b>#{}</b> not found.", id)),
                Err(err) => {
                    error!("Failed to cancel reminder: {:?}", err);
                    Some("❌ Database error cancelling reminder.".to_string())
                }
            }
        } else {
            Some("⚠️ Usage: <code>!remind cancel &lt;id&gt;</code> (or <code>!remind rm &lt;id&gt;</code>)".to_string())
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

        if let Some((dur, mut msg)) = parse_reminder_args(args_to_parse) {
            if (msg == "Reminder!" || msg.is_empty()) && quoted.is_some() {
                msg = quoted.unwrap().body.clone();
            }

            // Quota check: max 100 pending reminders per user
            match ReminderRepository::count_user_pending(pool, owner_id).await {
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

            match service::create(
                pool,
                broadcaster,
                owner_id,
                NewReminder {
                    message: &msg,
                    trigger_at,
                    room_id,
                    owner_name: sender_name,
                    recurrence: recurrence.clone(),
                },
            )
            .await
            {
                Ok(reminder) => {
                    let rec_note = match recurrence {
                        RecurrencePattern::Daily => " 🔄 (Recurring daily)",
                        RecurrencePattern::Weekly => " 🔄 (Recurring weekly)",
                        RecurrencePattern::Monthly => " 🔄 (Recurring monthly)",
                        RecurrencePattern::Yearly => " 🔄 (Recurring yearly)",
                        RecurrencePattern::None => "",
                    };
                    Some(format!(
                        "⏱️ <b>Reminder #{} set!</b> I will remind you in <b>{}</b> (at {}) for: <i>\"{}\"</i>{}<br/><font color=\"#888888\" size=\"-1\">React: 👍️ Done · 👎️ Delete · 😝 +1 Day · 🎉 Celebrate</font>",
                        reminder.id,
                        format_duration(dur),
                        format_datetime(&trigger_at, &tz),
                        html_escape(&msg),
                        rec_note
                    ))
                }
                Err(ReminderError::EmptyMessage) => {
                    Some("⚠️ Reminder message required.".to_string())
                }
                Err(ReminderError::MessageTooLong { max }) => Some(format!(
                    "⚠️ Reminder message too long! Maximum limit is {} characters.",
                    max
                )),
                Err(ReminderError::TriggerInPast) => {
                    Some("⚠️ Duration is too large or out of range!".to_string())
                }
                Err(ReminderError::NotFound) => {
                    // Unreachable from `create`, kept for exhaustiveness.
                    Some("❌ Database error creating reminder.".to_string())
                }
                Err(ReminderError::Db(err)) => {
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

    async fn setup_test_db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:1".to_string(),
            sender_mxid: "@bob:1".to_string(),
            owner_id: "@bob:1".to_string(),
            sender_name: "bob".to_string(),
            command: "remind".to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    #[tokio::test]
    async fn test_handle_reminder_set_list_cancel() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let res_set =
            handle_reminder_command(&pool, &settings, None, &ctx("!remind 10s Check oven"))
                .await
                .unwrap();
        assert!(res_set.contains("Reminder #1 set!"));

        let res_list = handle_reminder_command(&pool, &settings, None, &ctx("!remind list"))
            .await
            .unwrap();
        assert!(res_list.contains("Pending Reminders for bob"));
        assert!(res_list.contains("Check oven"));

        let res_cancel = handle_reminder_command(&pool, &settings, None, &ctx("!remind cancel 1"))
            .await
            .unwrap();
        assert!(res_cancel.contains("Cancelled reminder"));

        let res_bare = handle_reminder_command(&pool, &settings, None, &ctx("!remind"))
            .await
            .unwrap();
        assert!(res_bare.contains("no pending reminders"));

        let res_reminders = handle_reminder_command(&pool, &settings, None, &ctx("!reminders"))
            .await
            .unwrap();
        assert!(res_reminders.contains("no pending reminders"));

        // Add a reminder and cancel via rm
        handle_reminder_command(&pool, &settings, None, &ctx("!remind 20s Check water")).await;
        let res_rm = handle_reminder_command(&pool, &settings, None, &ctx("!remind rm 2"))
            .await
            .unwrap();
        assert!(res_rm.contains("Cancelled reminder"));

        // Check history command
        let res_hist = handle_reminder_command(&pool, &settings, None, &ctx("!remind history"))
            .await
            .unwrap();
        assert!(res_hist.contains("Reminder History for bob"));
        assert!(res_hist.contains("Check oven"));
        assert!(res_hist.contains("[Cancelled 🗑️]"));
    }

    #[tokio::test]
    async fn test_handle_reminder_with_quoted_message() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );
        let quoted = crate::interface::channels::matrix::quote::QuotedMessage {
            author: Some("alice".to_string()),
            body: "Deployment scheduled at midnight".to_string(),
            event_id: None,
        };
        let mut quoted_ctx = ctx("!remind 10m");
        quoted_ctx.quoted = Some(quoted);

        let res = handle_reminder_command(&pool, &settings, None, &quoted_ctx)
            .await
            .unwrap();

        assert!(res.contains("Reminder #1 set!"));
        assert!(res.contains("Deployment scheduled at midnight"));
    }
}
