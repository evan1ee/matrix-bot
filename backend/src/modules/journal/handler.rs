use chrono::NaiveDate;
use sqlx::SqlitePool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::life::activity::collect_day_activity;
use crate::modules::life::models::normalize_mood;
use crate::modules::life::repository::LifeRepository;
use crate::modules::settings::SettingsRepository;
use crate::modules::CommandContext;
use crate::utils::html::html_escape;

pub async fn handle_journal_command(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    ctx: &CommandContext,
) -> Option<String> {
    let cmd = ctx.command.to_lowercase();
    if cmd != "journal" && cmd != "daily" && cmd != "log" {
        return None;
    }

    let tz = SettingsRepository::get_or_default(pool, &ctx.owner_id)
        .await
        .tz();
    let today_str = chrono::Utc::now()
        .with_timezone(&tz)
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let raw_args = ctx.raw_args.trim();

    // If the top-level command was `!log`, route arguments directly to log handler
    if cmd == "log" {
        if raw_args.is_empty() {
            return Some(handle_view(pool, &ctx.owner_id, &today_str).await);
        }
        return Some(handle_log(pool, broadcaster, &ctx.owner_id, raw_args, &today_str).await);
    }

    let mut parts = raw_args.split_whitespace();
    let sub = parts.next().unwrap_or("").to_lowercase();

    match sub.as_str() {
        "" | "today" => Some(handle_view(pool, &ctx.owner_id, &today_str).await),
        "log" => {
            let rest = raw_args.trim_start_matches("log").trim();
            Some(handle_log(pool, broadcaster, &ctx.owner_id, rest, &today_str).await)
        }
        "activities" | "events" => {
            let target_date = parts.next().unwrap_or(&today_str);
            Some(handle_activities(pool, &ctx.owner_id, target_date).await)
        }
        "clear" | "delete" | "rm" => {
            let target_date = parts.next().unwrap_or(&today_str);
            Some(handle_clear(pool, broadcaster, &ctx.owner_id, target_date).await)
        }
        "help" => Some(handle_help()),
        date_cand if NaiveDate::parse_from_str(date_cand, "%Y-%m-%d").is_ok() => {
            Some(handle_view(pool, &ctx.owner_id, date_cand).await)
        }
        _ => Some(handle_help()),
    }
}

pub fn handle_help() -> String {
    "📔 <b>Daily Log (Journal) Commands:</b><br/>\
     • <code>!journal</code> / <code>!daily</code> — View today's status &amp; cross-module activity summary<br/>\
     • <code>!journal &lt;YYYY-MM-DD&gt;</code> — View log for a past date<br/>\
     • <code>!journal log &lt;mood&gt; [| weather] [| note]</code> — Record today (moods: amazing, good, neutral, low, rough)<br/>\
     • <code>!journal activities [date]</code> — Inspect auto-logged events across tasks, finance, habits, etc.<br/>\
     • <code>!journal clear [date]</code> — Clear a daily log entry".to_string()
}

