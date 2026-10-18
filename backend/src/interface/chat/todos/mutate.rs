//! Chat command handlers for modifying todo tasks.

use chrono::Utc;
use chrono_tz::Tz;
use tracing::error;

use crate::application::todos::TodoService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::chat::QuotedMessage;
use crate::domain::reminders::MAX_LEAD_MINUTES;
use crate::domain::todos::{NewTodo, TodoPatch, TodoPriority, MAX_TEXT_LEN};
use crate::utils::{format_datetime, html_escape, parse_reminder_args, priority_badge};

pub const DUE_USAGE: &str =
    "⚠️ Usage: <code>!todo due &lt;id&gt; &lt;time&gt; [remind &lt;lead&gt;]</code> \
     (e.g. <code>!todo due 1 2d remind 1h</code>, or \
     <code>!todo due 1 none</code> to clear)";

pub async fn handle_add(
    todos: &TodoService,
    room_id: &str,
    owner_id: &AccountId,
    sender_name: &str,
    args_body: &str,
    quoted: Option<&QuotedMessage>,
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

    // Limit active tasks per room to 500
    match todos.count_room_active(room_id).await {
        Ok(count) if count >= 500 => {
            return Some(
                "⚠️ Room task limit reached (500 active tasks). Clear completed tasks using \
                 <code>!todo clear room</code>."
                    .to_string(),
            );
        }
        Err(err) => {
            error!("Failed checking room task count: {:?}", err);
        }
        _ => {}
    }

    match todos
        .create(
            owner_id,
            NewTodo {
                text: raw_text.to_string(),
                priority,
                room_id: room_id.to_string(),
                sender_name: sender_name.to_string(),
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None, tags: vec![], recurrence: crate::domain::reminders::RecurrencePattern::None, recurrence_anchor: None,
            },
        )
        .await
    {
        Ok((item, _)) => Some(format!(
            "✅ Added Task <b>#{}</b> for {}: <i>\"{}\"</i> {}<br/><font color=\"#888888\" size=\"-1\">React: 👍️ Done · 👎️ Delete · 😝 +1 Day · 🎉 Celebrate</font>",
            item.id,
            html_escape(&item.sender_name),
            html_escape(&item.text),
            priority_badge(item.priority.as_str())
        )),
        Err(AppError::Validation(msg)) if msg.contains("exceeds") => Some(format!(
            "⚠️ Task description too long! Maximum limit is {} characters.",
            MAX_TEXT_LEN
        )),
        Err(err) => {
            error!("Failed to add todo item: {:?}", err);
            Some("❌ Failed to save task to database.".to_string())
        }
    }
}

/// `"<br/>↻ Next: #id, due date"` for the successor a completion spawned.
fn next_occurrence_line(next: &crate::domain::todos::TodoItem, tz: &Tz) -> String {
    let due = next
        .due_at
        .map(|d| format_datetime(&d, tz))
        .unwrap_or_default();
    format!(
        "<br/>↻ Next: <b>#{}</b>, due <b>{}</b>",
        next.id,
        html_escape(&due)
    )
}

pub async fn handle_done(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
    tz: &Tz,
) -> Option<String> {
    let id_str = args_body[4..].trim();
    if id_str.is_empty() {
        return Some(
            "⚠️ Usage: <code>!todo done &lt;id&gt;</code> \
                      (e.g. <code>!todo done 1</code>)"
                .to_string(),
        );
    }

    let Ok(id) = id_str.parse::<i64>() else {
        return Some("⚠️ Invalid task ID. Usage: <code>!todo done &lt;id&gt;</code>".to_string());
    };

    let had_reminder = todos
        .find_owned(owner_id, id)
        .await
        .ok()
        .flatten()
        .map(|existing| crate::domain::todos::wanted_trigger(&existing).is_some())
        .unwrap_or(false);

    match todos.set_completed(owner_id, id, true).await {
        Ok(outcome) => Some(format!(
            "🎉 Marked Task <b>#{}</b> (<i>\"{}\"</i>) as completed! ✅{}{}",
            outcome.todo.id,
            html_escape(&outcome.todo.text),
            if had_reminder {
                " <i>(due reminder cancelled)</i>"
            } else {
                ""
            },
            outcome
                .next
                .as_ref()
                .map(|next| next_occurrence_line(next, tz))
                .unwrap_or_default()
        )),
        Err(AppError::NotFound(_)) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to mark todo done: {:?}", err);
            Some("❌ Database error marking task completed.".to_string())
        }
    }
}

