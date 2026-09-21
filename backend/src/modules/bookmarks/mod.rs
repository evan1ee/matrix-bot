use async_trait::async_trait;
use reqwest::Client;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod metadata;
pub mod models;
pub mod repository;

pub struct BookmarksModule {
    http_client: Client,
}

impl BookmarksModule {
    pub fn new() -> Self {
        Self {
            http_client: Client::new(),
        }
    }
}

impl Default for BookmarksModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for BookmarksModule {
    fn name(&self) -> &str {
        "Bookmarks"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "save".to_string(),
                usage: "!save <url> [tag1,tag2]".to_string(),
                description: "Save web page with OpenGraph metadata and reader view".to_string(),
            },
            CommandHelp {
                name: "bookmarks".to_string(),
                usage: "!bookmarks [tag]".to_string(),
                description: "List unread bookmarks in reading list".to_string(),
            },
            CommandHelp {
                name: "bookmark fav".to_string(),
                usage: "!bookmark fav <id>".to_string(),
                description: "Toggle star/favorite on a bookmark".to_string(),
            },
            CommandHelp {
                name: "bookmark archive".to_string(),
                usage: "!bookmark archive <id>".to_string(),
                description: "Archive a bookmark or mark as read".to_string(),
            },
            CommandHelp {
                name: "bookmark rm".to_string(),
                usage: "!bookmark rm <id>".to_string(),
                description: "Delete a bookmark".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        match ctx.command.as_str() {
            "save" => Some(
                handler::handle_save_command(ctx, &db.pool, Some(&self.http_client), broadcaster)
                    .await,
            ),
            "bookmarks" | "reading" => Some(handler::handle_list_command(ctx, &db.pool).await),
            "bookmark" => Some(
                handler::handle_bookmark_subcommands(
                    ctx,
                    &db.pool,
                    Some(&self.http_client),
                    broadcaster,
                )
                .await,
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advertises_its_commands() {
        let module = BookmarksModule::new();
        assert_eq!(module.name(), "Bookmarks");
        let cmds = module.commands();
        assert!(cmds.iter().any(|c| c.name == "save"));
        assert!(cmds.iter().any(|c| c.name == "bookmarks"));
    }
}