async fn handle_view(pool: &SqlitePool, owner_id: &str, date_str: &str) -> String {
    let day_log = match LifeRepository::get_day_log(pool, owner_id, date_str).await {
        Ok(log) => log,
        Err(e) => {
            error!("Failed to get day log for {}: {:?}", date_str, e);
            return "⚠️ Database error retrieving daily log.".to_string();
        }
    };

    let activity = collect_day_activity(pool, owner_id, date_str).await;

    let (mood_disp, weather_disp, note_disp) = match day_log {
        Some(log) => {
            let mood_emoji = match log.mood.as_str() {
                "amazing" => "🌟",
                "good" => "🙂",
                "neutral" => "😐",
                "low" => "😔",
                "rough" => "🌧️",
                _ => "📝",
            };
            let mood = format!("{} <b>{}</b>", mood_emoji, html_escape(&log.mood));
            let weather = log
                .weather
                .map(|w| format!(" · 🌤️ {}", html_escape(&w)))
                .unwrap_or_default();
            let note = log
                .note
                .map(|n| format!("<br/><i>\"{}\"</i>", html_escape(&n)))
                .unwrap_or_default();
            (mood, weather, note)
        }
        None => (
            "<i>Not recorded yet</i>".to_string(),
            String::new(),
            String::new(),
        ),
    };

    let stats = activity.stats;
    let mut stat_pills: Vec<String> = Vec::new();
    if stats.todos_completed > 0 {
        stat_pills.push(format!("✅ {} tasks done", stats.todos_completed));
    }
    if stats.habits_done > 0 {
        stat_pills.push(format!("⚡ {} habits check-in", stats.habits_done));
    }
    if stats.expenses > 0 {
        stat_pills.push(format!("💸 {} expenses", stats.expenses));
    }
    if stats.income > 0 {
        stat_pills.push(format!("💰 {} income", stats.income));
    }
    if stats.portfolio_trades > 0 {
        stat_pills.push(format!("📈 {} trades", stats.portfolio_trades));
    }
    if stats.bookmarks_saved > 0 {
        stat_pills.push(format!("🔖 {} bookmarks", stats.bookmarks_saved));
    }
    if stats.footprints > 0 {
        stat_pills.push(format!("📍 {} places visited", stats.footprints));
    }

    let activities_summary = if stat_pills.is_empty() {
        "<i>No cross-module activities recorded for this day.</i>".to_string()
    } else {
        format!("<b>Auto-logged Activities:</b> {}", stat_pills.join(" · "))
    };

    format!(
        "📔 <b>Daily Log ({})</b><br/>\
         Mood: {}{}{}<br/><br/>\
         {}<br/><br/>\
         <i>Tip: Use <code>!journal log &lt;mood&gt; [| note]</code> to record today.</i>",
        date_str, mood_disp, weather_disp, note_disp, activities_summary
    )
}

async fn handle_activities(pool: &SqlitePool, owner_id: &str, date_str: &str) -> String {
    if NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
        return "⚠️ Invalid date format. Please use <code>YYYY-MM-DD</code>.".to_string();
    }

    let activity = collect_day_activity(pool, owner_id, date_str).await;
    if activity.events.is_empty() {
        return format!(
            "📅 <b>Activities for {}:</b><br/>No activity recorded from other modules on this day.",
            date_str
        );
    }

    let mut out = format!(
        "📅 <b>Cross-Module Activities for {} ({} events):</b><br/>",
        date_str, activity.stats.total_events
    );

    for ev in activity.events.iter().take(15) {
        let time_str = ev
            .time
            .as_deref()
            .map(|t| format!("<code>{}</code> ", t))
            .unwrap_or_default();
        out.push_str(&format!(
            "• {}{}{} &mdash; {}<br/>",
            ev.emoji,
            time_str,
            html_escape(&ev.action),
            html_escape(&ev.summary)
        ));
    }

    if activity.events.len() > 15 {
        out.push_str(&format!(
            "<i>...and {} more events on this day.</i><br/>",
            activity.events.len() - 15
        ));
    }

    out
}

