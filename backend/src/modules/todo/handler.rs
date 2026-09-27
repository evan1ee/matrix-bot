use crate::application::settings::SettingsService;
use crate::db::DbPool;
use crate::events::EventBroadcaster;
use crate::modules::todo::mutate::{
    handle_add, handle_clear, handle_delete, handle_done, handle_due, handle_edit, handle_priority,
};
use crate::modules::todo::repository::TodoRepository;
use crate::modules::todo::view::{format_todo_list, handle_list_room, handle_list_user, PAGE_SIZE};
use crate::modules::CommandContext;
use crate::utils::html_escape;

pub use crate::modules::todo::mutate::{MAX_ROOM_TASKS, MAX_TASK_TEXT_LEN};

pub async fn handle_todo_command(
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

    // Bare "todo" or "!todo" without a space still claims the message.
    let bare = clean_lower == "todo" || clean_lower == "!todo";
    if !bare && !(clean_lower.starts_with("todo ") || clean_lower.starts_with("!todo ")) {
        return None;
    }

    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let tz = settings.get_or_default(&acct).await.tz();

    if bare {
        let items = TodoRepository::list_all_user(pool, owner_id, None, PAGE_SIZE, 0)
            .await
            .ok()
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
        handle_add(
            pool,
            broadcaster,
            room_id,
            owner_id,
            sender_name,
            args_body,
            quoted,
        )
        .await
    } else if args_lower.starts_with("done") {
        handle_done(pool, broadcaster, owner_id, args_body).await
    } else if args_lower.starts_with("edit") {
        handle_edit(pool, broadcaster, owner_id, args_body).await
    } else if args_lower.starts_with("priority") {
        handle_priority(pool, broadcaster, owner_id, args_body).await
    } else if args_lower.starts_with("due") {
        handle_due(pool, broadcaster, room_id, owner_id, args_body, &tz).await
    } else if args_lower.starts_with("clear") {
        handle_clear(pool, broadcaster, room_id, owner_id, sender_name, args_body).await
    } else if args_lower.starts_with("delete") || args_lower.starts_with("rm") {
        handle_delete(pool, broadcaster, owner_id, args_body).await
    } else if args_lower.starts_with("room") {
        handle_list_room(pool, room_id, args_body, &tz).await
    } else if args_lower.starts_with("list")
        || args_lower.starts_with("my")
        || args_lower.starts_with("all")
        || args_lower.starts_with("global")
        || args_lower.is_empty()
    {
        handle_list_user(pool, owner_id, sender_name, args_body, &tz).await
    } else {
        Some("⚠️ Unknown todo command. Available: <code>add</code>, <code>list</code>, \
              <code>all</code>, <code>room</code>, <code>done</code>, <code>edit</code>, <code>priority</code>, \
              <code>due</code>, <code>clear</code>, <code>delete</code>".to_string())
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
            sender_mxid: "@alice:1".to_string(),
            owner_id: crate::domain::accounts::AccountId::new("@alice:1"),
            sender_name: "alice".to_string(),
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
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let res = handle_todo_command(&pool, &settings, None, &ctx("!todo add [high] Buy apples"))
            .await
            .unwrap();
        assert!(res.contains("Added Task"));
        assert!(res.contains("Buy apples"));
        assert!(res.contains("HIGH"));

        let res_done = handle_todo_command(&pool, &settings, None, &ctx("!todo done 1"))
            .await
            .unwrap();
        assert!(res_done.contains("Marked Task"));
        assert!(!res_done.contains("not found"));

        let res_empty_add = handle_todo_command(&pool, &settings, None, &ctx("!todo add"))
            .await
            .unwrap();
        assert!(res_empty_add.contains("Usage:"));
    }

    #[tokio::test]
    async fn test_handle_todo_edit_and_priority() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        handle_todo_command(&pool, &settings, None, &ctx("!todo add Test task")).await;

        let res_edit =
            handle_todo_command(&pool, &settings, None, &ctx("!todo edit 1 Updated task"))
                .await
                .unwrap();
        assert!(res_edit.contains("Updated Task"));

        let res_prio = handle_todo_command(&pool, &settings, None, &ctx("!todo priority 1 high"))
            .await
            .unwrap();
        assert!(res_prio.contains("Set Task"));
        assert!(res_prio.contains("HIGH"));
    }

    #[tokio::test]
    async fn test_handle_todo_list_and_clear() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        handle_todo_command(&pool, &settings, None, &ctx("!todo add Task A")).await;
        handle_todo_command(&pool, &settings, None, &ctx("!todo done 1")).await;

        let res_list = handle_todo_command(&pool, &settings, None, &ctx("!todo list"))
            .await
            .unwrap();
        assert!(res_list.contains("Your Tasks"));

        let res_clear = handle_todo_command(&pool, &settings, None, &ctx("!todo clear"))
            .await
            .unwrap();
        assert!(res_clear.contains("Cleared"));
    }

    #[tokio::test]
    async fn test_handle_unknown_command() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let res = handle_todo_command(&pool, &settings, None, &ctx("!todo foobar"))
            .await
            .unwrap();
        assert!(res.contains("Unknown todo command"));
    }

    #[tokio::test]
    async fn test_handle_todo_input_too_long() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let long_text = "x".repeat(501);
        let cmd = format!("!todo add {}", long_text);
        let res = handle_todo_command(&pool, &settings, None, &ctx(&cmd))
            .await
            .unwrap();
        assert!(res.contains("too long"));
    }

    #[tokio::test]
    async fn test_handle_todo_delete() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        handle_todo_command(&pool, &settings, None, &ctx("!todo add Task to remove")).await;
        let res_del = handle_todo_command(&pool, &settings, None, &ctx("!todo delete 1"))
            .await
            .unwrap();
        assert!(res_del.contains("Deleted Task"));

        // Alice tries to delete non-existent task
        let res_notfound = handle_todo_command(&pool, &settings, None, &ctx("!todo rm 1"))
            .await
            .unwrap();
        assert!(res_notfound.contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_todo_add_with_quoted_message() {
        let pool = setup_test_db().await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );
        let quoted = crate::interface::channels::matrix::quote::QuotedMessage {
            author: Some("bob".to_string()),
            body: "Remember to review PR 42".to_string(),
            event_id: None,
        };
        let mut quoted_ctx = ctx("!todo add");
        quoted_ctx.quoted = Some(quoted);

        // !todo add without description, should take quoted body
        let res = handle_todo_command(&pool, &settings, None, &quoted_ctx)
            .await
            .unwrap();

        assert!(res.contains("Added Task"));
        assert!(res.contains("Remember to review PR 42"));

        let res_list = handle_todo_command(&pool, &settings, None, &ctx("!todo list"))
            .await
            .unwrap();
        assert!(res_list.contains("Remember to review PR 42"));
    }
}
