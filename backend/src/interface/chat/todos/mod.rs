use async_trait::async_trait;

use crate::interface::channels::message::Reply;

use crate::db::Database;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod mutate;
pub mod view;

pub struct TodoModule;

impl Default for TodoModule {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for TodoModule {
    fn name(&self) -> &str {
        "To-Do"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "todo add".to_string(),
                usage: "!todo add [[high|low]] <task>".to_string(),
                description: "Add a new task (supports inline priority like [high])".to_string(),
            },
            CommandHelp {
                name: "todo list".to_string(),
                usage: "!todo [list|room] [page]".to_string(),
                description: "View personal or room tasks with pagination".to_string(),
            },
            CommandHelp {
                name: "todo done".to_string(),
                usage: "!todo done <id>".to_string(),
                description: "Mark a task #id as completed".to_string(),
            },
            CommandHelp {
                name: "todo priority".to_string(),
                usage: "!todo priority <id> <low|medium|high>".to_string(),
                description: "Change task priority".to_string(),
            },
            CommandHelp {
                name: "todo due".to_string(),
                usage: "!todo due <id> <time> [remind <lead>]".to_string(),
                description: "Set a task due date, optionally with a reminder beforehand \
                              (e.g. !todo due 1 2d remind 1h). Use 'none' to clear it."
                    .to_string(),
            },
            CommandHelp {
                name: "todo edit".to_string(),
                usage: "!todo edit <id> <new text>".to_string(),
                description: "Edit task description".to_string(),
            },
            CommandHelp {
                name: "todo clear".to_string(),
                usage: "!todo clear [room]".to_string(),
                description: "Remove completed tasks".to_string(),
            },
            CommandHelp {
                name: "todo delete".to_string(),
                usage: "!todo delete <id> / !todo rm <id>".to_string(),
                description: "Delete a task".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        handler::handle_todo_command(&services.todos, &services.settings, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::chat::CommandContext;

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            conversation: crate::domain::channels::ConversationAddress::Matrix(
                "!room:a".to_string(),
            ),
            owner_id: crate::domain::accounts::AccountId::from_seed("@alice:a"),
            sender: crate::interface::chat::test_support::sender("@tester:x", "alice"),
            command: body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('!')
                .to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            progress: None,
            outbound: None,
        }
    }

    async fn setup_services() -> (Database, crate::application::AppServices) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        let services = crate::application::AppServices::new(&pool);
        (Database { pool }, services)
    }

    #[test]
    fn test_advertises_its_commands() {
        let module = TodoModule::new();
        assert_eq!(module.name(), "To-Do");

        let commands = module.commands();
        assert!(commands.iter().any(|c| c.name == "todo due"));
        let due = commands.iter().find(|c| c.name == "todo due").unwrap();
        assert!(
            due.usage.contains("remind"),
            "usage should mention the reminder: {}",
            due.usage
        );
        assert!(commands
            .iter()
            .all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = TodoModule::new();
        let (db, services) = setup_services().await;

        assert!(module
            .handle_command(&ctx("!todo list"), &services, &db)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!expense add 1 EUR x"), &services, &db)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!remind me later"), &services, &db)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let module = TodoModule::new();
        let (db, services) = setup_services().await;

        module
            .handle_command(&ctx("!todo add <img src=x onerror=1>"), &services, &db)
            .await
            .unwrap();
        let listed = module
            .handle_command(&ctx("!todo list"), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();

        assert!(
            !listed.contains("<img"),
            "raw markup must not survive: {listed}"
        );
        assert!(listed.contains("&lt;img"));
    }

    #[tokio::test]
    async fn test_due_dates_render_in_the_senders_saved_timezone() {
        use crate::domain::settings::UserSettings;

        let module = TodoModule::new();
        let (db, services) = setup_services().await;

        module
            .handle_command(&ctx("!todo add ship it"), &services, &db)
            .await
            .unwrap();

        let utc_reply = module
            .handle_command(&ctx("!todo due 1 2d"), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(
            utc_reply.contains("UTC"),
            "default must render as UTC: {utc_reply}"
        );

        services
            .settings
            .save(
                &crate::domain::accounts::AccountId::from_seed("@alice:a"),
                UserSettings {
                    timezone: "Asia/Shanghai".to_string(),
                    currency: "CNY".to_string(),
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();

        let listed = module
            .handle_command(&ctx("!todo list"), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(
            listed.contains("CST"),
            "should render in Asia/Shanghai: {listed}"
        );
        assert!(
            !listed.contains("UTC"),
            "should no longer claim UTC: {listed}"
        );
    }

    #[tokio::test]
    async fn test_the_module_writes_events_to_the_outbox() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        let services = crate::application::AppServices::new(&pool);
        let db = Database { pool: pool.clone() };

        let reply = TodoModule::new()
            .handle_command(&ctx("!todo add Buy milk"), &services, &db)
            .await
            .expect("the add must be handled")
            .html_body()
            .to_owned();
        assert!(reply.contains("Added Task"), "unexpected reply: {reply}");

        let owner = crate::domain::accounts::AccountId::from_seed("@alice:a");
        assert_eq!(
            crate::infrastructure::postgres::repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["TodoCreated"],
            "dispatching through the module must write the creation to the outbox"
        );
    }

    #[tokio::test]
    async fn test_the_list_reply_carries_one_done_action_per_open_task() {
        let module = TodoModule::new();
        let (db, services) = setup_services().await;
        for text in ["one", "two"] {
            module
                .handle_command(&ctx(&format!("!todo add {text}")), &services, &db)
                .await
                .unwrap();
        }
        module
            .handle_command(&ctx("!todo done 1"), &services, &db)
            .await
            .unwrap();

        for body in ["!todo", "!todo list"] {
            let reply = module
                .handle_command(&ctx(body), &services, &db)
                .await
                .unwrap();
            let commands: Vec<_> = reply
                .content
                .actions
                .iter()
                .map(|a| a.command.as_str())
                .collect();
            assert_eq!(commands, vec!["todo done 2"], "for {body}");
        }
        let added = module
            .handle_command(&ctx("!todo add three"), &services, &db)
            .await
            .unwrap();
        assert!(added.content.actions.is_empty(), "only lists carry buttons");
    }
}
