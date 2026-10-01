//! Matrix bot module for Bookmarks.

use async_trait::async_trait;
use reqwest::Client;

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::bookmarks::BookmarkQuery;
use crate::infrastructure::web::fetch_and_extract_metadata;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape;

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
        services: &AppServices,
        _db: &Database,
    ) -> Option<String> {
        match ctx.command.as_str() {
            "save" => Some(handle_save_command(ctx, services, Some(&self.http_client)).await),
            "bookmarks" | "reading" => Some(handle_list_command(ctx, services).await),
            "bookmark" => {
                Some(handle_bookmark_subcommands(ctx, services, Some(&self.http_client)).await)
            }
            _ => None,
        }
    }
}

pub async fn handle_save_command(
    ctx: &CommandContext,
    services: &AppServices,
    http_client: Option<&Client>,
) -> String {
    let args = ctx.raw_args.trim();
    if args.is_empty() {
        return "❌ <b>Usage:</b> <code>!save &lt;url&gt; [tags,comma,separated]</code>"
            .to_string();
    }

    let mut parts = args.split_whitespace();
    let url_candidate = parts.next().unwrap_or_default();

    if !url_candidate.starts_with("http://") && !url_candidate.starts_with("https://") {
        return "❌ <b>Invalid URL:</b> URL must start with <code>http://</code> or <code>https://</code>".to_string();
    }

    if url_candidate.len() > 2048 {
        return "❌ <b>Invalid URL:</b> URL exceeds maximum length of 2048 characters.".to_string();
    }

    let tags = parts.collect::<Vec<_>>().join(" ");
    let clean_tags: Vec<String> = tags
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let joined_tags = clean_tags.join(",");
    if joined_tags.len() > 500 {
        return "❌ <b>Invalid tags:</b> Tags exceed maximum length of 500 characters.".to_string();
    }

    let meta = fetch_and_extract_metadata(http_client, url_candidate, None).await;

    let bookmark_res = services
        .bookmarks
        .create(&ctx.owner_id, url_candidate, None, Some(&clean_tags), meta)
        .await;

    match bookmark_res {
        Ok(b) => {
            let title_esc = html_escape(&b.title);
            let domain_esc = html_escape(&b.domain);
            let url_esc = html_escape(&b.url);
            let tags_display = if b.tags.is_empty() {
                String::new()
            } else {
                format!(" | 🏷️ <i>{}</i>", html_escape(&b.tags))
            };

            format!(
                "🔖 <b>Saved Bookmark #{}:</b> <a href=\"{}\">{}</a> (<code>{}</code>)<br/>\
                 ⏱️ ~{} min read{}",
                b.id, url_esc, title_esc, domain_esc, b.reading_time_minutes, tags_display
            )
        }
        Err(e) => format!(
            "❌ <b>Failed to save bookmark:</b> {}",
            html_escape(&e.to_string())
        ),
    }
}

pub async fn handle_list_command(ctx: &CommandContext, services: &AppServices) -> String {
    let tag = if ctx.raw_args.trim().is_empty() {
        None
    } else {
        Some(ctx.raw_args.trim().to_string())
    };

    let query = BookmarkQuery {
        archived: Some(false),
        tag,
        limit: Some(5),
        offset: Some(0),
        ..Default::default()
    };

    let items = match services.bookmarks.list(&ctx.owner_id, &query).await {
        Ok(res) => res,
        Err(e) => {
            return format!(
                "❌ <b>Error listing bookmarks:</b> {}",
                html_escape(&e.to_string())
            )
        }
    };

    if items.is_empty() {
        return "📚 <b>No unread bookmarks found.</b> Save one with <code>!save &lt;url&gt;</code>.".to_string();
    }

    let mut out = String::from("📚 <b>Your Reading List:</b><br/>");
    for b in items {
        let fav_star = if b.is_favorite { "⭐ " } else { "" };
        let title_esc = html_escape(&b.title);
        let url_esc = html_escape(&b.url);
        let domain_esc = html_escape(&b.domain);

        out.push_str(&format!(
            "• {}{}: <a href=\"{}\">{}</a> (<code>{}</code>, ~{} min)<br/>",
            fav_star, b.id, url_esc, title_esc, domain_esc, b.reading_time_minutes
        ));
    }

    out.push_str("<br/><i>Tip: Use <code>!bookmark fav &lt;id&gt;</code> or <code>!bookmark archive &lt;id&gt;</code>.</i>");
    out
}

