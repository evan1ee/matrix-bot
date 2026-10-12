//! Chat interface for habit tracking.
//!
//! Handles `!habit` / `!habits` Matrix commands via `HabitService`.

use async_trait::async_trait;

use crate::interface::channels::message::Reply;
use tracing::error;

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape;

pub struct HabitsModule;

impl HabitsModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HabitsModule {
    fn default() -> Self {
        Self::new()
    }
}

impl HabitsModule {
    /// The command's HTML answer; `handle_command` wraps it in a [`Reply`].
    async fn respond(&self, ctx: &CommandContext, services: &AppServices) -> Option<String> {
        let cmd = ctx.command.to_lowercase();
        if cmd != "habit" && cmd != "habits" {
            return None;
        }

        let owner = &ctx.owner_id;
        let args = ctx.raw_args.trim();
        let mut parts = args.split_whitespace();
        let sub = parts.next().unwrap_or("").to_lowercase();

        let settings = services.settings.get_or_default(owner).await;
        let today_str = chrono::Utc::now()
            .with_timezone(&settings.tz())
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();

        match sub.as_str() {
            "list" | "" => Some(handle_list(services, owner, &today_str).await),
            "done" | "check" => {
                let target = args
                    .trim_start_matches("done")
                    .trim_start_matches("check")
                    .trim();
                Some(handle_toggle(services, owner, target, &today_str, true).await)
            }
            "undo" => {
                let target = args.trim_start_matches("undo").trim();
                Some(handle_toggle(services, owner, target, &today_str, false).await)
            }
            "add" => {
                let habit_name = args.trim_start_matches("add").trim();
                Some(handle_add(services, owner, habit_name).await)
            }
            "streak" | "streaks" => Some(handle_streaks(services, owner, &today_str).await),
            "delete" | "rm" => {
                let id_str = parts.next().unwrap_or("");
                Some(handle_delete(services, owner, id_str).await)
            }
            _ => Some(handle_toggle(services, owner, args, &today_str, true).await),
        }
    }
}

#[async_trait]
impl BotModule for HabitsModule {
    fn name(&self) -> &str {
        "Habits"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "habit list".to_string(),
                usage: "!habit [list]".to_string(),
                description: "List today's habits, completion status, and streaks".to_string(),
            },
            CommandHelp {
                name: "habit done".to_string(),
                usage: "!habit done <name or id>".to_string(),
                description: "Check off a habit for today".to_string(),
            },
            CommandHelp {
                name: "habit undo".to_string(),
                usage: "!habit undo <name or id>".to_string(),
                description: "Uncheck a habit for today".to_string(),
            },
            CommandHelp {
                name: "habit add".to_string(),
                usage: "!habit add <name>".to_string(),
                description: "Create a new daily habit".to_string(),
            },
            CommandHelp {
                name: "habit streak".to_string(),
                usage: "!habit streak".to_string(),
                description: "View streaks leaderboard".to_string(),
            },
            CommandHelp {
                name: "habit delete".to_string(),
                usage: "!habit delete <id>".to_string(),
                description: "Delete a habit by ID".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        self.respond(ctx, services).await.map(Reply::from)
    }
}

async fn handle_list(services: &AppServices, owner: &AccountId, today_str: &str) -> String {
    let habits = match services.habits.list_with_status(owner, today_str).await {
        Ok(list) => list,
        Err(e) => {
            error!("Failed to list habits: {}", e);
            return "<p>⚠️ Failed to load your habits.</p>".to_string();
        }
    };

    if habits.is_empty() {
        return "<p>⚡ <b>Daily Habits:</b> No active habits yet. Add one with <code>!habit add &lt;name&gt;</code>!</p>".to_string();
    }

    let completed_count = habits.iter().filter(|h| h.completed_today).count();
    let total = habits.len();

    let mut html = format!(
        "<p>⚡ <b>Today's Cadence ({}/{} completed):</b></p><ul>",
        completed_count, total
    );

    for h in &habits {
        let check = if h.completed_today { "✅" } else { "⬜" };
        let streak_badge = if h.current_streak > 0 {
            format!(" · 🔥 {}d streak", h.current_streak)
        } else {
            String::new()
        };

        html.push_str(&format!(
            "<li>{} <b>#{} {}</b> (<i>{}</i>){}</li>",
            check,
            h.habit.id,
            html_escape(&h.habit.name),
            html_escape(&h.habit.category),
            streak_badge
        ));
    }

    html.push_str("</ul><p>Check off with <code>!habit done &lt;name or id&gt;</code></p>");
    html
}

