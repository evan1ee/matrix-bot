use chrono::Utc;
use chrono_tz::Tz;
use sqlx::SqlitePool;
use tracing::error;

use crate::modules::todo::due_reminder;
use crate::modules::todo::models::TodoPriority;
use crate::modules::todo::repository::TodoRepository;
use crate::utils::{format_datetime, html_escape, parse_reminder_args, priority_badge};

/// Maximum allowed length for task description text.
pub const MAX_TASK_TEXT_LEN: usize = 500;
/// Maximum active tasks per room.
pub const MAX_ROOM_TASKS: i64 = 500;

pub const DUE_USAGE: &str =
    "⚠️ Usage: <code>!todo due &lt;id&gt; &lt;time&gt; [remind &lt;lead&gt;]</code> \
     (e.g. <code>!todo due 1 2d remind 1h</code>, or \
     <code>!todo due 1 none</code> to clear)";

pub async fn handle_add(
    pool: &SqlitePool,
    room_id: &str,
    owner_id: &str,
    sender_name: &str,
    args_body: &str,
    quoted: Option<&crate::bot::quote::QuotedMessage>,
) -> Option<String> {
    let mut raw_text = args_body[3..].trim();
    let quoted_text_holder;
    if raw_text.is_empty() {
        if let Some(q) = quoted {
            quoted_text_holder = q.body.trim().to_string();
            raw_text = &quoted_text_holder;
        } else {
            return Some(
                "⚠️ Usage: <code>!todo add &lt;task description&gt;</code> \
                          (e.g. <code>!todo add [high] Buy milk</code>)"
                    .to_string(),
            );
        }
    }

    let mut priority = TodoPriority::Medium;

    // Parse inline [high], [med], [low]
    if raw_text.starts_with('[') {
        if let Some(end_idx) = raw_text.find(']') {
            let prio_tag = &raw_text[1..end_idx];
            priority = TodoPriority::from_str_or_default(prio_tag);
            raw_text = raw_text[end_idx + 1..].trim();
        }
    } else if let Some(idx) = raw_text.find("-p ") {
        let (task_part, prio_part) = raw_text.split_at(idx);
        let prio_tag = prio_part.trim_start_matches("-p ").trim();
        priority = TodoPriority::from_str_or_default(prio_tag);
        raw_text = task_part.trim();
    }

    if raw_text.is_empty() {
        return Some("⚠️ Please specify task description after priority tag!".to_string());
    }

    if raw_text.len() > MAX_TASK_TEXT_LEN {
        return Some(format!(
            "⚠️ Task description too long! Maximum limit is {} characters.",
            MAX_TASK_TEXT_LEN
        ));
    }

    match TodoRepository::count_room_active(pool, room_id).await {
        Ok(count) if count >= MAX_ROOM_TASKS => {
            return Some(format!(
                "⚠️ Room task limit reached ({} active tasks). Clear completed tasks using \
                 <code>!todo clear room</code>.",
                MAX_ROOM_TASKS
            ));
        }
        Err(err) => {
            error!("Failed checking room task count: {:?}", err);
        }
        _ => {}
    }

    match TodoRepository::add(
        pool,
        room_id,
        owner_id,
        sender_name,
        raw_text,
        priority,
        None,
        None,
    )
    .await
    {
        Ok(item) => Some(format!(
            "✅ Added Task <b>#{}</b> for {}: <i>\"{}\"</i> {}",
            item.id,
            html_escape(&item.sender_name),
            html_escape(&item.text),
            priority_badge(&item.priority)
        )),
        Err(err) => {
            error!("Failed to add todo item: {:?}", err);
            Some("❌ Failed to save task to database.".to_string())
        }
    }
}

pub async fn handle_done(pool: &SqlitePool, room_id: &str, args_body: &str) -> Option<String> {
    let id_str = args_body[4..].trim();
    if id_str.is_empty() {
        return Some(
            "⚠️ Usage: <code>!todo done &lt;id&gt;</code> \
                      (e.g. <code>!todo done 1</code>)"
                .to_string(),
        );
    }

    if let Ok(id) = id_str.parse::<i64>() {
        match TodoRepository::mark_done(pool, id).await {
            Ok(Some(item)) => {
                // A finished task should stop nagging.
                let cancelled = matches!(
                    due_reminder::sync(pool, None, &item, room_id).await,
                    Ok(due_reminder::SyncOutcome::Cancelled)
                );
                Some(format!(
                    "🎉 Marked Task <b>#{}</b> (<i>\"{}\"</i>) as completed! ✅{}",
                    item.id,
                    html_escape(&item.text),
                    if cancelled {
                        " <i>(due reminder cancelled)</i>"
                    } else {
                        ""
                    }
                ))
            }
            Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
            Err(err) => {
                error!("Failed to mark todo done: {:?}", err);
                Some("❌ Database error marking task completed.".to_string())
            }
        }
    } else {
        Some("⚠️ Invalid task ID. Usage: <code>!todo done &lt;id&gt;</code>".to_string())
    }
}

