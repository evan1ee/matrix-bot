//! Command handler and dispatcher for todo chat commands.

use crate::application::settings::SettingsService;
use crate::application::todos::TodoService;
use crate::interface::chat::CommandContext;
use crate::utils::html_escape;

use super::mutate::{
    handle_add, handle_clear, handle_delete, handle_done, handle_due, handle_edit, handle_priority,
};
use super::view::{format_todo_list, handle_list_room, handle_list_user, PAGE_SIZE};

pub async fn handle_todo_command(
    todos: &TodoService,
    settings: &SettingsService,
    ctx: &CommandContext,
) -> Option<String> {
    let room_id = ctx.room_id.as_str();
    let owner_id = &ctx.owner_id;
    let sender_name = ctx.sender_name.as_str();
    let clean_body = ctx.full_body.as_str();
    let quoted = ctx.quoted.as_ref();

    let clean_lower = clean_body.to_lowercase();

    // Bare "todo" or "!todo" without a space still claims the message.
    let bare = clean_lower == "todo" || clean_lower == "!todo";
    if !(bare || clean_lower.starts_with("todo ") || clean_lower.starts_with("!todo ")) {
        return None;
    }

    let tz = settings.get_or_default(owner_id).await.tz();

    if bare {
        let items = todos
            .list(owner_id, None, None, PAGE_SIZE, 0)
            .await
            .unwrap_or_default();
        return Some(format_todo_list(
            &format!("Your Tasks ({})", html_escape(sender_name)),
            &items,
            1,
            &tz,
        ));
    }

    let args_body = if clean_lower.starts_with("!todo ") {
        clean_body[6..].trim()
    } else {
        clean_body[5..].trim()
    };
    let args_lower = args_body.to_lowercase();

    if args_lower.starts_with("add") {
        handle_add(todos, room_id, owner_id, sender_name, args_body, quoted).await
    } else if args_lower.starts_with("done") {
        handle_done(todos, owner_id, args_body).await
    } else if args_lower.starts_with("edit") {
        handle_edit(todos, owner_id, args_body).await
    } else if args_lower.starts_with("priority") {
        handle_priority(todos, owner_id, args_body).await
    } else if args_lower.starts_with("due") {
        handle_due(todos, room_id, owner_id, args_body, &tz).await
    } else if args_lower.starts_with("clear") {
        handle_clear(todos, room_id, owner_id, sender_name, args_body).await
    } else if args_lower.starts_with("delete") || args_lower.starts_with("rm") {
        handle_delete(todos, owner_id, args_body).await
    } else if args_lower.starts_with("room") {
        handle_list_room(todos, room_id, args_body, &tz).await
    } else if args_lower.starts_with("list")
        || args_lower.starts_with("my")
        || args_lower.starts_with("all")
        || args_lower.starts_with("global")
        || args_lower.is_empty()
    {
        handle_list_user(todos, owner_id, sender_name, args_body, &tz).await
    } else {
        Some("⚠️ Unknown todo command. Available: <code>add</code>, <code>list</code>, \
              <code>all</code>, <code>room</code>, <code>done</code>, <code>edit</code>, <code>priority</code>, \
              <code>due</code>, <code>clear</code>, <code>delete</code>".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::reminders::ReminderService;
    use crate::domain::accounts::AccountId;
    use crate::events::EventBroadcaster;

    async fn setup_services() -> (TodoService, SettingsService) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let broadcaster = EventBroadcaster::default();
        let settings = SettingsService::new(pool.clone(), broadcaster.clone());
        let reminders = ReminderService::new(pool.clone(), broadcaster.clone());
        let todos = TodoService::new(pool, broadcaster, reminders);
        (todos, settings)
    }

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:1".to_string(),
            sender_mxid: "@bob:1".to_string(),
            owner_id: AccountId::new("@bob:1"),
            sender_name: "bob".to_string(),
            command: "todo".to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    #[tokio::test]
    async fn test_handle_todo_add_and_done() {
        let (todos, settings) = setup_services().await;

        let res_add = handle_todo_command(&todos, &settings, &ctx("!todo add Buy groceries"))
            .await
            .unwrap();
        assert!(res_add.contains("Added Task <b>#1</b>"));
        assert!(res_add.contains("Buy groceries"));

        let res_list = handle_todo_command(&todos, &settings, &ctx("!todo list"))
            .await
            .unwrap();
        assert!(res_list.contains("Buy groceries"));

        let res_done = handle_todo_command(&todos, &settings, &ctx("!todo done 1"))
            .await
            .unwrap();
        assert!(res_done.contains("Marked Task <b>#1</b>"));
        assert!(res_done.contains("as completed"));
    }

    #[tokio::test]
    async fn test_handle_todo_edit_and_priority() {
        let (todos, settings) = setup_services().await;

        handle_todo_command(&todos, &settings, &ctx("!todo add Initial task")).await;

        let res_edit =
            handle_todo_command(&todos, &settings, &ctx("!todo edit 1 Updated task text"))
                .await
                .unwrap();
        assert!(res_edit.contains("Updated Task <b>#1</b> text to: <i>\"Updated task text\"</i>"));

        let res_prio = handle_todo_command(&todos, &settings, &ctx("!todo priority 1 high"))
            .await
            .unwrap();
        assert!(res_prio.contains("Set Task <b>#1</b> priority to"));
        assert!(res_prio.contains("HIGH"));
    }

    #[tokio::test]
    async fn test_handle_todo_delete() {
        let (todos, settings) = setup_services().await;

        handle_todo_command(&todos, &settings, &ctx("!todo add Delete me")).await;

        let res_del = handle_todo_command(&todos, &settings, &ctx("!todo delete 1"))
            .await
            .unwrap();
        assert!(res_del.contains("Deleted Task <b>#1</b>: <i>\"Delete me\"</i>"));

        let res_del_again = handle_todo_command(&todos, &settings, &ctx("!todo delete 1"))
            .await
            .unwrap();
        assert!(res_del_again.contains("Task <b>#1</b> not found"));
    }

    #[tokio::test]
    async fn test_handle_todo_add_with_quoted_message() {
        let (todos, settings) = setup_services().await;

        let quoted = crate::domain::chat::QuotedMessage {
            author: Some("alice".to_string()),
            body: "Remember to review PR #42".to_string(),
            event_id: None,
        };
        let mut quoted_ctx = ctx("!todo add");
        quoted_ctx.quoted = Some(quoted);

        let res = handle_todo_command(&todos, &settings, &quoted_ctx)
            .await
            .unwrap();
        assert!(res.contains("Added Task <b>#1</b>"));
        assert!(res.contains("Remember to review PR #42"));
    }
}
