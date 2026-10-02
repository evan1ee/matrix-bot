//! HTML view rendering and listing sub-handlers for the To-Do module.

use chrono_tz::Tz;
use tracing::error;

use crate::application::todos::TodoService;
use crate::domain::accounts::AccountId;
use crate::domain::todos::TodoItem;
use crate::interface::channels::message::{Action, Reply};
use crate::utils::{format_datetime, html_escape, priority_badge};

pub const PAGE_SIZE: i64 = 25;

/// Telegram's inline keyboard limit; the other adapters ignore actions.
const MAX_DONE_ACTIONS: usize = 10;

/// One "Done" button per open item of the viewer's own, in list order (a room
/// list also shows other members' tasks, and `todo done` is owner-scoped). The command is the bare
/// text a user would type, so a press runs through the normal pipeline
/// (identity, scoping and `handle_done`), never a shortcut.
pub fn done_actions(owner_id: &AccountId, items: &[TodoItem]) -> Vec<Action> {
    items
        .iter()
        .filter(|item| !item.completed && item.owner_id == *owner_id)
        .take(MAX_DONE_ACTIONS)
        .map(|item| Action {
            label: format!("Done #{}", item.id),
            command: format!("todo done {}", item.id),
        })
        .collect()
}

/// The list as a reply: the HTML is `format_todo_list` unchanged.
pub fn list_reply(
    owner_id: &AccountId,
    title: &str,
    items: &[TodoItem],
    page: i64,
    tz: &Tz,
) -> Reply {
    Reply::html(format_todo_list(title, items, page, tz))
        .with_actions(done_actions(owner_id, items))
}

pub async fn handle_list_room(
    todos: &TodoService,
    owner_id: &AccountId,
    room_id: &str,
    args_body: &str,
    tz: &Tz,
) -> Option<Reply> {
    let rest = args_body.split_whitespace().nth(1).unwrap_or("1");
    let page: i64 = rest.parse().unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    match todos.list_room(room_id, PAGE_SIZE, offset).await {
        Ok(items) => Some(list_reply(owner_id, "Room Tasks", &items, page, tz)),
        Err(err) => {
            error!("Failed to fetch room todos: {:?}", err);
            Some("❌ Database error loading room tasks.".to_string().into())
        }
    }
}

pub async fn handle_list_user(
    todos: &TodoService,
    owner_id: &AccountId,
    sender_name: &str,
    args_body: &str,
    tz: &Tz,
) -> Option<Reply> {
    let rest = args_body.split_whitespace().nth(1).unwrap_or("1");
    let page: i64 = rest.parse().unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    match todos.list(owner_id, None, None, PAGE_SIZE, offset).await {
        Ok(items) => Some(list_reply(
            owner_id,
            &format!("Your Tasks ({})", html_escape(sender_name)),
            &items,
            page,
            tz,
        )),
        Err(err) => {
            error!("Failed to fetch user todos: {:?}", err);
            Some("❌ Database error loading tasks.".to_string().into())
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
        let badge = priority_badge(item.priority.as_str());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::todos::TodoPriority;

    fn owner() -> AccountId {
        AccountId::from_seed("t")
    }

    fn open_item(id: i64) -> TodoItem {
        TodoItem {
            id,
            room_id: "r".into(),
            owner_id: AccountId::from_seed("t"),
            sender_name: "Ann".into(),
            text: format!("task {id}"),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            completed: false,
            created_at: None,
        }
    }

    #[test]
    fn no_items_no_actions() {
        assert!(done_actions(&owner(), &[]).is_empty());
    }

    #[test]
    fn one_done_action_per_item_in_list_order() {
        let items: Vec<_> = [7, 3, 9].into_iter().map(open_item).collect();
        let actions = done_actions(&owner(), &items);
        assert_eq!(
            actions
                .iter()
                .map(|a| (a.label.as_str(), a.command.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("Done #7", "todo done 7"),
                ("Done #3", "todo done 3"),
                ("Done #9", "todo done 9")
            ]
        );
    }

    #[test]
    fn at_most_ten_actions_each_within_the_callback_limit() {
        let items: Vec<_> = (1..=12).map(|i| open_item(i64::MAX - i)).collect();
        let actions = done_actions(&owner(), &items);
        assert_eq!(actions.len(), 10);
        assert_eq!(actions[0].command, format!("todo done {}", i64::MAX - 1));
        assert!(actions.iter().all(|a| a.command.len() <= 64));
    }

    #[test]
    fn completed_items_get_no_button() {
        let mut done = open_item(1);
        done.completed = true;
        let actions = done_actions(&owner(), &[done, open_item(2)]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].command, "todo done 2");
    }

    #[test]
    fn the_list_reply_keeps_its_html_byte_for_byte_and_carries_the_actions() {
        let items = vec![open_item(1), open_item(2)];
        let reply = list_reply(&owner(), "Room Tasks", &items, 1, &chrono_tz::UTC);
        assert_eq!(
            reply.html_body(),
            "📋 <b>Room Tasks (Page 1):</b><br/>\
             • [<b>#1</b>] task 1 🟡 <b>[MED]</b> <i>(by Ann)</i><br/>\
             • [<b>#2</b>] task 2 🟡 <b>[MED]</b> <i>(by Ann)</i><br/>"
        );
        assert_eq!(reply.content.actions.len(), 2);
        assert_eq!(
            reply.html_body(),
            format_todo_list("Room Tasks", &items, 1, &chrono_tz::UTC)
        );
    }

    #[test]
    fn other_members_tasks_in_a_room_list_get_no_button() {
        let mut theirs = open_item(1);
        theirs.owner_id = AccountId::from_seed("someone-else");
        let actions = done_actions(&owner(), &[theirs, open_item(2)]);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].command, "todo done 2");
    }

    #[test]
    fn an_empty_list_has_no_actions() {
        let reply = list_reply(&owner(), "Room Tasks", &[], 1, &chrono_tz::UTC);
        assert!(reply.content.actions.is_empty());
    }
}