pub async fn handle_edit(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
) -> Option<String> {
    let rest = args_body[4..].trim();
    let mut parts = rest.splitn(2, ' ');
    let id_str = parts.next().unwrap_or("");
    let new_text = parts.next().unwrap_or("").trim();

    if id_str.is_empty() || new_text.is_empty() {
        return Some("⚠️ Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>".to_string());
    }

    if new_text.len() > MAX_TEXT_LEN {
        return Some(format!(
            "⚠️ Task text too long! Maximum limit is {} characters.",
            MAX_TEXT_LEN
        ));
    }

    let Ok(id) = id_str.parse::<i64>() else {
        return Some(
            "⚠️ Invalid task ID. Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>"
                .to_string(),
        );
    };

    match todos
        .update(
            owner_id,
            id,
            TodoPatch {
                text: Some(new_text.to_string()),
                ..Default::default()
            },
        )
        .await
    {
        Ok((item, _)) => Some(format!(
            "✏️ Updated Task <b>#{}</b> text to: <i>\"{}\"</i>",
            item.id,
            html_escape(&item.text)
        )),
        Err(AppError::NotFound(_)) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to edit todo text: {:?}", err);
            Some("❌ Database error updating task.".to_string())
        }
    }
}

pub async fn handle_priority(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
) -> Option<String> {
    let rest = args_body[8..].trim();
    let mut parts = rest.splitn(2, ' ');
    let id_str = parts.next().unwrap_or("");
    let prio_str = parts.next().unwrap_or("").trim();

    if id_str.is_empty() || prio_str.is_empty() {
        return Some(
            "⚠️ Usage: <code>!todo priority &lt;id&gt; &lt;low|medium|high&gt;</code>".to_string(),
        );
    }

    let Ok(id) = id_str.parse::<i64>() else {
        return Some(
            "⚠️ Invalid task ID. Usage: <code>!todo priority &lt;id&gt; &lt;low|medium|high&gt;</code>"
                .to_string(),
        );
    };

    let prio = TodoPriority::from_str_or_default(prio_str);
    match todos
        .update(
            owner_id,
            id,
            TodoPatch {
                priority: Some(prio),
                ..Default::default()
            },
        )
        .await
    {
        Ok((item, _)) => Some(format!(
            "🏷️ Set Task <b>#{}</b> priority to {}",
            item.id,
            priority_badge(item.priority.as_str())
        )),
        Err(AppError::NotFound(_)) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to update todo priority: {:?}", err);
            Some("❌ Database error updating priority.".to_string())
        }
    }
}