pub async fn handle_bookmark_subcommands(
    ctx: &CommandContext,
    services: &AppServices,
    http_client: Option<&Client>,
) -> String {
    let raw = ctx.raw_args.trim();
    let mut parts = raw.split_whitespace();
    let sub = parts.next().unwrap_or("");

    match sub {
        "" | "list" => handle_list_command(ctx, services).await,
        "fav" | "star" | "favorite" => {
            let id = parts.next().and_then(|s| s.parse::<i64>().ok());
            let Some(id) = id else {
                return "❌ <b>Usage:</b> <code>!bookmark fav &lt;id&gt;</code>".to_string();
            };

            match services.bookmarks.toggle_favorite(&ctx.owner_id, id).await {
                Ok(b) => {
                    let state = if b.is_favorite {
                        "Starred ⭐"
                    } else {
                        "Unstarred"
                    };
                    format!(
                        "{} bookmark <b>#{}</b>: {}",
                        state,
                        b.id,
                        html_escape(&b.title)
                    )
                }
                Err(_) => format!("❌ Bookmark #{} not found.", id),
            }
        }
        "archive" | "read" | "done" => {
            let id = parts.next().and_then(|s| s.parse::<i64>().ok());
            let Some(id) = id else {
                return "❌ <b>Usage:</b> <code>!bookmark archive &lt;id&gt;</code>".to_string();
            };

            match services.bookmarks.toggle_archived(&ctx.owner_id, id).await {
                Ok(b) => {
                    let state = if b.is_archived {
                        "Archived 📦"
                    } else {
                        "Restored to unread 📥"
                    };
                    format!(
                        "{} bookmark <b>#{}</b>: {}",
                        state,
                        b.id,
                        html_escape(&b.title)
                    )
                }
                Err(_) => format!("❌ Bookmark #{} not found.", id),
            }
        }
        "delete" | "rm" => {
            let id = parts.next().and_then(|s| s.parse::<i64>().ok());
            let Some(id) = id else {
                return "❌ <b>Usage:</b> <code>!bookmark rm &lt;id&gt;</code>".to_string();
            };

            match services.bookmarks.delete(&ctx.owner_id, id).await {
                Ok(()) => format!("🗑️ Deleted bookmark <b>#{}</b>", id),
                Err(_) => format!("❌ Bookmark #{} not found.", id),
            }
        }
        url if url.starts_with("http://") || url.starts_with("https://") => {
            handle_save_command(ctx, services, http_client).await
        }
        _ => format_help(),
    }
}

pub fn format_help() -> String {
    "🔖 <b>Bookmarks &amp; Reading List Commands:</b><br/>\
     • <code>!save &lt;url&gt; [tag1,tag2]</code> — Save web page &amp; extract reader view<br/>\
     • <code>!bookmarks</code> / <code>!reading</code> — List unread bookmarks<br/>\
     • <code>!bookmark fav &lt;id&gt;</code> — Toggle favorite star<br/>\
     • <code>!bookmark archive &lt;id&gt;</code> — Mark as read / archive<br/>\
     • <code>!bookmark rm &lt;id&gt;</code> — Delete bookmark"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> (AppServices, crate::domain::accounts::AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AppServices::new(&pool)
            .identity
            .provision_matrix_for_test("@alice:mbm.one")
            .await;
        let services = AppServices::new(&pool);
        (services, owner)
    }

    #[tokio::test]
    async fn test_handle_save_and_list_flow() {
        let (services, owner) = setup_db().await;

        let ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner,
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "save".to_string(),
            raw_args: "https://example.com/rust-tips rust,coding".to_string(),
            full_body: "!save https://example.com/rust-tips rust,coding".to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        };

        let reply = handle_save_command(&ctx, &services, None).await;
        assert!(reply.contains("Saved Bookmark #1"));
        assert!(reply.contains("example.com"));

        let list_ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner,
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "bookmarks".to_string(),
            raw_args: String::new(),
            full_body: "!bookmarks".to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        };

        let list_reply = handle_list_command(&list_ctx, &services).await;
        assert!(list_reply.contains("Your Reading List"));
        assert!(list_reply.contains("example.com"));

        // Toggle fav
        let fav_ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner,
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "bookmark".to_string(),
            raw_args: "fav 1".to_string(),
            full_body: "!bookmark fav 1".to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        };
        let fav_reply = handle_bookmark_subcommands(&fav_ctx, &services, None).await;
        assert!(fav_reply.contains("Starred ⭐"));

        // Toggle archive
        let arch_ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner,
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "bookmark".to_string(),
            raw_args: "archive 1".to_string(),
            full_body: "!bookmark archive 1".to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        };
        let arch_reply = handle_bookmark_subcommands(&arch_ctx, &services, None).await;
        assert!(arch_reply.contains("Archived 📦"));
    }
}
