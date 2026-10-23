//! Command handler and dispatcher for todo chat commands.

use crate::application::settings::SettingsService;
use crate::application::todos::TodoService;
use crate::interface::channels::message::Reply;
use crate::interface::chat::CommandContext;
use crate::utils::html_escape;

use super::mutate::{
    handle_add, handle_clear, handle_delete, handle_done, handle_due, handle_edit, handle_priority,
};
use super::steps::{handle_show, handle_step};
use super::tags::{handle_repeat, handle_tag, handle_tags};
use super::view::{handle_list_room, handle_list_user, page_reply, PAGE_SIZE};

pub async fn handle_todo_command(
    todos: &TodoService,
    settings: &SettingsService,
    ctx: &CommandContext,
) -> Option<Reply> {
    let room_key = ctx.room_key();
    let room_id = room_key.as_str();
    let owner_id = &ctx.owner_id;
    let sender_name = ctx.sender_name();
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
            .list(owner_id, None, None, None, PAGE_SIZE, 0)
            .await
            .unwrap_or_default();
        let title = format!("Your Tasks ({})", html_escape(sender_name));
        return Some(page_reply(todos, owner_id, &title, &items, 1, &tz).await);
    }

    let args_body = if clean_lower.starts_with("!todo ") {
        clean_body[6..].trim()
    } else {
        clean_body[5..].trim()
    };
    let args_lower = args_body.to_lowercase();

    // Matched on the whole first word, and `tags` ahead of `tag`, so neither
    // is swallowed by a looser prefix test below.
    let first_word = args_lower.split_whitespace().next().unwrap_or("");

    if first_word == "tags" {
        handle_tags(todos, owner_id).await.map(Reply::from)
    } else if first_word == "tag" {
        handle_tag(todos, owner_id, args_body)
            .await
            .map(Reply::from)
    } else if first_word == "repeat" {
        handle_repeat(todos, owner_id, args_body)
            .await
            .map(Reply::from)
    } else if first_word == "show" {
        handle_show(todos, owner_id, args_body, &tz)
            .await
            .map(Reply::from)
    } else if first_word == "step" {
        handle_step(todos, owner_id, args_body, &tz)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("add") {
        handle_add(
            todos,
            room_id,
            owner_id,
            sender_name,
            args_body,
            quoted,
            &tz,
        )
        .await
        .map(Reply::from)
    } else if args_lower.starts_with("done") {
        handle_done(todos, owner_id, args_body, &tz)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("edit") {
        handle_edit(todos, owner_id, args_body)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("priority") {
        handle_priority(todos, owner_id, args_body)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("due") {
        handle_due(todos, room_id, owner_id, args_body, &tz)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("clear") {
        handle_clear(todos, room_id, owner_id, sender_name, args_body)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("delete") || args_lower.starts_with("rm") {
        handle_delete(todos, owner_id, args_body)
            .await
            .map(Reply::from)
    } else if args_lower.starts_with("room") {
        handle_list_room(todos, owner_id, room_id, args_body, &tz).await
    } else if args_lower.starts_with("list")
        || args_lower.starts_with("my")
        || args_lower.starts_with("all")
        || args_lower.starts_with("global")
        || args_lower.is_empty()
    {
        handle_list_user(todos, owner_id, sender_name, args_body, &tz).await
    } else {
        Some(Reply::html("⚠️ Unknown todo command. Available: <code>add</code>, <code>list</code>, \
              <code>all</code>, <code>room</code>, <code>done</code>, <code>edit</code>, <code>priority</code>, \
              <code>due</code>, <code>clear</code>, <code>delete</code>, <code>tags</code>, \
              <code>tag</code>, <code>repeat</code>, <code>show</code>, <code>step</code>"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::reminders::ReminderService;
    use crate::domain::accounts::AccountId;

    async fn setup_services() -> (TodoService, SettingsService) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@bob:1"]).await;
        let settings = SettingsService::new(pool.clone());
        let reminders = ReminderService::new(pool.clone());
        let todos = TodoService::new(pool, reminders);
        (todos, settings)
    }

    fn html(reply: Option<Reply>) -> String {
        reply.unwrap().html_body().to_owned()
    }

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            conversation: crate::domain::channels::ConversationAddress::Matrix(
                "!room:1".to_string(),
            ),
            owner_id: AccountId::from_seed("@bob:1"),
            sender: crate::interface::chat::test_support::sender("@bob:1", "bob"),
            command: "todo".to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            progress: None,
            outbound: None,
        }
    }

    #[tokio::test]
    async fn test_handle_todo_add_and_done() {
        let (todos, settings) = setup_services().await;

        let res_add =
            html(handle_todo_command(&todos, &settings, &ctx("!todo add Buy groceries")).await);
        assert!(res_add.contains("Added Task <b>#1</b>"));
        assert!(res_add.contains("Buy groceries"));

        let res_list = html(handle_todo_command(&todos, &settings, &ctx("!todo list")).await);
        assert!(res_list.contains("Buy groceries"));

        let res_done = html(handle_todo_command(&todos, &settings, &ctx("!todo done 1")).await);
        assert!(res_done.contains("Marked Task <b>#1</b>"));
        assert!(res_done.contains("as completed"));
    }

    /// Given the new commands, When dispatched, Then `tags` is not taken for
    /// `tag`, and tags, repeat, steps and the filtered list all round-trip.
    #[tokio::test]
    async fn tags_repeat_and_steps_dispatch_and_round_trip() {
        let (todos, settings) = setup_services().await;
        let run = |body: &'static str| {
            let (t, s) = (todos.clone(), settings.clone());
            async move { html(handle_todo_command(&t, &s, &ctx(body)).await) }
        };

        let added = run("!todo add Pay <rent> #Home every month due 3d").await;
        assert!(added.contains("Added Task <b>#1</b>"), "{added}");
        assert!(
            added.contains("↻ monthly") && added.contains("#home"),
            "{added}"
        );
        assert!(added.contains("Pay &lt;rent&gt;"), "{added}");

        let needs_due = run("!todo add Pay rent every month").await;
        assert!(needs_due.contains("needs a due date"), "{needs_due}");

        assert!(run("!todo tag 1 +work -home").await.contains("#work"));
        let tags = run("!todo tags").await;
        assert!(tags.contains("#work") && !tags.contains("#home"), "{tags}");
        assert!(run("!todo list #work").await.contains("Pay &lt;rent&gt;"));
        assert!(!run("!todo list #home").await.contains("Pay"));
        assert!(run("!todo repeat 1 weekly")
            .await
            .contains("repeats weekly"));
        assert!(run("!todo tag 99 +x").await.contains("not found"));

        assert!(run("!todo step 1 add buy <milk>")
            .await
            .contains("buy &lt;milk&gt;"));
        run("!todo step 1 add call").await;
        let done = run("!todo step 1 done 2").await;
        assert!(
            done.contains("Steps 1/2") && done.contains("☑ 2."),
            "{done}"
        );
        assert!(run("!todo list").await.contains("· 1/2"));
        assert!(run("!todo step 1 rm 9").await.contains("no step"));
        assert!(run("!todo step 1 rm 1").await.contains("Steps 1/1"));
        assert!(run("!todo show 1").await.contains("☑ 1."));
        assert!(run("!todo show 42").await.contains("not found"));
    }

    #[tokio::test]
    async fn test_handle_todo_edit_and_priority() {
        let (todos, settings) = setup_services().await;

        handle_todo_command(&todos, &settings, &ctx("!todo add Initial task")).await;

        let res_edit = html(
            handle_todo_command(&todos, &settings, &ctx("!todo edit 1 Updated task text")).await,
        );
        assert!(res_edit.contains("Updated Task <b>#1</b> text to: <i>\"Updated task text\"</i>"));

        let res_prio =
            html(handle_todo_command(&todos, &settings, &ctx("!todo priority 1 high")).await);
        assert!(res_prio.contains("Set Task <b>#1</b> priority to"));
        assert!(res_prio.contains("HIGH"));
    }

    #[tokio::test]
    async fn test_handle_todo_delete() {
        let (todos, settings) = setup_services().await;

        handle_todo_command(&todos, &settings, &ctx("!todo add Delete me")).await;

        let res_del = html(handle_todo_command(&todos, &settings, &ctx("!todo delete 1")).await);
        assert!(res_del.contains("Deleted Task <b>#1</b>: <i>\"Delete me\"</i>"));

        let res_del_again =
            html(handle_todo_command(&todos, &settings, &ctx("!todo delete 1")).await);
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

        let res = html(handle_todo_command(&todos, &settings, &quoted_ctx).await);
        assert!(res.contains("Added Task <b>#1</b>"));
        assert!(res.contains("Remember to review PR #42"));
    }
}
