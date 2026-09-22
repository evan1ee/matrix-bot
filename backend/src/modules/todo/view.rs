//! HTML view rendering and listing sub-handlers for the To-Do module.

use crate::db::DbPool;
use chrono_tz::Tz;
use tracing::error;

use crate::modules::todo::models::TodoItem;
use crate::modules::todo::repository::TodoRepository;
use crate::utils::{format_datetime, html_escape, priority_badge};

pub const PAGE_SIZE: i64 = 25;

pub async fn handle_list_room(
    pool: &DbPool,
    room_id: &str,
    args_body: &str,
    tz: &Tz,
) -> Option<String> {
    let rest = args_body.split_whitespace().nth(1).unwrap_or("1");
    let page: i64 = rest.parse().unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    match TodoRepository::list_room(pool, room_id, PAGE_SIZE, offset).await {
        Ok(items) => Some(format_todo_list("Room Tasks", &items, page, tz)),
        Err(err) => {
            error!("Failed to fetch room todos: {:?}", err);
            Some("❌ Database error loading room tasks.".to_string())
        }
    }
}

pub async fn handle_list_user(
    pool: &DbPool,
    owner_id: &str,
    sender_name: &str,
    args_body: &str,
    tz: &Tz,
) -> Option<String> {
    let rest = args_body.split_whitespace().nth(1).unwrap_or("1");
    let page: i64 = rest.parse().unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    match TodoRepository::list_all_user(pool, owner_id, None, PAGE_SIZE, offset).await {
        Ok(items) => Some(format_todo_list(
            &format!("Your Tasks ({})", html_escape(sender_name)),
            &items,
            page,
            tz,
        )),
        Err(err) => {
            error!("Failed to fetch user todos: {:?}", err);
            Some("❌ Database error loading tasks.".to_string())
        }
    }
}

pub fn format_todo_list(title: &str, items: &[TodoItem], page: i64, tz: &Tz) -> String {
    if items.is_empty() {
        if page > 1 {
            return format!(
                "📋 <b>{} (Page {}):</b><br/>No more tasks on this page.",
                title, page
            );
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
            format!(
                " 📅 <i>(Due: {})</i>",
                html_escape(&format_datetime(due, tz))
            )
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

    if items.len() as i64 == PAGE_SIZE {
        html.push_str(&format!(
            "<i>(Page {} full. Use <code>!todo list {}</code> for next page)</i>",
            page,
            page + 1
        ));
    }
    html
}
