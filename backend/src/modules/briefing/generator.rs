use anyhow::Result;
use chrono::{DateTime, Duration, LocalResult, TimeZone, Utc};
use sqlx::SqlitePool;

use crate::modules::briefing::models::{BriefingPayload, BriefingSummary, TickerPerformance};
use crate::modules::briefing::ticker::{
    fetch_all_tickers_performance, format_ticker_html, format_ticker_plain,
};
use crate::modules::habits::models::HabitWithStatus;
use crate::modules::habits::repository::HabitsRepository;
use crate::modules::portfolio::repository::PortfolioRepository;
use crate::modules::recurring_bills::models::RecurringBill;
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::settings::UserSettings;
use crate::modules::todo::models::TodoItem;
use crate::modules::todo::repository::TodoRepository;
use crate::utils::html_escape;

/// Formats the local header date line (e.g. "2026-09-18 07:00 (Asia/Shanghai)").
pub fn format_briefing_header(now_utc: DateTime<Utc>, settings: &UserSettings) -> String {
    let tz = settings.tz();
    let local = now_utc.with_timezone(&tz);
    format!("{} ({})", local.format("%Y-%m-%d %H:%M"), settings.timezone)
}

/// Gathers all productivity and market data to construct the Morning Briefing.
pub async fn generate_briefing(
    pool: &SqlitePool,
    owner_id: &str,
    settings: &UserSettings,
) -> Result<BriefingPayload> {
    let now_utc = Utc::now();
    let tz = settings.tz();
    let now_local = now_utc.with_timezone(&tz);
    let today_str = now_local.format("%Y-%m-%d").to_string();

    // 1. Portfolio holdings & multi-timeframe tickers
    let holdings = PortfolioRepository::list(pool, owner_id)
        .await
        .unwrap_or_default();
    let tickers: Vec<TickerPerformance> = fetch_all_tickers_performance(&holdings).await;

    // 2. Pending tasks
    let todos = TodoRepository::list_all_user(pool, owner_id, Some(false), 50, 0)
        .await
        .unwrap_or_default();

    // 3. Today's habits
    let habits: Vec<HabitWithStatus> =
        HabitsRepository::find_all_with_status(pool, owner_id, &today_str)
            .await
            .unwrap_or_default();

    // 4. Upcoming bills due in the next 3 days
    let bills = RecurringBillRepository::list_by_owner(pool, owner_id)
        .await
        .unwrap_or_default();

    let upcoming_bills = filter_upcoming_bills(&bills, now_utc);

    // Build statistics summary
    let habits_completed_count = habits.iter().filter(|h| h.completed_today).count();
    let summary = BriefingSummary {
        pending_tasks_count: todos.len(),
        habits_completed_count,
        habits_total_count: habits.len(),
        upcoming_bills_count: upcoming_bills.len(),
        holdings_count: holdings.len(),
    };

    let (html_content, plain_text) = format_briefing_content(
        &format_briefing_header(now_utc, settings),
        &tickers,
        &todos,
        &habits,
        &upcoming_bills,
        now_utc,
        settings,
    );

    Ok(BriefingPayload {
        owner_id: owner_id.to_string(),
        html_content,
        plain_text,
        summary,
        tickers,
    })
}

fn filter_upcoming_bills(bills: &[RecurringBill], now: DateTime<Utc>) -> Vec<&RecurringBill> {
    let cutoff = now + Duration::days(3);
    bills
        .iter()
        .filter(|b| b.status == "active")
        .filter(|b| {
            if let Ok(due) =
                chrono::NaiveDateTime::parse_from_str(&b.next_due_at, "%Y-%m-%d %H:%M:%S")
            {
                let due_utc = Utc.from_utc_datetime(&due);
                due_utc <= cutoff
            } else if let Ok(due_d) = chrono::NaiveDate::parse_from_str(&b.next_due_at, "%Y-%m-%d")
            {
                if let LocalResult::Single(due_utc) =
                    Utc.from_local_datetime(&due_d.and_hms_opt(23, 59, 59).unwrap())
                {
                    due_utc <= cutoff
                } else {
                    false
                }
            } else {
                false
            }
        })
        .collect()
}

