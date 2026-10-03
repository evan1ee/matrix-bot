//! Matrix bot module for autonomous agent management and interactive approvals.

use async_trait::async_trait;
use uuid::Uuid;

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::agent::{AgentRunId, ApprovalId};
use crate::interface::channels::message::Reply;
use crate::interface::chat::{error_reply, BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape;

pub struct AgentModule;

impl AgentModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgentModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for AgentModule {
    fn name(&self) -> &str {
        "Agent"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "agent run".to_string(),
                usage: "!agent run <goal>".to_string(),
                description: "Start an autonomous multi-step agent run".to_string(),
            },
            CommandHelp {
                name: "agent status".to_string(),
                usage: "!agent status [run_id]".to_string(),
                description: "Check the status of an active or specific agent run".to_string(),
            },
            CommandHelp {
                name: "agent confirm".to_string(),
                usage: "!agent confirm <approval_id>".to_string(),
                description: "Approve a sensitive agent action".to_string(),
            },
            CommandHelp {
                name: "agent reject".to_string(),
                usage: "!agent reject <approval_id> [reason]".to_string(),
                description: "Reject a sensitive agent action".to_string(),
            },
            CommandHelp {
                name: "agent cancel".to_string(),
                usage: "!agent cancel [run_id]".to_string(),
                description: "Cancel an active or specific agent run".to_string(),
            },
            CommandHelp {
                name: "agent list".to_string(),
                usage: "!agent list".to_string(),
                description: "List recent agent runs".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        if ctx.command != "agent" {
            return None;
        }

        let args = ctx.raw_args.trim();
        let mut parts = args.split_whitespace();
        let subcommand = parts.next().unwrap_or("help");
        let rest = args.strip_prefix(subcommand).unwrap_or("").trim();

        let reply = match subcommand {
            "run" => handle_run(ctx, services, rest).await,
            "status" => handle_status(ctx, services, rest).await,
            "confirm" | "approve" => handle_confirm(ctx, services, rest).await,
            "reject" => handle_reject(ctx, services, rest).await,
            "cancel" => handle_cancel(ctx, services, rest).await,
            "list" => handle_list(ctx, services).await,
            _ => Reply::html(format_help()),
        };

        Some(reply)
    }
}

async fn handle_run(ctx: &CommandContext, services: &AppServices, goal: &str) -> Reply {
    let goal_trimmed = goal.trim();
    if goal_trimmed.is_empty() {
        return Reply::html("❌ <b>Usage:</b> <code>!agent run &lt;goal description&gt;</code>");
    }

    if goal_trimmed.chars().count() > 500 {
        return Reply::html("❌ <b>Goal too long:</b> Please keep goals under 500 characters.");
    }

    match services
        .agent
        .start_user_goal(
            ctx.owner_id,
            Some(ctx.room_key()),
            goal_trimmed,
            None,
        )
        .await
    {
        Ok(run) => Reply::html(format!(
            "<p>🤖 <b>Agent Run Started</b></p>\
             <ul>\
             <li><b>Run ID:</b> <code>{}</code></li>\
             <li><b>Goal:</b> {}</li>\
             <li><b>Max Steps:</b> {}</li>\
             </ul>\
             <p><i>The agent is executing autonomously in the background. You will receive updates here upon completion or if confirmation is required.</i></p>",
            run.id,
            html_escape(&run.goal),
            run.max_steps
        )),
        Err(err) => error_reply(err),
    }
}

async fn handle_status(ctx: &CommandContext, services: &AppServices, arg: &str) -> Reply {
    let run_res = if !arg.is_empty() {
        match Uuid::parse_str(arg.trim()) {
            Ok(u) => {
                services
                    .agent
                    .get_run(&ctx.owner_id, AgentRunId::new(u))
                    .await
            }
            Err(_) => {
                return Reply::html("❌ <b>Invalid Run ID:</b> Please provide a valid UUID.");
            }
        }
    } else {
        match services.agent.get_active_run(&ctx.owner_id).await {
            Ok(Some(r)) => Ok(r),
            Ok(None) => {
                return Reply::html(
                    "ℹ️ No active agent run found. Use <code>!agent list</code> to view past runs.",
                );
            }
            Err(e) => Err(e),
        }
    };

    match run_res {
        Ok(run) => {
            let steps = services
                .agent
                .get_run_steps(&ctx.owner_id, run.id)
                .await
                .unwrap_or_default();

            let steps_html: String = if steps.is_empty() {
                "<i>No steps recorded yet.</i>".to_string()
            } else {
                let items: Vec<String> = steps
                    .iter()
                    .rev()
                    .take(3)
                    .map(|s| {
                        format!(
                            "<li><b>Step {}:</b> {} [<code>{}</code>]</li>",
                            s.step_number,
                            html_escape(s.tool_name.as_deref().unwrap_or("Thought")),
                            s.status
                        )
                    })
                    .collect();
                format!("<ul>{}</ul>", items.join(""))
            };

            Reply::html(format!(
                "<p>🤖 <b>Agent Run Status</b></p>\
                 <ul>\
                 <li><b>ID:</b> <code>{}</code></li>\
                 <li><b>Status:</b> <code>{}</code></li>\
                 <li><b>Goal:</b> {}</li>\
                 <li><b>Progress:</b> Step {} / {}</li>\
                 <li><b>Tokens Used:</b> {}</li>\
                 </ul>\
                 <p><b>Recent Steps:</b></p>{}",
                run.id,
                run.status,
                html_escape(&run.goal),
                run.current_step,
                run.max_steps,
                run.total_tokens,
                steps_html
            ))
        }
        Err(err) => error_reply(err),
    }
}

async fn handle_confirm(ctx: &CommandContext, services: &AppServices, arg: &str) -> Reply {
    let approval_id_str = arg.trim();
    if approval_id_str.is_empty() {
        return Reply::html("❌ <b>Usage:</b> <code>!agent confirm &lt;approval_id&gt;</code>");
    }

    let approval_id = match Uuid::parse_str(approval_id_str) {
        Ok(u) => ApprovalId::new(u),
        Err(_) => return Reply::html("❌ <b>Invalid Approval ID:</b> Expected a valid UUID."),
    };

    match services.agent.approve_action(&ctx.owner_id, approval_id).await {
        Ok(()) => Reply::html(
            "✅ <b>Action Approved:</b> The agent has been resumed and will execute the approved action."
        ),
        Err(err) => error_reply(err),
    }
}

async fn handle_reject(ctx: &CommandContext, services: &AppServices, args: &str) -> Reply {
    let mut parts = args.split_whitespace();
    let approval_id_str = parts.next().unwrap_or("");
    if approval_id_str.is_empty() {
        return Reply::html(
            "❌ <b>Usage:</b> <code>!agent reject &lt;approval_id&gt; [reason]</code>",
        );
    }

    let approval_id = match Uuid::parse_str(approval_id_str) {
        Ok(u) => ApprovalId::new(u),
        Err(_) => return Reply::html("❌ <b>Invalid Approval ID:</b> Expected a valid UUID."),
    };

    let reason = args.strip_prefix(approval_id_str).unwrap_or("").trim();
    let reason_opt = if reason.is_empty() {
        None
    } else {
        Some(reason)
    };

    match services
        .agent
        .reject_action(&ctx.owner_id, approval_id, reason_opt)
        .await
    {
        Ok(()) => Reply::html(
            "🚫 <b>Action Rejected:</b> The rejection has been recorded and the agent informed to adapt its plan."
        ),
        Err(err) => error_reply(err),
    }
}

async fn handle_cancel(ctx: &CommandContext, services: &AppServices, arg: &str) -> Reply {
    let run_id = if !arg.trim().is_empty() {
        match Uuid::parse_str(arg.trim()) {
            Ok(u) => AgentRunId::new(u),
            Err(_) => return Reply::html("❌ <b>Invalid Run ID:</b> Expected a valid UUID."),
        }
    } else {
        match services.agent.get_active_run(&ctx.owner_id).await {
            Ok(Some(r)) => r.id,
            Ok(None) => return Reply::html("ℹ️ No active run to cancel."),
            Err(e) => return error_reply(e),
        }
    };

    match services.agent.cancel_run(&ctx.owner_id, run_id).await {
        Ok(()) => Reply::html(format!(
            "⏹️ <b>Agent Run Cancelled:</b> Run <code>{}</code> has been cancelled.",
            run_id
        )),
        Err(err) => error_reply(err),
    }
}

async fn handle_list(ctx: &CommandContext, services: &AppServices) -> Reply {
    match services.agent.list_runs(&ctx.owner_id, 5).await {
        Ok(runs) => {
            if runs.is_empty() {
                return Reply::html(
                    "ℹ️ No agent runs found. Start one with <code>!agent run &lt;goal&gt;</code>.",
                );
            }

            let items: Vec<String> = runs
                .iter()
                .map(|r| {
                    format!(
                        "<li><code>{}</code> [<b>{}</b>] - {} (Steps: {}/{})</li>",
                        r.id,
                        r.status,
                        html_escape(&r.goal),
                        r.current_step,
                        r.max_steps
                    )
                })
                .collect();

            Reply::html(format!(
                "<p>🤖 <b>Recent Agent Runs:</b></p><ul>{}</ul>",
                items.join("")
            ))
        }
        Err(err) => error_reply(err),
    }
}

fn format_help() -> String {
    "<p>🤖 <b>Agent Commands:</b></p>\
     <ul>\
     <li><code>!agent run &lt;goal&gt;</code> - Start an autonomous goal</li>\
     <li><code>!agent status [id]</code> - Check current or specific run status</li>\
     <li><code>!agent confirm &lt;approval_id&gt;</code> - Approve pending sensitive action</li>\
     <li><code>!agent reject &lt;approval_id&gt; [reason]</code> - Reject pending action</li>\
     <li><code>!agent cancel [id]</code> - Cancel active or specific run</li>\
     <li><code>!agent list</code> - View recent runs</li>\
     </ul>"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;

    async fn setup_db() -> (crate::db::DbPool, AppServices, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("test_agent_chat_module");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        (pool, services, owner)
    }

    fn make_ctx(owner: AccountId, command: &str, raw_args: &str) -> CommandContext {
        let mut ctx = crate::interface::chat::test_support::context_for(
            "@alice:server",
            "alice",
            command,
            raw_args,
            &format!("!{} {}", command, raw_args),
        );
        ctx.owner_id = owner;
        ctx
    }

    #[tokio::test]
    async fn agent_module_lifecycle_chat_commands() {
        let (pool, services, owner) = setup_db().await;
        let db = Database { pool };
        let module = AgentModule::new();

        // 1. Run goal
        let ctx_run = make_ctx(owner, "agent", "run Organize my inbox bookmarks");
        let reply = module
            .handle_command(&ctx_run, &services, &db)
            .await
            .unwrap();
        assert!(reply.html_body().contains("Agent Run Started"));

        // 2. Status
        let ctx_status = make_ctx(owner, "agent", "status");
        let reply_status = module
            .handle_command(&ctx_status, &services, &db)
            .await
            .unwrap();
        assert!(reply_status.html_body().contains("Agent Run Status"));

        // 3. Cancel
        let ctx_cancel = make_ctx(owner, "agent", "cancel");
        let reply_cancel = module
            .handle_command(&ctx_cancel, &services, &db)
            .await
            .unwrap();
        assert!(reply_cancel.html_body().contains("Agent Run Cancelled"));
    }
}
