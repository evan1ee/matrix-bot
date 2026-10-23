use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Duration, LocalResult, TimeZone, Utc};

use super::models::{BriefingPayload, BriefingSummary, TickerPerformance};
use super::ticker::{fetch_all_tickers_performance, format_ticker_html, format_ticker_plain};
use crate::application::finance::FinanceService;
use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::{BudgetState, BudgetStatus};
use crate::domain::finance::money::format_amount;
use crate::domain::habits::HabitWithStatus;
use crate::domain::recurring_bills::RecurringBill;
use crate::domain::settings::UserSettings;
use crate::domain::todos::{TodoItem, TodoPriority};
use crate::infrastructure::postgres::repositories::habits as habits_repo;
use crate::infrastructure::postgres::repositories::portfolio::holdings as portfolio_holdings_repo;
use crate::infrastructure::postgres::repositories::recurring_bills as bills_repo;
use crate::infrastructure::postgres::repositories::todos as todos_repo;
use crate::utils::html_escape;

/// Formats the local header date line (e.g. "2026-09-18 07:00 (Asia/Shanghai)").
pub fn format_briefing_header(now_utc: DateTime<Utc>, settings: &UserSettings) -> String {
    let tz = settings.tz();
    let local = now_utc.with_timezone(&tz);
    format!("{} ({})", local.format("%Y-%m-%d %H:%M"), settings.timezone)
}