fn format_briefing_content(
    header_date: &str,
    tickers: &[TickerPerformance],
    todos: &[TodoItem],
    habits: &[HabitWithStatus],
    bills: &[&RecurringBill],
    now_utc: DateTime<Utc>,
    _settings: &UserSettings,
) -> (String, String) {
    let mut html = String::with_capacity(1024);
    let mut plain = String::with_capacity(1024);

    // Title & timestamp
    html.push_str("🌅 <b>早安简报 · Morning Briefing</b><br/>");
    html.push_str(&format!(
        "<font color=\"#888888\">{}</font><br/>",
        html_escape(header_date)
    ));

    plain.push_str("🌅 早安简报 · Morning Briefing\n");
    plain.push_str(&format!("{}\n", header_date));

    // Section: Portfolio Tickers
    if !tickers.is_empty() {
        html.push_str("<br/>📈 <b>持仓资产速报 · Portfolio</b><br/>");
        plain.push_str("\n📈 持仓资产速报 · Portfolio\n");
        for t in tickers {
            html.push_str(&format_ticker_html(t));
            html.push_str("<br/>");
            plain.push_str(&format_ticker_plain(t));
            plain.push('\n');
        }
    }

    // Section: Tasks
    if !todos.is_empty() {
        html.push_str(&format!(
            "<br/>📋 <b>今日待办 · Tasks ({} 项待处理)</b><br/>",
            todos.len()
        ));
        plain.push_str(&format!(
            "\n📋 今日待办 · Tasks ({} 项待处理)\n",
            todos.len()
        ));

        // Display up to 6 prioritized tasks
        for item in todos.iter().take(6) {
            let is_high = item.priority.eq_ignore_ascii_case("high")
                || item.priority.eq_ignore_ascii_case("h");
            let pri_badge = if is_high {
                "<font color=\"#ef4444\">[高优]</font> "
            } else {
                ""
            };
            let pri_plain = if is_high { "[高优] " } else { "" };

            let due_suffix = if let Some(due) = item.due_at {
                if due < now_utc {
                    " <font color=\"#ef4444\">(已逾期)</font>"
                } else if due - now_utc < Duration::hours(24) {
                    " <font color=\"#f59e0b\">(今日截止)</font>"
                } else {
                    ""
                }
            } else {
                ""
            };

            html.push_str(&format!(
                "• {}{}{}<br/>",
                pri_badge,
                html_escape(&item.text),
                due_suffix
            ));
            plain.push_str(&format!("• {}{}\n", pri_plain, item.text));
        }

        if todos.len() > 6 {
            html.push_str(&format!(
                "<font color=\"#888888\">... 还有 {} 项待办</font><br/>",
                todos.len() - 6
            ));
            plain.push_str(&format!("... 还有 {} 项待办\n", todos.len() - 6));
        }
    }

    // Section: Habits
    if !habits.is_empty() {
        let completed_count = habits.iter().filter(|h| h.completed_today).count();
        html.push_str(&format!(
            "<br/>🔁 <b>习惯打卡 · Habits ({}/{} 已完成)</b><br/>",
            completed_count,
            habits.len()
        ));
        plain.push_str(&format!(
            "\n🔁 习惯打卡 · Habits ({}/{} 已完成)\n",
            completed_count,
            habits.len()
        ));

        for h in habits.iter().take(6) {
            let icon = if h.completed_today { "✅" } else { "⬜" };
            let streak_str = if h.current_streak > 0 {
                format!(" (连胜 {} 天)", h.current_streak)
            } else {
                String::new()
            };

            html.push_str(&format!(
                "• {} {}{}<br/>",
                icon,
                html_escape(&h.habit.name),
                streak_str
            ));
            plain.push_str(&format!("• {} {}{}\n", icon, h.habit.name, streak_str));
        }
    }

    // Section: Upcoming Bills
    if !bills.is_empty() {
        html.push_str("<br/>⚠️ <b>近期账单 · Upcoming Bills</b><br/>");
        plain.push_str("\n⚠️ 近期账单 · Upcoming Bills\n");

        for b in bills {
            let amount = (b.amount_minor as f64) / 100.0;
            html.push_str(&format!(
                "• {}: {:.2} {} (到期: {})<br/>",
                html_escape(&b.name),
                amount,
                b.currency,
                b.next_due_at
            ));
            plain.push_str(&format!(
                "• {}: {:.2} {} (到期: {})\n",
                b.name, amount, b.currency, b.next_due_at
            ));
        }
    }

    // Friendly closing
    html.push_str("<br/>祝你今天工作与生活顺利高效！✨");
    plain.push_str("\n祝你今天工作与生活顺利高效！✨\n");

    (html, plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_briefing_header() {
        let t = Utc.with_ymd_and_hms(2026, 9, 18, 7, 0, 0).unwrap();
        let settings = UserSettings {
            timezone: "UTC".to_string(),
            ..UserSettings::default()
        };
        let header = format_briefing_header(t, &settings);
        assert!(header.contains("2026-09-18 07:00"));
        assert!(header.contains("UTC"));
    }

    #[test]
    fn test_format_briefing_content_structure() {
        let settings = UserSettings::default();
        let now = Utc::now();
        let tickers = vec![TickerPerformance {
            symbol: "NVDA".to_string(),
            currency: "USD".to_string(),
            current_price: 120.0,
            change_1d_pct: Some(3.5),
            change_1w_pct: Some(1.2),
            change_1m_pct: Some(15.0),
        }];

        let (html, plain) = format_briefing_content(
            "2026-09-18 07:00 (UTC)",
            &tickers,
            &[],
            &[],
            &[],
            now,
            &settings,
        );

        assert!(html.contains("早安简报"));
        assert!(html.contains("NVDA"));
        assert!(html.contains("+3.5%"));
        assert!(plain.contains("NVDA"));
    }
}