pub async fn handle_due(
    todos: &TodoService,
    room_id: &str,
    owner_id: &AccountId,
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

    let (due_arg, lead_arg) = match time_arg.split_once(" remind ") {
        Some((due, lead)) => (due.trim(), Some(lead.trim())),
        None => (time_arg, None),
    };

    if matches!(due_arg.to_lowercase().as_str(), "none" | "clear") {
        return match todos
            .update(
                owner_id,
                id,
                TodoPatch {
                    due_at: Some(None),
                    ..Default::default()
                },
            )
            .await
        {
            Ok((item, _)) => Some(format!(
                "📅 Cleared the due date on Task <b>#{}</b>.",
                item.id
            )),
            Err(AppError::NotFound(_)) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
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
                if minutes <= 0 || minutes > MAX_LEAD_MINUTES {
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

    let had_reminder_before = todos
        .find_owned(owner_id, id)
        .await
        .ok()
        .flatten()
        .map(|existing| crate::domain::todos::wanted_trigger(&existing).is_some())
        .unwrap_or(false);

    let result = todos
        .update(
            owner_id,
            id,
            TodoPatch {
                due_at: Some(Some(due_dt)),
                remind_before_minutes: lead_minutes.map(Some),
                remind_room_id: Some(room_id.to_string()),
                ..Default::default()
            },
        )
        .await;

    match result {
        Ok((item, _)) => {
            let due_note = format!(
                "📅 Set Task <b>#{}</b> due date to <b>{}</b>",
                item.id,
                html_escape(&format_datetime(&due_dt, tz))
            );

            let reminder_note = match crate::domain::todos::wanted_trigger(&item) {
                Some(trigger) if trigger > Utc::now() => format!(
                    "<br/>⏰ Reminder set for <b>{}</b>.",
                    html_escape(&format_datetime(&trigger, tz))
                ),
                Some(_) => "<br/>⚠️ That lead time is already in the past, so no reminder was set."
                    .to_string(),
                None if had_reminder_before => {
                    "<br/>⏰ Previous due reminder cancelled.".to_string()
                }
                None => String::new(),
            };

            Some(format!("{}{}", due_note, reminder_note))
        }
        Err(AppError::NotFound(_)) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to set todo due date: {:?}", err);
            Some("❌ Database error setting due date.".to_string())
        }
    }
}

pub async fn handle_clear(
    todos: &TodoService,
    room_id: &str,
    owner_id: &AccountId,
    sender_name: &str,
    args_body: &str,
) -> Option<String> {
    let target = args_body[5..].trim();
    let for_room = target == "room";

    if for_room {
        match todos.clear_completed_room(room_id).await {
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
        match todos.clear_completed(owner_id).await {
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

pub async fn handle_delete(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
) -> Option<String> {
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

    let existing = todos.find_owned(owner_id, id).await.ok().flatten();

    match todos.delete(owner_id, id).await {
        Ok(_) => {
            let task_text = existing.map(|t| t.text).unwrap_or_default();
            Some(format!(
                "🗑️ Deleted Task <b>#{}</b>: <i>\"{}\"</i>",
                id,
                html_escape(&task_text)
            ))
        }
        Err(AppError::NotFound(_)) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(err) => {
            error!("Failed to delete task {}: {:?}", id, err);
            Some("❌ Database error deleting task.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::reminders::ReminderService;
    use crate::infrastructure::postgres::repositories;

    async fn setup_service() -> (TodoService, sqlx::PgPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:1"]).await;
        let reminders = ReminderService::new(pool.clone());
        let todos = TodoService::new(pool.clone(), reminders);
        (todos, pool)
    }

    #[tokio::test]
    async fn test_a_matrix_add_announces_itself() {
        let (todos, pool) = setup_service().await;
        let owner = AccountId::from_seed("@alice:1");

        let reply = handle_add(&todos, "!room:1", &owner, "alice", "add Buy milk", None)
            .await
            .unwrap();
        assert!(reply.contains("Added Task"));

        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["TodoCreated"]
        );
    }

    /// Given a recurring todo, When `!todo done` completes it, Then the reply
    /// names the spawned next occurrence and its due date.
    #[tokio::test]
    async fn done_on_a_recurring_todo_announces_the_next_occurrence() {
        let (todos, _pool) = setup_service().await;
        let owner = AccountId::from_seed("@alice:1");
        let due = crate::utils::time::now_at_storage_precision() + chrono::Duration::days(1);
        let input = NewTodo {
            room_id: "!room:1".to_string(),
            sender_name: "alice".to_string(),
            text: "water <plants>".to_string(),
            priority: TodoPriority::Medium,
            due_at: Some(due),
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: crate::domain::reminders::RecurrencePattern::Daily,
            recurrence_anchor: None,
        };
        let (item, _) = todos.create(&owner, input).await.unwrap();

        let reply = handle_done(&todos, &owner, &format!("done {}", item.id), &Tz::UTC)
            .await
            .unwrap();

        let next = todos.find_owned(&owner, item.id).await.unwrap().unwrap();
        let next_id = next.next_todo_id.unwrap();
        let next_due = format_datetime(&(due + chrono::Duration::days(1)), &Tz::UTC);
        assert!(reply.contains("water &lt;plants&gt;"), "{reply}");
        assert!(
            reply.contains(&format!(
                "<br/>↻ Next: <b>#{next_id}</b>, due <b>{next_due}</b>"
            )),
            "{reply}"
        );
    }
}