/// Gathers all productivity and market data to construct the Morning Briefing.
pub async fn generate_briefing(
    pool: &DbPool,
    owner_id: &AccountId,
    settings: &UserSettings,
) -> Result<BriefingPayload> {
    let now_utc = Utc::now();
    let tz = settings.tz();
    let now_local = now_utc.with_timezone(&tz);
    let today_str = now_local.format("%Y-%m-%d").to_string();

    // 1. Portfolio holdings & multi-timeframe tickers
    let holdings = portfolio_holdings_repo::list(pool, owner_id)
        .await
        .unwrap_or_default();
    let tickers: Vec<TickerPerformance> = fetch_all_tickers_performance(&holdings).await;

    // 2. Pending tasks
    let todos = todos_repo::list_all_user(pool, owner_id, Some(false), None, 50, 0)
        .await
        .unwrap_or_default();

    // 3. Today's habits
    let habits: Vec<HabitWithStatus> =
        habits_repo::find_all_with_status(pool, owner_id, &today_str)
            .await
            .unwrap_or_default();

    // 4. Upcoming bills due in the next 3 days
    let bills = bills_repo::list_by_owner(pool, owner_id)
        .await
        .unwrap_or_default();

    let upcoming_bills = filter_upcoming_bills(&bills, now_utc);

    // 5. Budgets at or over 80 % this month (status rule lives in FinanceService)
    let budgets = match FinanceService::new(pool.clone())
        .budget_status(owner_id, None)
        .await
    {
        Ok(all) => all,
        Err(error) => {
            tracing::warn!(%error, %owner_id, "briefing: budget status failed; omitting section");
            Vec::new()
        }
    };
    let budgets = budgets_needing_attention(&budgets);

    // Build statistics summary
    let habits_completed_count = habits.iter().filter(|h| h.completed_today).count();
    let summary = BriefingSummary {
        pending_tasks_count: todos.len(),
        habits_completed_count,
        habits_total_count: habits.len(),
        upcoming_bills_count: upcoming_bills.len(),
        holdings_count: holdings.len(),
        budgets_attention_count: budgets.len(),
    };

    let (html_content, plain_text) = format_briefing_content(
        &format_briefing_header(now_utc, settings),
        &tickers,
        &todos,
        &habits,
        &upcoming_bills,
        &budgets,
        now_utc,
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

fn budgets_needing_attention(all: &[BudgetStatus]) -> Vec<&BudgetStatus> {
    all.iter().filter(|b| b.state != BudgetState::Ok).collect()
}

/// Section: budgets in Warning or Exceeded state; nothing when there are none.
fn append_budgets(html: &mut String, plain: &mut String, budgets: &[&BudgetStatus]) {
    if budgets.is_empty() {
        return;
    }
    html.push_str("<br/>💰 <b>预算提醒 · Budgets</b><br/>");
    plain.push_str(
        "
💰 预算提醒 · Budgets
",
    );
    for b in budgets {
        let limit = format_amount(b.budget.limit_minor, b.budget.currency_exponent as u32);
        let line = format!(
            "{}: {} % of {} {} used",
            b.budget.scope.label(),
            b.percent_used,
            limit,
            b.budget.currency
        );
        html.push_str(&format!("• {}<br/>", html_escape(&line)));
        plain.push_str(&format!(
            "• {line}
"
        ));
    }
}

fn format_briefing_content(
    header_date: &str,
    tickers: &[TickerPerformance],
    todos: &[TodoItem],
    habits: &[HabitWithStatus],
    bills: &[&RecurringBill],
    budgets: &[&BudgetStatus],
    now_utc: DateTime<Utc>,
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
            let is_high = item.priority == TodoPriority::High;
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

    append_budgets(&mut html, &mut plain, budgets);

    // Friendly closing
    html.push_str("<br/>祝你今天工作与生活顺利高效！✨");
    plain.push_str("\n祝你今天工作与生活顺利高效！✨\n");

    (html, plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::finance::budget::{Budget, BudgetScope, Month};

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
        let now = Utc::now();
        let tickers = vec![TickerPerformance {
            symbol: "NVDA".to_string(),
            currency: "USD".to_string(),
            current_price: 120.0,
            change_1d_pct: Some(3.5),
            change_1w_pct: Some(1.2),
            change_1m_pct: Some(15.0),
        }];

        let (html, plain) =
            format_briefing_content("2026-09-18 07:00 (UTC)", &tickers, &[], &[], &[], &[], now);

        assert!(html.contains("早安简报"));
        assert!(html.contains("NVDA"));
        assert!(html.contains("+3.5%"));
        assert!(plain.contains("NVDA"));
    }

    fn budget_status(
        label: Option<&str>,
        spent: i64,
        percent: i64,
        state: BudgetState,
    ) -> BudgetStatus {
        let now = Utc.with_ymd_and_hms(2026, 10, 15, 0, 0, 0).unwrap();
        BudgetStatus {
            budget: Budget {
                id: 1,
                owner_id: AccountId::generate(),
                scope: match label {
                    Some(name) => BudgetScope::Category {
                        id: 1,
                        name: name.to_string(),
                    },
                    None => BudgetScope::Overall,
                },
                currency: "EUR".to_string(),
                currency_exponent: 2,
                limit_minor: 30_000,
                alerts_enabled: true,
                created_at: now,
                updated_at: now,
            },
            month: Month::containing(now, chrono_tz::UTC),
            spent_minor: spent,
            remaining_minor: 30_000 - spent,
            percent_used: percent,
            state,
            pace: None,
        }
    }

    /// Given one Warning and one Ok budget, When the briefing is formatted, Then
    /// only the Warning budget gets a line.
    #[test]
    fn briefing_lists_budgets_needing_attention() {
        let budgets = vec![
            budget_status(Some("Food"), 24_600, 82, BudgetState::Warning),
            budget_status(Some("Fun"), 3_000, 10, BudgetState::Ok),
        ];
        assert_eq!(budgets_needing_attention(&budgets).len(), 1);
        let (html, plain) = format_briefing_content(
            "h",
            &[],
            &[],
            &[],
            &[],
            &budgets_needing_attention(&budgets),
            Utc::now(),
        );
        assert!(html.contains("Food: 82 % of 300.00 EUR used"));
        assert!(plain.contains("Food: 82 % of 300.00 EUR used"));
        assert!(!plain.contains("Fun"));
    }

    #[test]
    fn briefing_omits_budget_section_when_all_ok() {
        let budgets = vec![budget_status(Some("Fun"), 3_000, 10, BudgetState::Ok)];
        let attention = budgets_needing_attention(&budgets);
        assert!(attention.is_empty());
        let (html, plain) =
            format_briefing_content("h", &[], &[], &[], &[], &attention, Utc::now());
        assert!(!html.contains("Budgets"));
        assert!(!plain.contains("Budgets"));
    }
}
