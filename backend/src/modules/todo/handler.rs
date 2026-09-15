use sqlx::SqlitePool;

use crate::modules::settings::SettingsRepository;
use crate::modules::todo::mutate::{
    handle_add, handle_clear, handle_delete, handle_done, handle_due, handle_edit, handle_priority,
};
use crate::modules::todo::repository::TodoRepository;
use crate::modules::todo::view::{format_todo_list, handle_list_room, handle_list_user, PAGE_SIZE};
use crate::utils::html_escape;

pub use crate::modules::todo::mutate::{MAX_ROOM_TASKS, MAX_TASK_TEXT_LEN};

pub async fn handle_todo_command(
    pool: &SqlitePool,
    room_id: &str,
    owner_id: &str,
    sender_name: &str,
    clean_body: &str,
    quoted: Option<&crate::bot::quote::QuotedMessage>,
) -> Option<String> {
    let clean_lower = clean_body.to_lowercase();

    // Bare "todo" or "!todo" without a space still claims the message.
    let bare = clean_lower == "todo" || clean_lower == "!todo";
    if !bare && !(clean_lower.starts_with("todo ") || clean_lower.starts_with("!todo ")) {
        return None;
    }

    // Read after the command is claimed, so an unrelated message never costs a
    // settings query.
    let tz = SettingsRepository::get_or_default(pool, owner_id)
        .await
        .tz();

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
        handle_add(pool, room_id, owner_id, sender_name, args_body, quoted).await
    } else if args_lower.starts_with("done") {
        handle_done(pool, room_id, args_body).await
    } else if args_lower.starts_with("edit") {
        handle_edit(pool, args_body).await
    } else if args_lower.starts_with("priority") {
        handle_priority(pool, args_body).await
    } else if args_lower.starts_with("due") {
        handle_due(pool, room_id, args_body, &tz).await
    } else if args_lower.starts_with("clear") {
        handle_clear(pool, room_id, owner_id, sender_name, args_body).await
    } else if args_lower.starts_with("delete") || args_lower.starts_with("rm") {
        handle_delete(pool, owner_id, args_body).await
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
    use crate::db::schema::run_migrations;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_handle_todo_add_and_done() {
        let pool = setup_test_db().await;

        let res = handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo add [high] Buy apples",
            None,
        )
        .await
        .unwrap();
        assert!(res.contains("Added Task"));
        assert!(res.contains("Buy apples"));
        assert!(res.contains("HIGH"));

        let res_done =
            handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo done 1", None)
                .await
                .unwrap();
        assert!(res_done.contains("Marked Task"));
        assert!(!res_done.contains("not found"));

        let res_empty_add =
            handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo add", None)
                .await
                .unwrap();
        assert!(res_empty_add.contains("Usage:"));
    }

    #[tokio::test]
    async fn test_handle_todo_edit_and_priority() {
        let pool = setup_test_db().await;

        handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo add Test task",
            None,
        )
        .await;

        let res_edit = handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo edit 1 Updated task",
            None,
        )
        .await
        .unwrap();
        assert!(res_edit.contains("Updated Task"));

        let res_prio = handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo priority 1 high",
            None,
        )
        .await
        .unwrap();
        assert!(res_prio.contains("Set Task"));
        assert!(res_prio.contains("HIGH"));
    }

    #[tokio::test]
    async fn test_handle_todo_list_and_clear() {
        let pool = setup_test_db().await;

        handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo add Task A",
            None,
        )
        .await;
        handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo done 1", None).await;

        let res_list =
            handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo list", None)
                .await
                .unwrap();
        assert!(res_list.contains("Your Tasks"));

        let res_clear =
            handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo clear", None)
                .await
                .unwrap();
        assert!(res_clear.contains("Cleared"));
    }

    #[tokio::test]
    async fn test_handle_unknown_command() {
        let pool = setup_test_db().await;

        let res = handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo foobar", None)
            .await
            .unwrap();
        assert!(res.contains("Unknown todo command"));
    }

    #[tokio::test]
    async fn test_handle_todo_input_too_long() {
        let pool = setup_test_db().await;

        let long_text = "x".repeat(501);
        let cmd = format!("!todo add {}", long_text);
        let res = handle_todo_command(&pool, "!room:1", "@alice:1", "alice", &cmd, None)
            .await
            .unwrap();
        assert!(res.contains("too long"));
    }

    #[tokio::test]
    async fn test_handle_todo_delete() {
        let pool = setup_test_db().await;

        handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo add Task to remove",
            None,
        )
        .await;
        let res_del = handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo delete 1",
            None,
        )
        .await
        .unwrap();
        assert!(res_del.contains("Deleted Task"));

        // Alice tries to delete non-existent task
        let res_notfound =
            handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo rm 1", None)
                .await
                .unwrap();
        assert!(res_notfound.contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_todo_add_with_quoted_message() {
        let pool = setup_test_db().await;
        let quoted = crate::bot::quote::QuotedMessage {
            author: Some("bob".to_string()),
            body: "Remember to review PR 42".to_string(),
            event_id: None,
        };

        // !todo add without description, should take quoted body
        let res = handle_todo_command(
            &pool,
            "!room:1",
            "@alice:1",
            "alice",
            "!todo add",
            Some(&quoted),
        )
        .await
        .unwrap();

        assert!(res.contains("Added Task"));
        assert!(res.contains("Remember to review PR 42"));

        let res_list =
            handle_todo_command(&pool, "!room:1", "@alice:1", "alice", "!todo list", None)
                .await
                .unwrap();
        assert!(res_list.contains("Remember to review PR 42"));
    }
}