async fn handle_toggle(
    services: &AppServices,
    owner: &AccountId,
    target: &str,
    today_str: &str,
    mark_completed: bool,
) -> String {
    if target.is_empty() {
        return "<p>⚠️ Usage: <code>!habit done &lt;name or id&gt;</code></p>".to_string();
    }

    let habit = if let Ok(id) = target.parse::<i64>() {
        services.habits.find_by_id(owner, id).await
    } else {
        services.habits.find_by_name(owner, target).await
    };

    let habit = match habit {
        Ok(Some(h)) => h,
        Ok(None) => return format!("<p>⚠️ Habit \"{}\" not found.</p>", html_escape(target)),
        Err(e) => {
            return format!(
                "<p>⚠️ Error finding habit: {}</p>",
                html_escape(&e.to_string())
            )
        }
    };

    let current_state = services
        .habits
        .list_with_status(owner, today_str)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|h| h.habit.id == habit.id);

    let is_already = current_state
        .as_ref()
        .map(|h| h.completed_today)
        .unwrap_or(false);

    if is_already != mark_completed {
        if let Err(e) = services
            .habits
            .toggle_log(owner, habit.id, today_str, None)
            .await
        {
            return format!(
                "<p>⚠️ Error updating habit log: {}</p>",
                html_escape(&e.to_string())
            );
        }
    }

    let updated = services
        .habits
        .list_with_status(owner, today_str)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|h| h.habit.id == habit.id);

    let streak = updated.as_ref().map(|h| h.current_streak).unwrap_or(0);

    let (icon, verb) = if mark_completed {
        ("✅", "completed")
    } else {
        ("↩️", "unmarked")
    };

    let streak_str = if streak > 0 {
        format!(" 🔥 Streak: <b>{} days</b>", streak)
    } else {
        String::new()
    };

    format!(
        "<p>{} Habit <b>{}</b> {} for today!{}</p>",
        icon,
        html_escape(&habit.name),
        verb,
        streak_str
    )
}

async fn handle_add(services: &AppServices, owner: &AccountId, name: &str) -> String {
    if name.is_empty() {
        return "<p>⚠️ Usage: <code>!habit add &lt;habit name&gt;</code></p>".to_string();
    }
    if name.len() > 100 {
        return "<p>⚠️ Habit name must be 100 characters or fewer.</p>".to_string();
    }

    match services
        .habits
        .create_habit(owner, name, None, Some("daily"), Some("wellness"), Some(7))
        .await
    {
        Ok(habit) => {
            format!(
                "<p>⚡ Created habit #{} <b>{}</b> (daily). Check it off today with <code>!habit done {}</code>!</p>",
                habit.id,
                html_escape(&habit.name),
                habit.id
            )
        }
        Err(e) => format!(
            "<p>⚠️ Failed to create habit: {}</p>",
            html_escape(&e.to_string())
        ),
    }
}

async fn handle_streaks(services: &AppServices, owner: &AccountId, today_str: &str) -> String {
    let habits = match services.habits.list_with_status(owner, today_str).await {
        Ok(list) => list,
        Err(e) => return format!("<p>⚠️ Error: {}</p>", html_escape(&e.to_string())),
    };

    if habits.is_empty() {
        return "<p>⚠️ No active habits found.</p>".to_string();
    }

    let mut html = "<p>🔥 <b>Habit Streaks Leaderboard:</b></p><ul>".to_string();
    for h in habits {
        html.push_str(&format!(
            "<li><b>{}</b>: 🔥 {} days (Best: {} days, All-time: {} check-ins)</li>",
            html_escape(&h.habit.name),
            h.current_streak,
            h.longest_streak,
            h.total_completions
        ));
    }
    html.push_str("</ul>");
    html
}

async fn handle_delete(services: &AppServices, owner: &AccountId, id_str: &str) -> String {
    let id = match id_str.parse::<i64>() {
        Ok(val) => val,
        Err(_) => {
            return "<p>⚠️ Provide a valid numeric habit ID: <code>!habit delete &lt;id&gt;</code></p>".to_string()
        }
    };

    match services.habits.delete_habit(owner, id).await {
        Ok(()) => format!("<p>🗑️ Deleted habit #{}.</p>", id),
        Err(crate::application::AppError::NotFound(_)) => {
            format!("<p>⚠️ Habit #{} not found.</p>", id)
        }
        Err(e) => format!(
            "<p>⚠️ Error deleting habit: {}</p>",
            html_escape(&e.to_string())
        ),
    }
}