pub async fn handle_edit(pool: &SqlitePool, args_body: &str) -> Option<String> {
    let rest = args_body[4..].trim();
    let mut parts = rest.splitn(2, ' ');
    let id_str = parts.next().unwrap_or("");
    let new_text = parts.next().unwrap_or("").trim();

    if id_str.is_empty() || new_text.is_empty() {
        return Some("⚠️ Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>".to_string());
    }

    if new_text.len() > MAX_TASK_TEXT_LEN {
        return Some(format!(
            "⚠️ Task text too long! Maximum limit is {} characters.",
            MAX_TASK_TEXT_LEN
        ));
    }

    if let Ok(id) = id_str.parse::<i64>() {
        match TodoRepository::edit_text(pool, id, new_text).await {
            Ok(Some(item)) => Some(format!(
                "✏️ Updated Task <b>#{}</b> text to: <i>\"{}\"</i>",
                item.id,
                html_escape(&item.text)
            )),
            Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
            Err(err) => {
                error!("Failed to edit todo text: {:?}", err);
                Some("❌ Database error updating task.".to_string())
            }
        }
    } else {
        Some(
            "⚠️ Invalid task ID. Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>"
                .to_string(),
        )
    }
}

pub async fn handle_priority(pool: &SqlitePool, args_body: &str) -> Option<String> {
    let rest = args_body[8..].trim();
    let mut parts = rest.splitn(2, ' ');
    let id_str = parts.next().unwrap_or("");
    let prio_str = parts.next().unwrap_or("").trim();

    if id_str.is_empty() || prio_str.is_empty() {
        return Some(
            "⚠️ Usage: <code>!todo priority &lt;id&gt; &lt;low|medium|high&gt;</code>".to_string(),
        );
    }

    if let Ok(id) = id_str.parse::<i64>() {
        let prio = TodoPriority::from_str_or_default(prio_str);
        match TodoRepository::set_priority(pool, id, prio.clone()).await {
            Ok(Some(item)) => Some(format!(
                "🏷️ Set Task <b>#{}</b> priority to {}",
                item.id,
                priority_badge(&item.priority)
            )),
            Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
            Err(err) => {
                error!("Failed to update todo priority: {:?}", err);
                Some("❌ Database error updating priority.".to_string())
            }
        }
    } else {
        Some("⚠️ Invalid task ID. Usage: <code>!todo priority &lt;id&gt; &lt;low|medium|high&gt;</code>".to_string())
    }
}

