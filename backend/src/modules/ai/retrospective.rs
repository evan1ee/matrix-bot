use crate::db::DbPool;
use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::application::settings::SettingsService;
use crate::modules::finance::money::format_amount;
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::habits::repository::HabitsRepository;
use crate::modules::life::repository::LifeRepository;
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::todo::repository::TodoRepository;
use crate::utils::html::html_escape;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CurrencySpendSummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub spent_minor: i64,
    pub income_minor: i64,
    pub net_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpcomingBillsSummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub total_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WeeklyRetrospective {
    pub period_start: String,
    pub period_end: String,
    pub total_spent_minor: i64,
    pub total_income_minor: i64,
    pub net_cashflow_minor: i64,
    pub currency: String,
    pub spends_by_currency: Vec<CurrencySpendSummary>,
    pub upcoming_bills_by_currency: Vec<UpcomingBillsSummary>,
    pub formatted_spent: String,
    pub formatted_income: String,
    pub formatted_net: String,
    pub formatted_upcoming_bills: String,
    pub top_expense_category: Option<String>,
    pub completed_tasks_count: usize,
    pub active_tasks_count: usize,
    pub habits_adherence_percent: f64,
    pub active_habits_count: usize,
    pub upcoming_bills_count: usize,
    pub upcoming_bills_total_minor: i64,
    pub upcoming_reminders_count: usize,
    pub days_logged_count: usize,
    pub dominant_mood: Option<String>,
    pub formatted_markdown: String,
}

