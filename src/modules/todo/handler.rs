use sqlx::SqlitePool;
use tracing::error;

use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::todo::repository::TodoRepository;
use crate::utils::{html_escape, priority_badge};

pub async fn handle_todo_command(
    pool: &SqlitePool,
    room_id: &str,
    sender_mxid: &str,
    sender_name: &str,
    clean_body: &str,
) -> Option<String> {
    let clean_lower = clean_body.to_lowercase();

    // 1. !todo add <text>
    if clean_lower.starts_with("todo add ") || clean_lower.starts_with("!todo add ") {
        let text = clean_body
            .trim_start_matches("!todo add ")
            .trim_start_matches("todo add ")
            .trim();

        if text.is_empty() {
            return Some("⚠️ Please specify task text: <code>!todo add Buy groceries</code>".to_string());
        }

        match TodoRepository::add(pool, room_id, sender_mxid, sender_name, text, TodoPriority::Medium).await {
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
    else if clean_lower.starts_with("todo done ") || clean_lower.starts_with("!todo done ") {
        let id_str = clean_body
            .trim_start_matches("!todo done ")
            .trim_start_matches("todo done ")
            .trim();

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
            Some("⚠️ Usage: <code>!todo done &lt;id&gt;</code> (e.g. <code>!todo done 1</code>)".to_string())
        }
    }
    // 3. !todo edit <id> <new_text>
    else if clean_lower.starts_with("todo edit ") || clean_lower.starts_with("!todo edit ") {
        let args = clean_body
            .trim_start_matches("!todo edit ")
            .trim_start_matches("todo edit ")
            .trim();
        let mut parts = args.splitn(2, ' ');
        let id_str = parts.next().unwrap_or("");
        let new_text = parts.next().unwrap_or("").trim();

        if let (Ok(id), false) = (id_str.parse::<i64>(), new_text.is_empty()) {
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
            Some("⚠️ Usage: <code>!todo edit &lt;id&gt; &lt;new text&gt;</code>".to_string())
        }
    }
    // 4. !todo priority <id> <low|med|high>
    else if clean_lower.starts_with("todo priority ") || clean_lower.starts_with("!todo priority ") {
        let args = clean_body
            .trim_start_matches("!todo priority ")
            .trim_start_matches("todo priority ")
            .trim();
        let mut parts = args.splitn(2, ' ');
        let id_str = parts.next().unwrap_or("");
        let prio_str = parts.next().unwrap_or("").trim();

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
            Some("⚠️ Usage: <code>!todo priority &lt;id&gt; &lt;low|medium|high&gt;</code>".to_string())
        }
    }
    // 5. !todo clear room / !todo clear all
    else if clean_lower == "todo clear room"
        || clean_lower == "!todo clear room"
        || clean_lower == "todo clear all"
        || clean_lower == "!todo clear all"
    {
        match TodoRepository::clear_completed_room(pool, room_id).await {
            Ok(count) => Some(format!("🗑️ Cleared <b>{}</b> completed task(s) from the room!", count)),
            Err(err) => {
                error!("Failed to clear room todos: {:?}", err);
                Some("❌ Database error clearing room tasks.".to_string())
            }
        }
    }
    // 6. !todo clear
    else if clean_lower == "todo clear" || clean_lower == "!todo clear" {
        match TodoRepository::clear_completed_user(pool, room_id, sender_mxid).await {
            Ok(count) => Some(format!("🗑️ Cleared <b>{}</b> of your completed task(s)!", count)),
            Err(err) => {
                error!("Failed to clear user todos: {:?}", err);
                Some("❌ Database error clearing tasks.".to_string())
            }
        }
    }
    // 7. !todo room / !todo all
    else if clean_lower == "todo room"
        || clean_lower == "!todo room"
        || clean_lower == "todo all"
        || clean_lower == "!todo all"
    {
        match TodoRepository::list_room(pool, room_id).await {
            Ok(items) => Some(format_todo_list("Room Tasks", &items)),
            Err(err) => {
                error!("Failed to fetch room todos: {:?}", err);
                Some("❌ Database error loading room tasks.".to_string())
            }
        }
    }
    // 8. !todo / !todo list / !todo my
    else if clean_lower == "todo"
        || clean_lower == "!todo"
        || clean_lower == "todo list"
        || clean_lower == "!todo list"
        || clean_lower == "todo my"
        || clean_lower == "!todo my"
    {
        match TodoRepository::list_user(pool, room_id, sender_mxid).await {
            Ok(items) => Some(format_todo_list(&format!("Your Tasks ({})", html_escape(sender_name)), &items)),
            Err(err) => {
                error!("Failed to fetch user todos: {:?}", err);
                Some("❌ Database error loading tasks.".to_string())
            }
        }
    } else {
        None
    }
}

fn format_todo_list(title: &str, items: &[TodoItem]) -> String {
    if items.is_empty() {
        return format!(
            "📋 <b>{}:</b><br/>No active tasks! Add one using <code>!todo add &lt;task&gt;</code>.",
            title
        );
    }

    let mut html = format!("📋 <b>{}:</b><br/>", title);
    for item in items {
        let badge = priority_badge(&item.priority);
        if item.completed {
            html.push_str(&format!(
                "• [<b>#{}</b>] <del>{}</del> ✅ {} <i>(by {})</i><br/>",
                item.id,
                html_escape(&item.text),
                badge,
                html_escape(&item.sender_name)
            ));
        } else {
            html.push_str(&format!(
                "• [<b>#{}</b>] {} {} <i>(by {})</i><br/>",
                item.id,
                html_escape(&item.text),
                badge,
                html_escape(&item.sender_name)
            ));
        }
    }
    html
}
