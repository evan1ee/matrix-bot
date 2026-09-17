use crate::db::Database;
use crate::modules::mcp::{adapter, models, prompts, repository::McpTokenRepository, resources};
use crate::modules::CommandContext;
use crate::utils::html::html_escape;

pub async fn handle_command(ctx: &CommandContext, db: &Database) -> Option<String> {
    if ctx.command != "mcp" {
        return None;
    }

    let raw = ctx.raw_args.trim();
    let mut parts = raw.split_whitespace();
    let sub = parts.next().unwrap_or("");

    match sub {
        "token" | "create" | "new" => {
            if !ctx.is_direct {
                return Some(
                    "⚠️ <b>Security Notice</b>: MCP access tokens grant direct access to your personal tools and data.<br/>\
                     Please send <code>!mcp token</code> in a <b>private direct message (DM)</b> with the bot to protect your credentials."
                        .to_string(),
                );
            }

            let token_name = {
                let rest: Vec<&str> = parts.collect();
                if rest.is_empty() {
                    "Claude Desktop".to_string()
                } else {
                    rest.join(" ")
                }
            };

            let (raw_token, token) = match McpTokenRepository::create(
                &db.pool,
                &ctx.owner_id,
                &token_name,
                None,
            )
            .await
            {
                Ok(res) => res,
                Err(err) => {
                    tracing::error!("Failed to create MCP token: {:?}", err);
                    return Some(format!("❌ Failed to create MCP token: {}", err));
                }
            };

            let endpoint_url = "https://your-bot-domain.com/api/v1/mcp/sse";
            let claude_cfg = models::build_claude_config(endpoint_url, &raw_token);
            let cursor_cfg = models::build_cursor_config(endpoint_url, &raw_token);

            let claude_json = serde_json::to_string_pretty(&claude_cfg).unwrap_or_default();
            let cursor_json = serde_json::to_string_pretty(&cursor_cfg).unwrap_or_default();

            let reply = format!(
                "<h3>🔑 MCP Token Generated</h3>\
                 <p><b>Name:</b> {}<br/>\
                 <b>Token:</b> <code>{}</code><br/>\
                 <small><i>Store this token safely now — it will not be displayed again.</i></small></p>\
                 <p><b>Claude Desktop Configuration (<code>claude_desktop_config.json</code>):</b></p>\
                 <pre><code>{}</code></pre>\
                 <p><b>Cursor Configuration (<code>.cursor/mcp.json</code>):</b></p>\
                 <pre><code>{}</code></pre>\
                 <p><i>To revoke this token at any time, run <code>!mcp revoke {}</code> or manage it in the Web Dashboard.</i></p>",
                html_escape(&token.name),
                html_escape(&raw_token),
                html_escape(&claude_json),
                html_escape(&cursor_json),
                token.id
            );

            Some(reply)
        }
        "list" | "ls" => {
            let tokens = match McpTokenRepository::list_for_account(&db.pool, &ctx.owner_id).await {
                Ok(t) => t,
                Err(err) => return Some(format!("❌ Could not list MCP tokens: {}", err)),
            };

            if tokens.is_empty() {
                return Some(
                    "<b>No active MCP tokens found.</b><br/>\
                     Type <code>!mcp token [name]</code> in a DM to generate one."
                        .to_string(),
                );
            }

            let mut list_html = String::from("<b>Active MCP Tokens:</b><ul>");
            for t in tokens {
                let last_used = t
                    .last_used_at
                    .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "Never".to_string());
                list_html.push_str(&format!(
                    "<li><b>{}</b> — Prefix: <code>{}</code> (ID: <code>{}</code>)<br/>\
                     Created: {} | Last used: {}</li>",
                    html_escape(&t.name),
                    html_escape(&t.prefix),
                    t.id,
                    t.created_at.format("%Y-%m-%d"),
                    last_used
                ));
            }
            list_html.push_str("</ul><p>Use <code>!mcp revoke &lt;ID&gt;</code> to revoke a token.</p>");

            Some(list_html)
        }
        "revoke" | "delete" | "rm" => {
            let target = parts.next().unwrap_or("");
            if target.is_empty() {
                return Some("⚠️ Please provide a token ID or prefix to revoke: <code>!mcp revoke &lt;ID&gt;</code>".to_string());
            }

            match McpTokenRepository::revoke_by_id_or_prefix(&db.pool, &ctx.owner_id, target).await {
                Ok(true) => Some("✅ MCP token revoked successfully.".to_string()),
                Ok(false) => Some("❌ No active MCP token found matching that ID or prefix.".to_string()),
                Err(err) => Some(format!("❌ Failed to revoke token: {}", err)),
            }
        }
        _ => {
            let tools = adapter::get_mcp_tools();
            let res = resources::list_resources();
            let p = prompts::list_prompts();

            let html = format!(
                "<b>Model Context Protocol (MCP) Server Active</b><br/>\
                 <ul>\
                   <li><b>Exposed Tools:</b> {} tools (todos, finance, portfolio, habits, bookmarks, life)</li>\
                   <li><b>Readable Resources:</b> {} URIs (<code>matrixbot://...</code>)</li>\
                   <li><b>Prompt Templates:</b> {} templates</li>\
                 </ul>\
                 <b>Available Commands:</b><br/>\
                 • <code>!mcp token [name]</code> — Generate personal access token and instant Claude/Cursor configs (DM only)<br/>\
                 • <code>!mcp list</code> — View all active tokens and last used timestamps<br/>\
                 • <code>!mcp revoke &lt;id&gt;</code> — Revoke a token<br/>\
                 • <code>!mcp</code> — Show this overview",
                tools.len(),
                res.len(),
                p.len()
            );

            Some(html)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    fn dummy_ctx(cmd: &str, raw_args: &str, is_direct: bool) -> CommandContext {
        CommandContext {
            room_id: "!room:test".to_string(),
            owner_id: "acct_test".to_string(),
            sender_mxid: "@user:test".to_string(),
            sender_name: "Test User".to_string(),
            command: cmd.to_string(),
            raw_args: raw_args.to_string(),
            full_body: format!("!{} {}", cmd, raw_args),
            is_direct,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    #[tokio::test]
    async fn test_mcp_token_refuses_public_room() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let db = Database { pool };
        let ctx = dummy_ctx("mcp", "token", false);

        let reply = handle_command(&ctx, &db).await.unwrap();
        assert!(reply.contains("Security Notice"));
        assert!(reply.contains("private direct message"));
    }

    #[tokio::test]
    async fn test_mcp_token_generates_in_direct_room() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema_auth::create_tables(&pool).await.unwrap();
        crate::db::schema_mcp::create_tables(&pool).await.unwrap();

        sqlx::query("INSERT INTO accounts (id, display_name) VALUES ('acct_test', 'Tester')")
            .execute(&pool)
            .await
            .unwrap();

        let db = Database { pool };
        let ctx = dummy_ctx("mcp", "token Claude Laptop", true);

        let reply = handle_command(&ctx, &db).await.unwrap();
        assert!(reply.contains("MCP Token Generated"));
        assert!(reply.contains("Claude Laptop"));
        assert!(reply.contains("mb_mcp_"));
        assert!(reply.contains("claude_desktop_config.json"));
    }
}