async fn handle_log(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    args: &str,
    today_str: &str,
) -> String {
    if args.is_empty() {
        return "⚠️ Usage: <code>!journal log &lt;mood&gt; [| weather] [| note]</code><br/>\
                Moods: amazing, good, neutral, low, rough"
            .to_string();
    }

    let (mood_raw, weather_raw, note_raw) = if args.contains('|') {
        let segments: Vec<&str> = args.split('|').map(|s| s.trim()).collect();
        let m = segments[0];
        let w = segments.get(1).copied().filter(|s| !s.is_empty());
        let n = segments.get(2).copied().filter(|s| !s.is_empty());
        (m, w, n)
    } else {
        let mut parts = args.splitn(2, ' ');
        let m = parts.next().unwrap_or("");
        let rest = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());
        (m, None, rest)
    };

    let normalized_mood = match normalize_mood(mood_raw) {
        Some(m) => m,
        None => {
            return "⚠️ Unknown mood. Choose from: <b>amazing</b>, <b>good</b>, <b>neutral</b>, <b>low</b>, or <b>rough</b>.".to_string();
        }
    };

    match LifeRepository::upsert_day_log(
        pool,
        owner_id,
        today_str,
        normalized_mood,
        weather_raw,
        None,
        note_raw,
        None,
    )
    .await
    {
        Ok(log) => {
            if let Some(bc) = broadcaster {
                bc.send(SystemEvent::LifeDayLogged(log.clone()));
            }

            let mood_emoji = match normalized_mood {
                "amazing" => "🌟",
                "good" => "🙂",
                "neutral" => "😐",
                "low" => "😔",
                "rough" => "🌧️",
                _ => "📝",
            };

            let weather_disp = log
                .weather
                .map(|w| format!(" · 🌤️ {}", html_escape(&w)))
                .unwrap_or_default();
            let note_disp = log
                .note
                .map(|n| format!("<br/><i>\"{}\"</i>", html_escape(&n)))
                .unwrap_or_default();

            format!(
                "✅ <b>Daily Log Saved!</b> ({})<br/>\
                 Mood: {} <b>{}</b>{}{}",
                today_str, mood_emoji, normalized_mood, weather_disp, note_disp
            )
        }
        Err(e) => {
            error!("Failed to upsert daily log: {:?}", e);
            "⚠️ Failed to save daily log to database.".to_string()
        }
    }
}

async fn handle_clear(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    date_str: &str,
) -> String {
    if NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err() {
        return "⚠️ Invalid date format. Use <code>YYYY-MM-DD</code>.".to_string();
    }

    match LifeRepository::delete_day_log(pool, owner_id, date_str).await {
        Ok(true) => {
            if let Some(bc) = broadcaster {
                bc.send(SystemEvent::LifeDayCleared {
                    owner_id: owner_id.to_string(),
                    date: date_str.to_string(),
                });
            }
            format!("🗑️ Cleared daily log for <b>{}</b>.", date_str)
        }
        Ok(false) => format!("⚠️ No daily log entry found for <b>{}</b>.", date_str),
        Err(e) => {
            error!("Failed to delete daily log for {}: {:?}", date_str, e);
            "⚠️ Failed to clear daily log.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema_life::run_life_schema;

    async fn setup_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_life_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_journal_flow() {
        let pool = setup_db().await;
        let owner = "user_journal_test";

        let ctx = CommandContext {
            room_id: "!room:1".to_string(),
            sender_mxid: "@user:1".to_string(),
            owner_id: owner.to_string(),
            sender_name: "Tester".to_string(),
            command: "journal".to_string(),
            raw_args: "help".to_string(),
            full_body: "!journal help".to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        };

        let help = handle_journal_command(&pool, None, &ctx).await.unwrap();
        assert!(help.contains("Daily Log (Journal) Commands"));

        // Log today
        let mut log_ctx = ctx.clone();
        log_ctx.raw_args = "log amazing | sunny | High productivity day".to_string();
        let log_res = handle_journal_command(&pool, None, &log_ctx).await.unwrap();
        assert!(log_res.contains("Daily Log Saved"));
        assert!(log_res.contains("amazing"));

        // View today
        let mut view_ctx = ctx.clone();
        view_ctx.raw_args = "".to_string();
        let view_res = handle_journal_command(&pool, None, &view_ctx)
            .await
            .unwrap();
        assert!(view_res.contains("amazing"));
        assert!(view_res.contains("High productivity day"));

        // Activities
        let mut act_ctx = ctx.clone();
        act_ctx.raw_args = "activities".to_string();
        let act_res = handle_journal_command(&pool, None, &act_ctx).await.unwrap();
        assert!(act_res.contains("Activities for") || act_res.contains("No activity"));

        // Clear
        let mut clear_ctx = ctx.clone();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        clear_ctx.raw_args = format!("clear {}", today);
        let clear_res = handle_journal_command(&pool, None, &clear_ctx)
            .await
            .unwrap();
        assert!(clear_res.contains("Cleared daily log"));
    }
}
