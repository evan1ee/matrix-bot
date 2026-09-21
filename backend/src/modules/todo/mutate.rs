use crate::db::DbPool;
use chrono::Utc;
use chrono_tz::Tz;
use tracing::error;

use crate::events::EventBroadcaster;
use crate::modules::todo::due_reminder;
use crate::modules::todo::models::TodoPriority;
use crate::modules::todo::repository::TodoRepository;
use crate::modules::todo::service::{self, TodoError, TodoPatch};
use crate::utils::{format_datetime, html_escape, parse_reminder_args, priority_badge};

// `MAX_TASK_TEXT_LEN` and `MAX_ROOM_TASKS` live in `service` now — this module
// owns the reply wording, not the limits.
pub use crate::modules::todo::service::{MAX_ROOM_TASKS, MAX_TASK_TEXT_LEN};

pub const DUE_USAGE: &str =
    "⚠️ Usage: <code>!todo due &lt;id&gt; &lt;time&gt; [remind &lt;lead&gt;]</code> \
     (e.g. <code>!todo due 1 2d remind 1h</code>, or \
     <code>!todo due 1 none</code> to clear)";

pub async fn handle_add(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
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

    match service::create(
        pool,
        broadcaster,
        owner_id,
        service::NewTodo {
            text: raw_text,
            priority,
            room_id,
            sender_name,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
        },
    )
    .await
    {
        Ok(item) => Some(format!(
            "✅ Added Task <b>#{}</b> for {}: <i>\"{}\"</i> {}<br/><font color=\"#888888\" size=\"-1\">React: 👍️ Done · 👎️ Delete · 😝 +1 Day · 🎉 Celebrate</font>",
            item.id,
            html_escape(&item.sender_name),
            html_escape(&item.text),
            priority_badge(&item.priority)
        )),
        Err(TodoError::TextTooLong { max }) => Some(format!(
            "⚠️ Task description too long! Maximum limit is {} characters.",
            max
        )),
        Err(TodoError::RoomFull { max }) => Some(format!(
            "⚠️ Room task limit reached ({} active tasks). Clear completed tasks using \
             <code>!todo clear room</code>.",
            max
        )),
        Err(TodoError::Db(err)) => {
            error!("Failed to add todo item: {:?}", err);
            Some("❌ Failed to save task to database.".to_string())
        }
        Err(_) => Some("❌ Failed to save task to database.".to_string()),
    }
}

pub async fn handle_done(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    args_body: &str,
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

    // A finished task should stop nagging. Whether it *was* nagging has to be
    // read before the update — completing the task is exactly what removes
    // the signal we'd otherwise use to tell.
    let had_reminder = TodoRepository::find_owned(pool, id, owner_id)
        .await
        .ok()
        .flatten()
        .map(|existing| due_reminder::wanted_trigger(&existing).is_some())
        .unwrap_or(false);

    match service::update(
        pool,
        broadcaster,
        owner_id,
        id,
        TodoPatch {
            completed: Some(true),
            ..Default::default()
        },
    )
    .await
    {
        Ok(item) => Some(format!(
            "🎉 Marked Task <b>#{}</b> (<i>\"{}\"</i>) as completed! ✅{}",
            item.id,
            html_escape(&item.text),
            if had_reminder {
                " <i>(due reminder cancelled)</i>"
            } else {
                ""
            }
        )),
        Err(TodoError::NotFound) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(TodoError::Db(err)) => {
            error!("Failed to mark todo done: {:?}", err);
            Some("❌ Database error marking task completed.".to_string())
        }
        Err(_) => Some("❌ Database error marking task completed.".to_string()),
    }
}

pub async fn handle_edit(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    args_body: &str,
) -> Option<String> {
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

    let Ok(id) = id_str.parse::<i64>() else {
        return Some(
            "⚠️ Invalid task ID. Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>"
                .to_string(),
        );
    };

    match service::update(
        pool,
        broadcaster,
        owner_id,
        id,
        TodoPatch {
            text: Some(new_text),
            ..Default::default()
        },
    )
    .await
    {
        Ok(item) => Some(format!(
            "✏️ Updated Task <b>#{}</b> text to: <i>\"{}\"</i>",
            item.id,
            html_escape(&item.text)
        )),
        Err(TodoError::NotFound) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(TodoError::Db(err)) => {
            error!("Failed to edit todo text: {:?}", err);
            Some("❌ Database error updating task.".to_string())
        }
        Err(_) => Some("❌ Database error updating task.".to_string()),
    }
}

