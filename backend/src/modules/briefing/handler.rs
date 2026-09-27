use crate::application::settings::SettingsService;
use crate::db::DbPool;
use crate::domain::settings::parse_briefing_time;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::briefing::generator::generate_briefing;
use crate::utils::html_escape;

pub async fn handle_briefing_command(
    pool: &DbPool,
    settings: &SettingsService,
    owner_id: &str,
    clean_body: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    let body = clean_body.trim();
    let lower = body.to_lowercase();

    let rest = strip_command(&lower, body)?;
    let mut parts = rest.split_whitespace();
    let subcmd = parts.next().unwrap_or("").to_lowercase();
    let arg = parts.next().unwrap_or("").trim();

    match subcmd.as_str() {
        "" | "now" | "today" => Some(generate_now(pool, settings, owner_id, broadcaster).await),
        "time" => Some(set_time(settings, owner_id, arg).await),
        "on" | "enable" => Some(toggle(settings, owner_id, true).await),
        "off" | "disable" => Some(toggle(settings, owner_id, false).await),
        "status" => Some(show_status(settings, owner_id).await),
        "help" => Some(usage()),
        other => Some(format!(
            "⚠️ 未知简报指令 <code>{}</code>。<br/>{}",
            html_escape(other),
            usage()
        )),
    }
}

fn strip_command<'a>(lower: &str, body: &'a str) -> Option<&'a str> {
    for prefix in ["!briefing", "briefing", "!morning", "morning"] {
        if lower == prefix {
            return Some("");
        }
        if let Some(stripped) = lower.strip_prefix(&format!("{} ", prefix)) {
            let offset = body.len() - stripped.len();
            return Some(body[offset..].trim());
        }
    }
    None
}

async fn generate_now(
    pool: &DbPool,
    settings: &SettingsService,
    owner_id: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let settings = settings.get_or_default(&acct).await;
    match generate_briefing(pool, owner_id, &settings).await {
        Ok(payload) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::MorningBriefingSent {
                    owner_id: owner_id.to_string(),
                    delivered_at: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                    summary: format!(
                        "{} tasks, {} habits, {} bills, {} holdings",
                        payload.summary.pending_tasks_count,
                        payload.summary.habits_completed_count,
                        payload.summary.upcoming_bills_count,
                        payload.summary.holdings_count
                    ),
                });
            }
            payload.html_content
        }
        Err(err) => {
            format!("❌ 生成早安简报失败: {}", html_escape(&err.to_string()))
        }
    }
}

async fn set_time(settings: &SettingsService, owner_id: &str, raw_time: &str) -> String {
    if raw_time.is_empty() {
        return "⚠️ 请提供24小时制时间，例如: <code>!briefing time 07:30</code>".to_string();
    }

    if let Err(e) = parse_briefing_time(raw_time) {
        return format!(
            "⚠️ 时间格式无效: {} (例如 07:00)",
            html_escape(&e.to_string())
        );
    }

    let briefing_time = raw_time.trim().to_string();
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    match settings
        .modify(&acct, |current| {
            current.briefing_time = briefing_time;
            Ok(())
        })
        .await
    {
        Ok(saved) => format!(
            "✅ 早安简报时间已设置为 <b>{}</b> (时区: <code>{}</code>)。",
            html_escape(&saved.briefing_time),
            html_escape(&saved.timezone)
        ),
        Err(err) => format!("❌ 保存失败: {}", html_escape(&err.to_string())),
    }
}

async fn toggle(settings: &SettingsService, owner_id: &str, enabled: bool) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    match settings
        .modify(&acct, |current| {
            current.briefing_enabled = enabled;
            Ok(())
        })
        .await
    {
        Ok(saved) => {
            if enabled {
                format!(
                    "✅ 每日早安简报已<b>开启</b>，将于每天 <b>{}</b> (<code>{}</code>) 发送。",
                    html_escape(&saved.briefing_time),
                    html_escape(&saved.timezone)
                )
            } else {
                "ℹ️ 每日早安简报已<b>关闭</b>。随时可输入 <code>!briefing on</code> 重新开启。"
                    .to_string()
            }
        }
        Err(err) => format!("❌ 保存失败: {}", html_escape(&err.to_string())),
    }
}

async fn show_status(service: &SettingsService, owner_id: &str) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let settings = service.get_or_default(&acct).await;
    let last_date = service
        .get_last_briefing_date(&acct)
        .await
        .unwrap_or_else(|| "尚未发送过".to_string());

    let state_label = if settings.briefing_enabled {
        "<font color=\"#10b981\">已开启</font>"
    } else {
        "<font color=\"#888888\">已关闭</font>"
    };

    format!(
        "🌅 <b>早安简报设置状态</b><br/>\
         • 状态: {}<br/>\
         • 预定时间: <b>{}</b><br/>\
         • 用户时区: <code>{}</code><br/>\
         • 最近发送日期: <code>{}</code>",
        state_label,
        html_escape(&settings.briefing_time),
        html_escape(&settings.timezone),
        html_escape(&last_date)
    )
}

fn usage() -> String {
    "<b>早安简报指令用法:</b><br/>\
     • <code>!briefing</code> / <code>!morning</code> — 立即生成并发送当前简报<br/>\
     • <code>!briefing time 07:00</code> — 设置每日定时发送时间<br/>\
     • <code>!briefing on</code> / <code>!briefing off</code> — 开启或关闭自动简报<br/>\
     • <code>!briefing status</code> — 查看当前简报配置状态"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_and_settings() -> (DbPool, SettingsService) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let settings = SettingsService::new(pool.clone(), EventBroadcaster::default());
        (pool, settings)
    }

    #[tokio::test]
    async fn test_strip_command() {
        assert_eq!(strip_command("!briefing", "!briefing"), Some(""));
        assert_eq!(
            strip_command("!briefing time 08:00", "!briefing time 08:00"),
            Some("time 08:00")
        );
        assert_eq!(strip_command("!morning", "!morning"), Some(""));
        assert_eq!(strip_command("!todo list", "!todo list"), None);
    }

    #[tokio::test]
    async fn test_briefing_status_and_time_commands() {
        let (pool, settings) = pool_and_settings().await;
        let owner = "@alice:example.org";

        let res = handle_briefing_command(&pool, &settings, owner, "!briefing status", None)
            .await
            .unwrap();
        assert!(res.contains("早安简报设置状态"));
        assert!(res.contains("07:00"));

        let res_time =
            handle_briefing_command(&pool, &settings, owner, "!briefing time 08:30", None)
                .await
                .unwrap();
        assert!(res_time.contains("08:30"));

        let res_off = handle_briefing_command(&pool, &settings, owner, "!briefing off", None)
            .await
            .unwrap();
        assert!(res_off.contains("关闭"));
    }
}
