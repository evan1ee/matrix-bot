use reqwest::Client;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::bookmarks::metadata::fetch_and_extract_metadata;
use crate::modules::bookmarks::models::BookmarkQuery;
use crate::modules::bookmarks::repository::BookmarksRepository;
use crate::modules::CommandContext;
use crate::utils::html::html_escape;

pub async fn handle_save_command(
    ctx: &CommandContext,
    pool: &SqlitePool,
    http_client: Option<&Client>,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let args = ctx.raw_args.trim();
    if args.is_empty() {
        return "❌ <b>Usage:</b> <code>!save &lt;url&gt; [tags,comma,separated]</code>".to_string();
    }

    let mut parts = args.split_whitespace();
    let url_candidate = parts.next().unwrap_or_default();

    if !url_candidate.starts_with("http://") && !url_candidate.starts_with("https://") {
        return "❌ <b>Invalid URL:</b> URL must start with <code>http://</code> or <code>https://</code>".to_string();
    }

    let tags = parts.collect::<Vec<_>>().join(" ");
    let clean_tags = tags
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(",");

    let meta = fetch_and_extract_metadata(http_client, url_candidate, None).await;

    let bookmark_res = BookmarksRepository::create(
        pool,
        &ctx.owner_id,
        url_candidate,
        &meta.title,
        meta.description.as_deref(),
        meta.author.as_deref(),
        meta.image_url.as_deref(),
        &meta.domain,
        meta.reader_content.as_deref(),
        meta.reading_time_minutes,
        &clean_tags,
    )
    .await;

    match bookmark_res {
        Ok(b) => {
            if let Some(bc) = broadcaster {
                bc.send(SystemEvent::BookmarkCreated(b.clone()));
            }

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
        Err(e) => format!("❌ <b>Failed to save bookmark:</b> {}", html_escape(&e.to_string())),
    }
}

pub async fn handle_list_command(
    ctx: &CommandContext,
    pool: &SqlitePool,
) -> String {
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

    let items = match BookmarksRepository::list(pool, &ctx.owner_id, &query).await {
        Ok(res) => res,
        Err(e) => return format!("❌ <b>Error listing bookmarks:</b> {}", html_escape(&e.to_string())),
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
    pool: &SqlitePool,
    http_client: Option<&Client>,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let raw = ctx.raw_args.trim();
    let mut parts = raw.split_whitespace();
    let sub = parts.next().unwrap_or("");

    match sub {
        "fav" | "star" | "favorite" => {
            let id = parts.next().and_then(|s| s.parse::<i64>().ok());
            let Some(id) = id else {
                return "❌ <b>Usage:</b> <code>!bookmark fav &lt;id&gt;</code>".to_string();
            };

            match BookmarksRepository::toggle_favorite(pool, id, &ctx.owner_id).await {
                Ok(b) => {
                    if let Some(bc) = broadcaster {
                        bc.send(SystemEvent::BookmarkUpdated(b.clone()));
                    }
                    let state = if b.is_favorite { "Starred ⭐" } else { "Unstarred" };
                    format!("{} bookmark <b>#{}</b>: {}", state, b.id, html_escape(&b.title))
                }
                Err(_) => format!("❌ Bookmark #{} not found.", id),
            }
        }
        "archive" | "read" | "done" => {
            let id = parts.next().and_then(|s| s.parse::<i64>().ok());
            let Some(id) = id else {
                return "❌ <b>Usage:</b> <code>!bookmark archive &lt;id&gt;</code>".to_string();
            };

            match BookmarksRepository::toggle_archived(pool, id, &ctx.owner_id).await {
                Ok(b) => {
                    if let Some(bc) = broadcaster {
                        bc.send(SystemEvent::BookmarkUpdated(b.clone()));
                    }
                    let state = if b.is_archived { "Archived 📦" } else { "Restored to unread 📥" };
                    format!("{} bookmark <b>#{}</b>: {}", state, b.id, html_escape(&b.title))
                }
                Err(_) => format!("❌ Bookmark #{} not found.", id),
            }
        }
        "delete" | "rm" => {
            let id = parts.next().and_then(|s| s.parse::<i64>().ok());
            let Some(id) = id else {
                return "❌ <b>Usage:</b> <code>!bookmark rm &lt;id&gt;</code>".to_string();
            };

            match BookmarksRepository::delete(pool, id, &ctx.owner_id).await {
                Ok(true) => {
                    if let Some(bc) = broadcaster {
                        bc.send(SystemEvent::BookmarkDeleted {
                            id,
                            owner_id: ctx.owner_id.clone(),
                        });
                    }
                    format!("🗑️ Deleted bookmark <b>#{}</b>", id)
                }
                _ => format!("❌ Bookmark #{} not found.", id),
            }
        }
        url if url.starts_with("http://") || url.starts_with("https://") => {
            handle_save_command(ctx, pool, http_client, broadcaster).await
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
     • <code>!bookmark rm &lt;id&gt;</code> — Delete bookmark".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> (SqlitePool, String) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let owner = crate::accounts::AccountRepository::resolve_or_provision(&pool, "@alice:mbm.one")
            .await
            .unwrap();
        (pool, owner)
    }

    #[tokio::test]
    async fn test_handle_save_and_list_flow() {
        let (pool, owner) = setup_db().await;

        let ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner.clone(),
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "save".to_string(),
            raw_args: "https://example.com/rust-tips rust,coding".to_string(),
            full_body: "!save https://example.com/rust-tips rust,coding".to_string(),
            is_direct: true,
        };

        let reply = handle_save_command(&ctx, &pool, None, None).await;
        assert!(reply.contains("Saved Bookmark #1"));
        assert!(reply.contains("example.com"));

        let list_ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner.clone(),
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "bookmarks".to_string(),
            raw_args: String::new(),
            full_body: "!bookmarks".to_string(),
            is_direct: true,
        };

        let list_reply = handle_list_command(&list_ctx, &pool).await;
        assert!(list_reply.contains("Your Reading List"));
        assert!(list_reply.contains("example.com"));

        // Toggle fav
        let fav_ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner.clone(),
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "bookmark".to_string(),
            raw_args: "fav 1".to_string(),
            full_body: "!bookmark fav 1".to_string(),
            is_direct: true,
        };
        let fav_reply = handle_bookmark_subcommands(&fav_ctx, &pool, None, None).await;
        assert!(fav_reply.contains("Starred ⭐"));

        // Toggle archive
        let arch_ctx = CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: owner.clone(),
            sender_mxid: "@alice:mbm.one".to_string(),
            sender_name: "Alice".to_string(),
            command: "bookmark".to_string(),
            raw_args: "archive 1".to_string(),
            full_body: "!bookmark archive 1".to_string(),
            is_direct: true,
        };
        let arch_reply = handle_bookmark_subcommands(&arch_ctx, &pool, None, None).await;
        assert!(arch_reply.contains("Archived 📦"));
    }
}
