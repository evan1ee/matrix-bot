use chrono::Utc;
use sqlx::SqlitePool;
use tracing::error;

use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::todo::repository::TodoRepository;
use crate::utils::{format_datetime, html_escape, parse_reminder_args, priority_badge};

pub async fn handle_todo_command(
    pool: &SqlitePool,
    room_id: &str,
    sender_mxid: &str,
    sender_name: &str,
    clean_body: &str,
) -> Option<String> {
    let clean_lower = clean_body.to_lowercase();

    // Catch bare "todo" or "!todo" without space
    if clean_lower == "todo" || clean_lower == "!todo" {
        let items = TodoRepository::list_user(pool, room_id, sender_mxid, 25, 0).await.ok().unwrap_or_default();
        return Some(format_todo_list(&format!("Your Tasks ({})", html_escape(sender_name)), &items, 1));
    }

    if !(clean_lower.starts_with("todo ") || clean_lower.starts_with("!todo ")) {
        return None;
    }

    let args_body = if clean_lower.starts_with("!todo ") {
        clean_body[6..].trim()
    } else {
        clean_body[5..].trim()
    };
    let args_lower = args_body.to_lowercase();

    // 1. !todo add <text> (supports inline priority like [high] or -p high)
    if args_lower.starts_with("add") {
        let mut raw_text = args_body[3..].trim();
        if raw_text.is_empty() {
            return Some("⚠️ Usage: <code>!todo add &lt;task description&gt;</code> (e.g. <code>!todo add [high] Buy milk</code>)".to_string());
        }

        let mut priority = TodoPriority::Medium;

        // Parse inline [high], [med], [low]
        if raw_text.starts_with('[') {
            if let Some(end_idx) = raw_text.find(']') {
                let prio_tag = &raw_text[1..end_idx];
                priority = TodoPriority::from_str(prio_tag);
                raw_text = raw_text[end_idx + 1..].trim();
            }
        } else if let Some(idx) = raw_text.find("-p ") {
            let (task_part, prio_part) = raw_text.split_at(idx);
            let prio_tag = prio_part.trim_start_matches("-p ").trim();
            priority = TodoPriority::from_str(prio_tag);
            raw_text = task_part.trim();
        }

        if raw_text.is_empty() {
            return Some("⚠️ Please specify task description after priority tag!".to_string());
        }

        // Security: Input length limit (500 chars)
        if raw_text.len() > 500 {
            return Some("⚠️ Task description too long! Maximum limit is 500 characters.".to_string());
        }

        // Quota check: max 500 active tasks per room
        match TodoRepository::count_room_active(pool, room_id).await {
            Ok(count) if count >= 500 => {
                return Some("⚠️ Room task limit reached (500 active tasks). Clear completed tasks using <code>!todo clear room</code>.".to_string());
            }
            Err(err) => {
                error!("Failed checking room task count: {:?}", err);
            }
            _ => {}
        }

        match TodoRepository::add(pool, room_id, sender_mxid, sender_name, raw_text, priority).await {
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
    // 2. !todo done <id>
    else if args_lower.starts_with("done") {
        let id_str = args_body[4..].trim();
        if id_str.is_empty() {
            return Some("⚠️ Usage: <code>!todo done &lt;id&gt;</code> (e.g. <code>!todo done 1</code>)".to_string());
        }

        if let Ok(id) = id_str.parse::<i64>() {
            match TodoRepository::mark_done(pool, room_id, id).await {
                Ok(Some(item)) => Some(format!(
                    "🎉 Marked Task <b>#{}</b> (<i>\"{}\"</i>) as completed! ✅",
                    item.id,
                    html_escape(&item.text)
                )),
                Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found in this room.", id)),
                Err(err) => {
                    error!("Failed to mark todo done: {:?}", err);
                    Some("❌ Database error marking task completed.".to_string())
                }
            }
        } else {
            Some("⚠️ Invalid task ID. Usage: <code>!todo done &lt;id&gt;</code>".to_string())
        }
    }
    // 3. !todo edit <id> <new_text>
    else if args_lower.starts_with("edit") {
        let rest = args_body[4..].trim();
        let mut parts = rest.splitn(2, ' ');
        let id_str = parts.next().unwrap_or("");
        let new_text = parts.next().unwrap_or("").trim();

        if id_str.is_empty() || new_text.is_empty() {
            return Some("⚠️ Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>".to_string());
        }

        if new_text.len() > 500 {
            return Some("⚠️ Task text too long! Maximum limit is 500 characters.".to_string());
        }

        if let Ok(id) = id_str.parse::<i64>() {
            match TodoRepository::edit_text(pool, room_id, id, new_text).await {
                Ok(Some(item)) => Some(format!(
                    "✏️ Updated Task <b>#{}</b> text to: <i>\"{}\"</i>",
                    item.id,
                    html_escape(&item.text)
                )),
                Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found in this room.", id)),
                Err(err) => {
                    error!("Failed to edit todo text: {:?}", err);
                    Some("❌ Database error updating task.".to_string())
                }
            }
        } else {
            Some("⚠️ Invalid task ID. Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>".to_string())
        }
    }
    // 4. !todo priority <id> <low|med|high>
    else if args_lower.starts_with("priority") {
        let rest = args_body[8..].trim();
        let mut parts = rest.splitn(2, ' ');
        let id_str = parts.next().unwrap_or("");
        let prio_str = parts.next().unwrap_or("").trim();

        if id_str.is_empty() || prio_str.is_empty() {
            return Some("⚠️ Usage: <code>!todo priority &lt;id&gt; &lt;low|medium|high&gt;</code>".to_string());
        }

        if let Ok(id) = id_str.parse::<i64>() {
            let prio = TodoPriority::from_str(prio_str);
            match TodoRepository::set_priority(pool, room_id, id, prio.clone()).await {
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
    // 5. !todo due <id> <time> (e.g. !todo due 1 2d)
    else if args_lower.starts_with("due") {
        let rest = args_body[3..].trim();
        let mut parts = rest.splitn(2, ' ');
        let id_str = parts.next().unwrap_or("");
        let time_arg = parts.next().unwrap_or("").trim();

        if id_str.is_empty() || time_arg.is_empty() {
            return Some("⚠️ Usage: <code>!todo due &lt;id&gt; &lt;time&gt;</code> (e.g. <code>!todo due 1 2d</code>)".to_string());
        }

        if let (Ok(id), Some((dur, _))) = (id_str.parse::<i64>(), parse_reminder_args(time_arg)) {
            let due_dt = Utc::now() + chrono::Duration::from_std(dur).unwrap_or(chrono::Duration::days(1));
            let due_str = format_datetime(&due_dt);

            match TodoRepository::set_due_date(pool, room_id, id, &due_str).await {
                Ok(Some(item)) => Some(format!(
                    "📅 Set Task <b>#{}</b> due date to <b>{}</b>",
                    item.id,
                    html_escape(&due_str)
                )),
                Ok(None) => Some(format!("⚠️ Task <b>#{}</b> not found.", id)),
                Err(err) => {
                    error!("Failed to set todo due date: {:?}", err);
                    Some("❌ Database error setting due date.".to_string())
                }
            }
        } else {
            Some("⚠️ Usage: <code>!todo due &lt;id&gt; &lt;time&gt;</code> (e.g. <code>!todo due 1 2d</code>)".to_string())
        }
    }
    // 6. !todo clear room / !todo clear
    else if args_lower.starts_with("clear") {
        let target = args_body[5..].trim();
        if target == "room" || target == "all" {
            match TodoRepository::clear_completed_room(pool, room_id).await {
                Ok(count) => Some(format!("🗑️ Cleared <b>{}</b> completed task(s) from the room!", count)),
                Err(err) => {
                    error!("Failed to clear room todos: {:?}", err);
                    Some("❌ Database error clearing room tasks.".to_string())
                }
            }
        } else {
            match TodoRepository::clear_completed_user(pool, room_id, sender_mxid).await {
                Ok(count) => Some(format!("🗑️ Cleared <b>{}</b> of your completed task(s)!", count)),
                Err(err) => {
                    error!("Failed to clear user todos: {:?}", err);
                    Some("❌ Database error clearing tasks.".to_string())
                }
            }
        }
    }
    // 7. !todo room [page] / !todo all [page]
    else if args_lower.starts_with("room") || args_lower.starts_with("all") {
        let rest = args_body.split_whitespace().nth(1).unwrap_or("1");
        let page: i64 = rest.parse().unwrap_or(1).max(1);
        let offset = (page - 1) * 25;

        match TodoRepository::list_room(pool, room_id, 25, offset).await {
            Ok(items) => Some(format_todo_list("Room Tasks", &items, page)),
            Err(err) => {
                error!("Failed to fetch room todos: {:?}", err);
                Some("❌ Database error loading room tasks.".to_string())
            }
        }
    }
    // 8. !todo [list|my] [page]
    else if args_lower.starts_with("list") || args_lower.starts_with("my") || args_lower.is_empty() {
        let rest = args_body.split_whitespace().nth(1).unwrap_or("1");
        let page: i64 = rest.parse().unwrap_or(1).max(1);
        let offset = (page - 1) * 25;

        match TodoRepository::list_user(pool, room_id, sender_mxid, 25, offset).await {
            Ok(items) => Some(format_todo_list(&format!("Your Tasks ({})", html_escape(sender_name)), &items, page)),
            Err(err) => {
                error!("Failed to fetch user todos: {:?}", err);
                Some("❌ Database error loading tasks.".to_string())
            }
        }
    } else {
        Some("⚠️ Unknown todo command. Available: <code>add</code>, <code>list</code>, <code>room</code>, <code>done</code>, <code>edit</code>, <code>priority</code>, <code>due</code>, <code>clear</code>".to_string())
    }
}

fn format_todo_list(title: &str, items: &[TodoItem], page: i64) -> String {
    if items.is_empty() {
        if page > 1 {
            return format!("📋 <b>{} (Page {}):</b><br/>No more tasks on this page.", title, page);
        }
        return format!(
            "📋 <b>{}:</b><br/>No active tasks! Add one using <code>!todo add &lt;task&gt;</code>.",
            title
        );
    }

    let mut html = format!("📋 <b>{} (Page {}):</b><br/>", title, page);
    for item in items {
        let badge = priority_badge(&item.priority);
        let due_note = if let Some(due) = &item.due_at {
            format!(" 📅 <i>(Due: {})</i>", html_escape(due))
        } else {
            String::new()
        };

        if item.completed {
            html.push_str(&format!(
                "• [<b>#{}</b>] <del>{}</del> ✅ {}{} <i>(by {})</i><br/>",
                item.id,
                html_escape(&item.text),
                badge,
                due_note,
                html_escape(&item.sender_name)
            ));
        } else {
            html.push_str(&format!(
                "• [<b>#{}</b>] {} {}{} <i>(by {})</i><br/>",
                item.id,
                html_escape(&item.text),
                badge,
                due_note,
                html_escape(&item.sender_name)
            ));
        }
    }

    if items.len() == 25 {
        html.push_str(&format!("<i>(Page {} full. Use <code>!todo list {}</code> for next page)</i>", page, page + 1));
    }
    html
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
    async fn test_handle_todo_add_and_done() {
        let pool = setup_test_db().await;

        let res = handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo add [high] Buy apples")
            .await
            .unwrap();
        assert!(res.contains("Added Task"));
        assert!(res.contains("Buy apples"));
        assert!(res.contains("HIGH"));

        let res_done = handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo done 1")
            .await
            .unwrap();
        assert!(res_done.contains("Marked Task"));

        let res_empty_add = handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo add")
            .await
            .unwrap();
        assert!(res_empty_add.contains("Usage:"));
    }
}
