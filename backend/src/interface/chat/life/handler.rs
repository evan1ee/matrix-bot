use tracing::error;

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::life::normalize_mood;
use crate::domain::life::DayLogInput;
use crate::utils::html::html_escape;

pub async fn handle_life_command(
    services: &AppServices,
    owner_id: &AccountId,
    command: &str,
    raw_args: &str,
) -> Option<String> {
    let cmd = command.to_lowercase();
    if cmd != "life" && cmd != "lifegrid" {
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
        "stats" | "" => Some(handle_stats(services, owner_id, &today_str).await),
        "today" => Some(handle_today(services, owner_id, &today_str).await),
        "set-birth" | "birth" => {
            let rest = args
                .trim_start_matches("set-birth")
                .trim_start_matches("birth")
                .trim();
            Some(handle_set_birth(services, owner_id, rest).await)
        }
        "log" => {
            let rest = args.trim_start_matches("log").trim();
            Some(handle_log(services, owner_id, rest, &today_str).await)
        }
        "help" => Some(handle_help()),
        _ => Some(handle_help()),
    }
}

fn handle_help() -> String {
    "<b>Visualizing Life (Life Grid) Commands:</b><br/>\
     • <code>!life stats</code> — View age in days, % of life lived, and log summary<br/>\
     • <code>!life today</code> — View today's recorded status<br/>\
     • <code>!life set-birth &lt;YYYY-MM-DD&gt; [years]</code> — Set birth date &amp; expected lifespan (e.g. <code>!life set-birth 1995-06-15 85</code>)<br/>\
     • <code>!life log &lt;mood&gt; [note]</code> — Log today (moods: amazing, good, neutral, low, rough)".to_string()
}

async fn handle_stats(services: &AppServices, owner_id: &AccountId, today_str: &str) -> String {
    match services.life.get_stats(owner_id, today_str).await {
        Ok(stats) => {
            let profile_msg = match &stats.profile {
                Some(p) => format!(
                    "🎂 <b>Birth date:</b> {} (lifespan: {} yrs)<br/>",
                    p.birth_date, p.expected_lifespan_years
                ),
                None => "<i>Birth date not set yet (use <code>!life set-birth YYYY-MM-DD</code>).</i><br/>".to_string(),
            };

            let dominant = stats.dominant_mood.unwrap_or_else(|| "none".to_string());

            format!(
                "<b>Life Grid Statistics</b><br/>\
                 {}\
                 ⏳ <b>Days passed:</b> {} days<br/>\
                 📊 <b>Life elapsed:</b> {}%<br/>\
                 📝 <b>Days logged:</b> {}<br/>\
                 🌟 <b>Dominant mood:</b> {}",
                profile_msg,
                stats.age_days,
                stats.days_lived_percent,
                stats.total_logged_days,
                html_escape(&dominant)
            )
        }
        Err(e) => {
            error!("Failed to fetch life stats: {}", e);
            "⚠️ Failed to retrieve life statistics.".to_string()
        }
    }
}

async fn handle_today(services: &AppServices, owner_id: &AccountId, today_str: &str) -> String {
    match services.life.get_day_log(owner_id, today_str).await {
        Ok(Some(log)) => {
            let note = log
                .note
                .map(|n| format!("<br/><i>\"{}\"</i>", html_escape(&n)))
                .unwrap_or_default();
            let weather = log
                .weather
                .map(|w| format!(" · 🌤️ {}", html_escape(&w)))
                .unwrap_or_default();
            format!(
                "📅 <b>Today ({})</b><br/>\
                 Mood: <b>{}</b>{}{}",
                today_str,
                html_escape(&log.mood),
                weather,
                note
            )
        }
        Ok(None) => {
            format!(
                "📅 <b>Today ({})</b> has not been logged yet.<br/>\
                 Use <code>!life log &lt;mood&gt; [note]</code> to record today!",
                today_str
            )
        }
        Err(e) => {
            error!("Failed to get today's log: {}", e);
            "⚠️ Failed to check today's record.".to_string()
        }
    }
}

async fn handle_set_birth(services: &AppServices, owner_id: &AccountId, args: &str) -> String {
    let mut parts = args.split_whitespace();
    let birth_date = parts.next().unwrap_or("");
    let lifespan_str = parts.next().unwrap_or("80");

    if chrono::NaiveDate::parse_from_str(birth_date, "%Y-%m-%d").is_err() {
        return "⚠️ Invalid date format. Please use <code>YYYY-MM-DD</code> (e.g. <code>!life set-birth 1995-06-15 85</code>).".to_string();
    }

    let lifespan = lifespan_str.parse::<i64>().unwrap_or(80).clamp(40, 130);

    match services
        .life
        .upsert_profile(owner_id, birth_date, lifespan)
        .await
    {
        Ok(profile) => {
            format!(
                "✅ <b>Life Profile Updated!</b><br/>\
                 Birth date set to <b>{}</b> with expected lifespan of <b>{} years</b>.",
                profile.birth_date, profile.expected_lifespan_years
            )
        }
        Err(e) => {
            error!("Failed to update life profile: {}", e);
            "⚠️ Failed to save life profile.".to_string()
        }
    }
}

async fn handle_log(
    services: &AppServices,
    owner_id: &AccountId,
    args: &str,
    today_str: &str,
) -> String {
    let mut parts = args.splitn(2, ' ');
    let mood_raw = parts.next().unwrap_or("");
    let note_raw = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

    let normalized_mood = match normalize_mood(mood_raw) {
        Some(m) => m,
        None => {
            return "⚠️ Unknown mood. Choose from: <b>amazing</b>, <b>good</b>, <b>neutral</b>, <b>low</b>, or <b>rough</b>.".to_string();
        }
    };

    match services
        .life
        .upsert_day_log(
            owner_id,
            today_str,
            DayLogInput {
                mood: normalized_mood,
                weather: None,
                activities: None,
                note: note_raw,
                time_allocation: None,
            },
        )
        .await
    {
        Ok(log) => {
            let note_disp = log
                .note
                .map(|n| format!("<br/><i>\"{}\"</i>", html_escape(&n)))
                .unwrap_or_default();
            format!(
                "✅ <b>Day Recorded!</b> ({})<br/>Mood: <b>{}</b>{}",
                today_str, normalized_mood, note_disp
            )
        }
        Err(e) => {
            error!("Failed to log day: {}", e);
            "⚠️ Failed to record day.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_life_commands() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["user_matrix_1"]).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("user_matrix_1");

        // Help
        let help = handle_life_command(&services, &owner, "life", "help")
            .await
            .unwrap();
        assert!(help.contains("Visualizing Life"));

        // Set birth
        let set_res = handle_life_command(&services, &owner, "life", "set-birth 1990-01-01 80")
            .await
            .unwrap();
        assert!(set_res.contains("Life Profile Updated"));

        // Log today
        let log_res = handle_life_command(&services, &owner, "life", "log amazing Best day ever")
            .await
            .unwrap();
        assert!(log_res.contains("Day Recorded"));

        // Today command
        let today_res = handle_life_command(&services, &owner, "life", "today")
            .await
            .unwrap();
        assert!(today_res.contains("amazing"));

        // Stats command
        let stats_res = handle_life_command(&services, &owner, "life", "stats")
            .await
            .unwrap();
        assert!(stats_res.contains("Life Grid Statistics"));
    }
}