pub async fn generate_weekly_retrospective(
    pool: &DbPool,
    settings: &SettingsService,
    owner_id: &str,
) -> anyhow::Result<WeeklyRetrospective> {
    let settings = settings.get_or_default(owner_id).await;
    let tz = settings.tz();
    let default_currency = settings.currency.clone();

    let now = Utc::now();
    let today = now.with_timezone(&tz).date_naive();
    let seven_days_ago = today - Duration::days(7);
    let next_seven_days = today + Duration::days(7);

    let period_start = seven_days_ago.format("%Y-%m-%d").to_string();
    let period_end = today.format("%Y-%m-%d").to_string();
    let next_period_end = next_seven_days.format("%Y-%m-%d").to_string();

    // 1. Finance (last 7 days) — Currencies must NEVER be added together.
    let start_dt = seven_days_ago
        .and_hms_opt(0, 0, 0)
        .and_then(|naive_dt| tz.from_local_datetime(&naive_dt).single())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| {
            seven_days_ago
                .and_hms_opt(0, 0, 0)
                .expect("valid midnight")
                .and_utc()
        });

    let summaries = FinanceRepository::summary(pool, owner_id, start_dt, now).await?;

    let mut spends_by_currency: Vec<CurrencySpendSummary> = summaries
        .iter()
        .map(|s| CurrencySpendSummary {
            currency: s.currency.clone(),
            currency_exponent: s.currency_exponent,
            spent_minor: s.expense_minor,
            income_minor: s.income_minor,
            net_minor: s.net_minor,
        })
        .collect();
    spends_by_currency.sort_by(|a, b| a.currency.cmp(&b.currency));

    let (currency, total_spent_minor, total_income_minor, net_cashflow_minor) =
        if let Some(primary) = spends_by_currency
            .iter()
            .find(|s| s.currency == default_currency)
        {
            (
                default_currency.clone(),
                primary.spent_minor,
                primary.income_minor,
                primary.net_minor,
            )
        } else if let Some(first) = spends_by_currency.first() {
            (
                first.currency.clone(),
                first.spent_minor,
                first.income_minor,
                first.net_minor,
            )
        } else {
            (default_currency.clone(), 0, 0, 0)
        };

    let formatted_spent = {
        let non_zero: Vec<String> = spends_by_currency
            .iter()
            .filter(|s| s.spent_minor > 0)
            .map(|s| {
                let dec = format_amount(s.spent_minor, s.currency_exponent as u32);
                format!("{} {}", s.currency, dec)
            })
            .collect();
        if non_zero.is_empty() {
            format!("{} 0.00", currency)
        } else {
            non_zero.join(", ")
        }
    };

    let formatted_income = {
        let non_zero: Vec<String> = spends_by_currency
            .iter()
            .filter(|s| s.income_minor > 0)
            .map(|s| {
                let dec = format_amount(s.income_minor, s.currency_exponent as u32);
                format!("{} {}", s.currency, dec)
            })
            .collect();
        if non_zero.is_empty() {
            format!("{} 0.00", currency)
        } else {
            non_zero.join(", ")
        }
    };

    let formatted_net = {
        let active: Vec<String> = spends_by_currency
            .iter()
            .filter(|s| s.spent_minor > 0 || s.income_minor > 0)
            .map(|s| {
                let dec = format_amount(s.net_minor, s.currency_exponent as u32);
                format!("{} {}", s.currency, dec)
            })
            .collect();
        if active.is_empty() {
            format!("{} 0.00", currency)
        } else {
            active.join(", ")
        }
    };

    // Find top category without cross-currency addition.
    let top_expense_category = {
        let mut all_cats: Vec<(&str, i64, &str, i64)> = Vec::new();
        for s in &summaries {
            for c in &s.categories {
                if c.expense_minor > 0 {
                    let name = c.category.as_deref().unwrap_or("Uncategorized");
                    all_cats.push((name, c.expense_minor, &s.currency, s.currency_exponent));
                }
            }
        }
        all_cats.sort_by(|a, b| {
            let a_is_default = a.2 == default_currency;
            let b_is_default = b.2 == default_currency;
            match (a_is_default, b_is_default) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.1.cmp(&a.1),
            }
        });
        all_cats.first().map(|(name, amount, curr, exp)| {
            format!(
                "{} ({} {})",
                name,
                curr,
                format_amount(*amount, *exp as u32)
            )
        })
    };

    // 2. Tasks
    let all_tasks = TodoRepository::list_all_user(pool, owner_id, None, 200, 0).await?;
    let completed_tasks_count = all_tasks.iter().filter(|t| t.completed).count();
    let active_tasks_count = all_tasks.iter().filter(|t| !t.completed).count();

    // 3. Habits
    let habits_with_status =
        HabitsRepository::find_all_with_status(pool, owner_id, &period_end).await?;
    let active_habits_count = habits_with_status.len();
    let mut total_possible_habit_days = 0;
    let mut total_completed_habit_days = 0;

    for h in &habits_with_status {
        total_possible_habit_days += 7;
        let completed_in_week = h.last_7_days.iter().filter(|d| d.completed).count();
        total_completed_habit_days += completed_in_week;
    }

    let habits_adherence_percent = if total_possible_habit_days > 0 {
        ((total_completed_habit_days as f64) / (total_possible_habit_days as f64) * 1000.0).round()
            / 10.0
    } else {
        0.0
    };

    // 4. Upcoming Bills (next 7 days) — Group by currency, never sum across currencies.
    let bills = RecurringBillRepository::list_by_owner(pool, owner_id).await?;
    let mut upcoming_bills_count = 0;
    let mut bills_by_currency: HashMap<String, (i64, i64)> = HashMap::new();

    for b in &bills {
        if b.status == "active" && b.next_due_at.as_str() <= next_period_end.as_str() {
            upcoming_bills_count += 1;
            let entry = bills_by_currency
                .entry(b.currency.to_uppercase())
                .or_insert((0, b.currency_exponent as i64));
            entry.0 += b.amount_minor;
        }
    }

    let mut upcoming_bills_by_currency: Vec<UpcomingBillsSummary> = bills_by_currency
        .into_iter()
        .map(|(curr, (total_minor, exp))| UpcomingBillsSummary {
            currency: curr,
            currency_exponent: exp,
            total_minor,
        })
        .collect();
    upcoming_bills_by_currency.sort_by(|a, b| a.currency.cmp(&b.currency));

    let upcoming_bills_total_minor = if let Some(primary_bill) = upcoming_bills_by_currency
        .iter()
        .find(|b| b.currency == default_currency)
    {
        primary_bill.total_minor
    } else if let Some(first_bill) = upcoming_bills_by_currency.first() {
        first_bill.total_minor
    } else {
        0
    };

    let formatted_upcoming_bills = if upcoming_bills_by_currency.is_empty() {
        format!("{} 0.00", default_currency)
    } else {
        upcoming_bills_by_currency
            .iter()
            .map(|b| {
                let dec = format_amount(b.total_minor, b.currency_exponent as u32);
                format!("{} {}", b.currency, dec)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    // 5. Upcoming Reminders (next 7 days)
    let reminders = ReminderRepository::list_user_pending(pool, owner_id, 50, 0).await?;
    let upcoming_reminders_count = reminders
        .iter()
        .filter(|r| r.trigger_at.to_rfc3339().as_str() <= next_period_end.as_str())
        .count();

    // 6. Life & Wellness (last 7 days)
    let mut days_logged_count = 0;
    let mut mood_counts: HashMap<String, usize> = HashMap::new();

    for i in 0..7 {
        let d = (seven_days_ago + Duration::days(i))
            .format("%Y-%m-%d")
            .to_string();
        if let Ok(Some(log)) = LifeRepository::get_day_log(pool, owner_id, &d).await {
            days_logged_count += 1;
            *mood_counts.entry(log.mood.to_lowercase()).or_insert(0) += 1;
        }
    }

    let dominant_mood = mood_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(m, _)| m);

    // 7. Format Markdown & HTML
    let top_cat_display = top_expense_category.as_deref().unwrap_or("None");
    let mood_display = dominant_mood.as_deref().unwrap_or("Unrecorded");

    let formatted_markdown = format!(
        "### 🌟 Weekly Executive Retrospective ({period_start} to {period_end})\n\n\
         **💰 Financial Velocity**\n\
         - **Total Spent**: `{formatted_spent}`\n\
         - **Total Income**: `{formatted_income}`\n\
         - **Net Cashflow**: `{formatted_net}`\n\
         - **Top Category**: {top_cat_display}\n\n\
         **📋 Productivity & Habits**\n\
         - **Tasks Completed**: {completed_tasks_count} (Active remaining: {active_tasks_count})\n\
         - **Habit Adherence**: {habits_adherence_percent}% across {active_habits_count} tracked habits\n\n\
         **🧬 Life & Wellness**\n\
         - **Days Logged**: {days_logged_count} of 7 days\n\
         - **Dominant Mood**: {mood_display}\n\n\
         **📅 Upcoming Focus (Next 7 Days)**\n\
         - **Due Bills**: {upcoming_bills_count} bills (`{formatted_upcoming_bills}`)\n\
         - **Pending Reminders**: {upcoming_reminders_count} reminders scheduled\n"
    );

    Ok(WeeklyRetrospective {
        period_start,
        period_end,
        total_spent_minor,
        total_income_minor,
        net_cashflow_minor,
        currency,
        spends_by_currency,
        upcoming_bills_by_currency,
        formatted_spent,
        formatted_income,
        formatted_net,
        formatted_upcoming_bills,
        top_expense_category,
        completed_tasks_count,
        active_tasks_count,
        habits_adherence_percent,
        active_habits_count,
        upcoming_bills_count,
        upcoming_bills_total_minor,
        upcoming_reminders_count,
        days_logged_count,
        dominant_mood,
        formatted_markdown,
    })
}

pub async fn handle_retrospective_command(
    pool: &DbPool,
    settings: &SettingsService,
    owner_id: &str,
) -> String {
    match generate_weekly_retrospective(pool, settings, owner_id).await {
        Ok(retro) => {
            let top_cat = html_escape(retro.top_expense_category.as_deref().unwrap_or("None"));
            let mood = html_escape(retro.dominant_mood.as_deref().unwrap_or("Unrecorded"));
            let formatted_spent = html_escape(&retro.formatted_spent);
            let formatted_income = html_escape(&retro.formatted_income);
            let formatted_net = html_escape(&retro.formatted_net);
            let formatted_bills = html_escape(&retro.formatted_upcoming_bills);

            let html = format!(
                "<h3>🌟 Weekly Executive Retrospective</h3>\
                 <p><strong>Period:</strong> {period_start} to {period_end}</p>\
                 <h4>💰 Financial Velocity</h4>\
                 <ul>\
                   <li><strong>Total Spent:</strong> <code>{formatted_spent}</code></li>\
                   <li><strong>Total Income:</strong> <code>{formatted_income}</code></li>\
                   <li><strong>Net Flow:</strong> <code>{formatted_net}</code></li>\
                   <li><strong>Top Category:</strong> {top_cat}</li>\
                 </ul>\
                 <h4>📋 Productivity & Habits</h4>\
                 <ul>\
                   <li><strong>Tasks Completed:</strong> {completed} (Active remaining: {active})</li>\
                   <li><strong>Habit Adherence:</strong> {adherence}% ({habits_cnt} active habits)</li>\
                 </ul>\
                 <h4>🧬 Life & Wellness</h4>\
                 <ul>\
                   <li><strong>Days Logged:</strong> {days_logged}/7</li>\
                   <li><strong>Dominant Mood:</strong> {mood}</li>\
                 </ul>\
                 <h4>📅 Upcoming Focus (Next 7 Days)</h4>\
                 <ul>\
                   <li><strong>Due Bills:</strong> {bills_cnt} (<code>{formatted_bills}</code>)</li>\
                   <li><strong>Pending Reminders:</strong> {reminders_cnt}</li>\
                 </ul>",
                period_start = html_escape(&retro.period_start),
                period_end = html_escape(&retro.period_end),
                formatted_spent = formatted_spent,
                formatted_income = formatted_income,
                formatted_net = formatted_net,
                top_cat = top_cat,
                completed = retro.completed_tasks_count,
                active = retro.active_tasks_count,
                adherence = retro.habits_adherence_percent,
                habits_cnt = retro.active_habits_count,
                days_logged = retro.days_logged_count,
                mood = mood,
                bills_cnt = retro.upcoming_bills_count,
                formatted_bills = formatted_bills,
                reminders_cnt = retro.upcoming_reminders_count,
            );

            format!(
                "{}\n\n<!-- html_marker -->{}",
                retro.formatted_markdown, html
            )
        }
        Err(err) => {
            format!("Failed to compile weekly retrospective: {}", err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::UserSettings;
    use crate::events::EventBroadcaster;
    use crate::modules::finance::repository::FinanceRepository;
    use crate::modules::recurring_bills::models::CreateBillPayload;
    use crate::modules::recurring_bills::repository::RecurringBillRepository;

    async fn test_db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_retrospective_keeps_currencies_strictly_apart() {
        let pool = test_db().await;
        let owner = "@user:example.com";
        let settings = SettingsService::new(pool.clone(), EventBroadcaster::default());

        // User sets default currency to CNY
        settings
            .save(
                owner,
                UserSettings {
                    timezone: "Asia/Shanghai".to_string(),
                    currency: "CNY".to_string(),
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();

        // Add expense: Shake Shack lunch: 16.50 USD (1650 minor)
        let cat_food = FinanceRepository::create_category(&pool, owner, "Food")
            .await
            .unwrap()
            .unwrap();
        FinanceRepository::create_transaction(
            &pool,
            "room1",
            owner,
            "User",
            "expense",
            1650,
            "USD",
            2,
            Some(cat_food.id),
            Some("Lunch at Shake Shack"),
            None,
            None,
            Utc::now(),
        )
        .await
        .unwrap();

        // Add expense: Groceries: 45.00 CNY (4500 minor)
        let cat_groceries = FinanceRepository::create_category(&pool, owner, "Groceries")
            .await
            .unwrap()
            .unwrap();
        FinanceRepository::create_transaction(
            &pool,
            "room1",
            owner,
            "User",
            "expense",
            4500,
            "CNY",
            2,
            Some(cat_groceries.id),
            Some("groceries"),
            None,
            None,
            Utc::now(),
        )
        .await
        .unwrap();

        // Add upcoming bills in USD and CNY
        let tomorrow = (Utc::now() + Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        RecurringBillRepository::create(
            &pool,
            "room1",
            owner,
            "User",
            CreateBillPayload {
                name: "US Service".to_string(),
                category: "subscription".to_string(),
                amount_minor: 1000,
                currency: "USD".to_string(),
                currency_exponent: Some(2),
                frequency: Some("monthly".to_string()),
                due_day: Some(1),
                next_due_at: tomorrow.clone(),
                total_installments: None,
                auto_pay: Some(false),
                notes: None,
                remind_before_minutes: Some(60),
                remind_room_id: None,
            },
        )
        .await
        .unwrap();

        RecurringBillRepository::create(
            &pool,
            "room1",
            owner,
            "User",
            CreateBillPayload {
                name: "CN Service".to_string(),
                category: "subscription".to_string(),
                amount_minor: 3000,
                currency: "CNY".to_string(),
                currency_exponent: Some(2),
                frequency: Some("monthly".to_string()),
                due_day: Some(1),
                next_due_at: tomorrow,
                total_installments: None,
                auto_pay: Some(false),
                notes: None,
                remind_before_minutes: Some(60),
                remind_room_id: None,
            },
        )
        .await
        .unwrap();

        let retro = generate_weekly_retrospective(&pool, &settings, owner)
            .await
            .unwrap();

        // Primary currency is CNY, total_spent_minor must ONLY reflect CNY (4500), NEVER 6150 (1650 + 4500)
        assert_eq!(retro.currency, "CNY");
        assert_eq!(retro.total_spent_minor, 4500);
        assert_eq!(retro.net_cashflow_minor, -4500);

        // spends_by_currency must separate both currencies
        assert_eq!(retro.spends_by_currency.len(), 2);
        let cny = retro
            .spends_by_currency
            .iter()
            .find(|s| s.currency == "CNY")
            .unwrap();
        assert_eq!(cny.spent_minor, 4500);
        let usd = retro
            .spends_by_currency
            .iter()
            .find(|s| s.currency == "USD")
            .unwrap();
        assert_eq!(usd.spent_minor, 1650);

        // Formatted spent must state both currencies explicitly without summing
        assert!(retro.formatted_spent.contains("CNY 45.00"));
        assert!(retro.formatted_spent.contains("USD 16.50"));
        assert!(!retro.formatted_spent.contains("61.50"));

        // Markdown must show formatted spent and upcoming bills without summing
        assert!(retro
            .formatted_markdown
            .contains("**Total Spent**: `CNY 45.00, USD 16.50`"));
        assert!(retro
            .formatted_markdown
            .contains("Due Bills**: 2 bills (`CNY 30.00, USD 10.00`)"));

        // Top category must show category and currency without cross-currency addition
        assert!(retro
            .top_expense_category
            .as_ref()
            .unwrap()
            .contains("Groceries (CNY 45.00)"));

        // Upcoming bills count is 2, and upcoming_bills_by_currency keeps them apart
        assert_eq!(retro.upcoming_bills_count, 2);
        assert_eq!(retro.upcoming_bills_by_currency.len(), 2);
    }
}