pub async fn handle_due(
    pool: &SqlitePool,
    room_id: &str,
    args_body: &str,
    tz: &Tz,
) -> Option<String> {
    let rest = args_body[3..].trim();
    let mut parts = rest.splitn(2, ' ');
    let id_str = parts.next().unwrap_or("");
    let time_arg = parts.next().unwrap_or("").trim();

    if id_str.is_empty() || time_arg.is_empty() {
        return Some(DUE_USAGE.to_string());
    }

    let Ok(id) = id_str.parse::<i64>() else {
        return Some(DUE_USAGE.to_string());
    };

    // `remind <lead>` is an optional suffix on the due time.
    let (due_arg, lead_arg) = match time_arg.split_once(" remind ") {
        Some((due, lead)) => (due.trim(), Some(lead.trim())),
        None => (time_arg, None),
    };

    if matches!(due_arg.to_lowercase().as_str(), "none" | "clear") {
        return match TodoRepository::set_due_date(pool, id, None).await {
            Ok(Some(item)) => {
                due_reminder::tear_down(pool, None, item.id).await.ok();
                Some(format!(
                    "📅 Cleared the due date on Task <b>#{}</b>.",
                    item.id
                ))
            }
            Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
            Err(err) => {
                error!("Failed to clear todo due date: {:?}", err);
                Some("❌ Database error clearing due date.".to_string())
            }
        };
    }

    let Some((dur, _)) = parse_reminder_args(due_arg) else {
        return Some(DUE_USAGE.to_string());
    };
    let due_dt = Utc::now() + chrono::Duration::from_std(dur).unwrap_or(chrono::Duration::days(1));

    let lead_minutes = match lead_arg {
        Some(raw) => match parse_reminder_args(raw) {
            Some((lead, _)) => {
                let minutes = (lead.as_secs() / 60) as i64;
                if minutes <= 0 || minutes > due_reminder::MAX_LEAD_MINUTES {
                    return Some(
                        "⚠️ The reminder lead time must be between 1 minute and 30 days."
                            .to_string(),
                    );
                }
                Some(minutes)
            }
            None => return Some(DUE_USAGE.to_string()),
        },
        None => None,
    };

    if let Err(err) = TodoRepository::set_due_date(pool, id, Some(due_dt)).await {
        error!("Failed to set todo due date: {:?}", err);
        return Some("❌ Database error setting due date.".to_string());
    }

    // An explicit `remind` replaces whatever was set before; omitting it leaves
    // an existing lead time alone, so re-dating a task keeps its reminder.
    if lead_minutes.is_some() {
        if let Err(err) = TodoRepository::set_remind_before(pool, id, lead_minutes).await {
            error!("Failed to set todo reminder lead time: {:?}", err);
            return Some("❌ Database error setting reminder lead time.".to_string());
        }
    }

    let item = match TodoRepository::find(pool, id).await {
        Ok(Some(item)) => item,
        Ok(None) => return Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to re-read todo after setting due date: {:?}", err);
            return Some("❌ Database error setting due date.".to_string());
        }
    };

    let due_note = format!(
        "📅 Set Task <b>#{}</b> due date to <b>{}</b>",
        item.id,
        html_escape(&format_datetime(&due_dt, tz))
    );

    let reminder_note = match due_reminder::sync(pool, None, &item, room_id).await {
        Ok(due_reminder::SyncOutcome::Scheduled(at)) => format!(
            "<br/>⏰ Reminder set for <b>{}</b>.",
            html_escape(&format_datetime(&at, tz))
        ),
        Ok(due_reminder::SyncOutcome::LeadTimeInPast(_)) => {
            "<br/>⚠️ That lead time is already in the past, so no reminder was set.".to_string()
        }
        Ok(due_reminder::SyncOutcome::Cancelled) => {
            "<br/>⏰ Previous due reminder cancelled.".to_string()
        }
        Ok(due_reminder::SyncOutcome::Nothing) => String::new(),
        Err(err) => {
            error!("Failed to sync due reminder: {:?}", err);
            "<br/>❌ Due date saved, but the reminder could not be scheduled.".to_string()
        }
    };

    Some(format!("{}{}", due_note, reminder_note))
}

pub async fn handle_clear(
    pool: &SqlitePool,
    room_id: &str,
    owner_id: &str,
    sender_name: &str,
    args_body: &str,
) -> Option<String> {
    let target = args_body[5..].trim();
    if target == "room" {
        match TodoRepository::clear_completed_room(pool, room_id).await {
            Ok(count) => Some(format!(
                "🗑️ Cleared <b>{}</b> completed task(s) from this room!",
                count
            )),
            Err(err) => {
                error!("Failed to clear room todos: {:?}", err);
                Some("❌ Database error clearing room tasks.".to_string())
            }
        }
    } else {
        match TodoRepository::clear_completed_user(pool, owner_id).await {
            Ok(count) => Some(format!(
                "🗑️ Cleared <b>{}</b> of {}'s completed task(s) globally!",
                count,
                html_escape(sender_name)
            )),
            Err(err) => {
                error!("Failed to clear user todos: {:?}", err);
                Some("❌ Database error clearing tasks.".to_string())
            }
        }
    }
}

pub async fn handle_delete(pool: &SqlitePool, owner_id: &str, args_body: &str) -> Option<String> {
    let id_str = args_body
        .trim_start_matches("delete")
        .trim_start_matches("rm")
        .trim();
    if id_str.is_empty() {
        return Some(
            "⚠️ Usage: <code>!todo delete &lt;id&gt;</code> (or <code>!todo rm &lt;id&gt;</code>)"
                .to_string(),
        );
    }

    let Ok(id) = id_str.parse::<i64>() else {
        return Some("⚠️ Task ID must be a number.".to_string());
    };

    match TodoRepository::find_owned(pool, id, owner_id).await {
        Ok(Some(item)) => {
            due_reminder::tear_down(pool, None, id).await.ok();
            match TodoRepository::delete(pool, id).await {
                Ok(true) => Some(format!(
                    "🗑️ Deleted Task <b>#{}</b>: <i>\"{}\"</i>",
                    id,
                    html_escape(&item.text)
                )),
                _ => Some(format!("⚠️ Failed to delete task <b>#{}</b>.", id)),
            }
        }
        Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to delete task {}: {:?}", id, err);
            Some("❌ Database error deleting task.".to_string())
        }
    }
}