pub async fn handle_priority(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
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
    match service::update(
        pool,
        broadcaster,
        owner_id,
        id,
        TodoPatch {
            priority: Some(prio),
            ..Default::default()
        },
    )
    .await
    {
        Ok(item) => Some(format!(
            "🏷️ Set Task <b>#{}</b> priority to {}",
            item.id,
            priority_badge(&item.priority)
        )),
        Err(TodoError::NotFound) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(TodoError::Db(err)) => {
            error!("Failed to update todo priority: {:?}", err);
            Some("❌ Database error updating priority.".to_string())
        }
        Err(_) => Some("❌ Database error updating priority.".to_string()),
    }
}

pub async fn handle_due(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    room_id: &str,
    owner_id: &str,
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
        return match service::update(
            pool,
            broadcaster,
            owner_id,
            id,
            TodoPatch {
                due_at: Some(None),
                ..Default::default()
            },
        )
        .await
        {
            Ok(item) => Some(format!(
                "📅 Cleared the due date on Task <b>#{}</b>.",
                item.id
            )),
            Err(TodoError::NotFound) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
            Err(TodoError::Db(err)) => {
                error!("Failed to clear todo due date: {:?}", err);
                Some("❌ Database error clearing due date.".to_string())
            }
            Err(_) => Some("❌ Database error clearing due date.".to_string()),
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

    // An explicit `remind` replaces whatever was set before; omitting it
    // leaves an existing lead time alone, so re-dating a task keeps its
    // reminder — this is exactly `TodoPatch::remind_before_minutes`'s
    // "None leaves it alone, Some(x) sets it" semantics.
    let had_reminder_before = TodoRepository::find_owned(pool, id, owner_id)
        .await
        .ok()
        .flatten()
        .map(|existing| due_reminder::wanted_trigger(&existing).is_some())
        .unwrap_or(false);

    let result = service::update(
        pool,
        broadcaster,
        owner_id,
        id,
        TodoPatch {
            due_at: Some(Some(due_dt)),
            remind_before_minutes: lead_minutes.map(Some),
            remind_room_id: Some(room_id),
            ..Default::default()
        },
    )
    .await;

    match result {
        Ok(item) => {
            let due_note = format!(
                "📅 Set Task <b>#{}</b> due date to <b>{}</b>",
                item.id,
                html_escape(&format_datetime(&due_dt, tz))
            );

            let reminder_note = match due_reminder::wanted_trigger(&item) {
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
        Err(TodoError::NotFound) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(TodoError::Db(err)) => {
            error!("Failed to set todo due date: {:?}", err);
            Some("❌ Database error setting due date.".to_string())
        }
        Err(_) => Some("❌ Database error setting due date.".to_string()),
    }
}

pub async fn handle_clear(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    room_id: &str,
    owner_id: &str,
    sender_name: &str,
    args_body: &str,
) -> Option<String> {
    let target = args_body[5..].trim();
    let for_room = target == "room";
    let room_arg = for_room.then_some(room_id);

    match service::clear_completed(pool, broadcaster, owner_id, room_arg).await {
        Ok(items) => {
            let count = items.len();
            if for_room {
                Some(format!(
                    "🗑️ Cleared <b>{}</b> completed task(s) from this room!",
                    count
                ))
            } else {
                Some(format!(
                    "🗑️ Cleared <b>{}</b> of {}'s completed task(s) globally!",
                    count,
                    html_escape(sender_name)
                ))
            }
        }
        Err(TodoError::Db(err)) => {
            if for_room {
                error!("Failed to clear room todos: {:?}", err);
                Some("❌ Database error clearing room tasks.".to_string())
            } else {
                error!("Failed to clear user todos: {:?}", err);
                Some("❌ Database error clearing tasks.".to_string())
            }
        }
        Err(_) => Some("❌ Database error clearing tasks.".to_string()),
    }
}

pub async fn handle_delete(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
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

    match service::delete(pool, broadcaster, owner_id, id).await {
        Ok(item) => Some(format!(
            "🗑️ Deleted Task <b>#{}</b>: <i>\"{}\"</i>",
            id,
            html_escape(&item.text)
        )),
        Err(TodoError::NotFound) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
        Err(TodoError::Db(err)) => {
            error!("Failed to delete task {}: {:?}", id, err);
            Some("❌ Database error deleting task.".to_string())
        }
        Err(_) => Some("❌ Database error deleting task.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::{EventBroadcaster, SystemEvent};
    use sqlx::SqlitePool;

    async fn setup_test_db() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    /// The bug this refactor exists to fix: `!todo add` never reached an open
    /// browser because the Matrix path never broadcast anything.
    #[tokio::test]
    async fn test_a_matrix_add_announces_itself() {
        let pool = setup_test_db().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let reply = handle_add(
            &pool,
            Some(&broadcaster),
            "!room:1",
            "@alice:1",
            "alice",
            "add Buy milk",
            None,
        )
        .await
        .unwrap();
        assert!(reply.contains("Added Task"));

        let event = rx.try_recv().expect("the add must broadcast an event");
        assert!(matches!(event, SystemEvent::TodoCreated(_)));
    }
}
