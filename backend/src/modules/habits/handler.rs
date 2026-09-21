use crate::db::DbPool;
use tracing::error;

use super::repository::HabitsRepository;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::utils::html::html_escape;

pub async fn handle_habit_command(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    command: &str,
    raw_args: &str,
) -> Option<String> {
    let cmd = command.to_lowercase();
    if cmd != "habit" && cmd != "habits" {
        return None;
    }

    let args = raw_args.trim();
    let mut parts = args.split_whitespace();
    let sub = parts.next().unwrap_or("").to_lowercase();

    let today_str = chrono::Utc::now()
        .naive_utc()
        .date()
        .format("%Y-%m-%d")
        .to_string();

    match sub.as_str() {
        "list" | "" => Some(handle_list(pool, owner_id, &today_str).await),
        "done" | "check" => {
            let target = args
                .trim_start_matches("done")
                .trim_start_matches("check")
                .trim();
            Some(handle_toggle(pool, broadcaster, owner_id, target, &today_str, true).await)
        }
        "undo" => {
            let target = args.trim_start_matches("undo").trim();
            Some(handle_toggle(pool, broadcaster, owner_id, target, &today_str, false).await)
        }
        "add" => {
            let habit_name = args.trim_start_matches("add").trim();
            Some(handle_add(pool, broadcaster, owner_id, habit_name).await)
        }
        "streak" | "streaks" => Some(handle_streaks(pool, owner_id, &today_str).await),
        "delete" | "rm" => {
            let id_str = parts.next().unwrap_or("");
            Some(handle_delete(pool, broadcaster, owner_id, id_str).await)
        }
        _ => Some(handle_toggle(pool, broadcaster, owner_id, args, &today_str, true).await),
    }
}

async fn handle_list(pool: &DbPool, owner_id: &str, today_str: &str) -> String {
    let habits = match HabitsRepository::find_all_with_status(pool, owner_id, today_str).await {
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
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    target: &str,
    today_str: &str,
    mark_completed: bool,
) -> String {
    if target.is_empty() {
        return "<p>⚠️ Usage: <code>!habit done &lt;name or id&gt;</code></p>".to_string();
    }

    let habit = if let Ok(id) = target.parse::<i64>() {
        HabitsRepository::find_by_id(pool, id, owner_id).await
    } else {
        HabitsRepository::find_by_name(pool, target, owner_id).await
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

    let current_state = HabitsRepository::find_all_with_status(pool, owner_id, today_str)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|h| h.habit.id == habit.id);

    let is_already = current_state
        .as_ref()
        .map(|h| h.completed_today)
        .unwrap_or(false);

    if is_already != mark_completed {
        if let Err(e) =
            HabitsRepository::toggle_log(pool, habit.id, owner_id, today_str, None).await
        {
            return format!(
                "<p>⚠️ Error updating habit log: {}</p>",
                html_escape(&e.to_string())
            );
        }

        if let Some(bc) = broadcaster {
            bc.send(SystemEvent::HabitLogToggled {
                habit_id: habit.id,
                owner_id: owner_id.to_string(),
                log_date: today_str.to_string(),
                completed: mark_completed,
            });
        }
    }

    let updated = HabitsRepository::find_all_with_status(pool, owner_id, today_str)
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

async fn handle_add(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
) -> String {
    if name.is_empty() {
        return "<p>⚠️ Usage: <code>!habit add &lt;habit name&gt;</code></p>".to_string();
    }
    if name.len() > 100 {
        return "<p>⚠️ Habit name must be 100 characters or fewer.</p>".to_string();
    }

    match HabitsRepository::create_habit(pool, owner_id, name, None, "daily", "wellness", 7).await {
        Ok(habit) => {
            if let Some(bc) = broadcaster {
                bc.send(SystemEvent::HabitCreated(habit.clone()));
            }
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

async fn handle_streaks(pool: &DbPool, owner_id: &str, today_str: &str) -> String {
    let habits = match HabitsRepository::find_all_with_status(pool, owner_id, today_str).await {
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

async fn handle_delete(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id_str: &str,
) -> String {
    let id = match id_str.parse::<i64>() {
        Ok(val) => val,
        Err(_) => return "<p>⚠️ Provide a valid numeric habit ID: <code>!habit delete &lt;id&gt;</code></p>".to_string(),
    };

    match HabitsRepository::delete_habit(pool, id, owner_id).await {
        Ok(true) => {
            if let Some(bc) = broadcaster {
                bc.send(SystemEvent::HabitDeleted {
                    id,
                    owner_id: owner_id.to_string(),
                });
            }
            format!("<p>🗑️ Deleted habit #{}.</p>", id)
        }
        Ok(false) => format!("<p>⚠️ Habit #{} not found.</p>", id),
        Err(e) => format!(
            "<p>⚠️ Error deleting habit: {}</p>",
            html_escape(&e.to_string())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::schema_habits::create_tables(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_habit_commands() {
        let pool = setup_db().await;
        let broadcaster = EventBroadcaster::new(10);

        // Add
        let res = handle_habit_command(
            &pool,
            Some(&broadcaster),
            "user_1",
            "habit",
            "add Read 30 mins",
        )
        .await
        .unwrap();
        assert!(res.contains("Read 30 mins"));

        // List
        let list_res = handle_habit_command(&pool, Some(&broadcaster), "user_1", "habit", "list")
            .await
            .unwrap();
        assert!(list_res.contains("Read 30 mins"));
        assert!(list_res.contains("⬜"));

        // Done
        let done_res = handle_habit_command(
            &pool,
            Some(&broadcaster),
            "user_1",
            "habit",
            "done Read 30 mins",
        )
        .await
        .unwrap();
        assert!(done_res.contains("completed for today"));

        // Streak
        let streak_res =
            handle_habit_command(&pool, Some(&broadcaster), "user_1", "habit", "streak")
                .await
                .unwrap();
        assert!(streak_res.contains("Leaderboard"));
    }
}
