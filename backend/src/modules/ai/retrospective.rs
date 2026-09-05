use std::collections::HashMap;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::modules::finance::repository::FinanceRepository;
use crate::modules::habits::repository::HabitsRepository;
use crate::modules::life::repository::LifeRepository;
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;
use crate::modules::todo::repository::TodoRepository;
use crate::utils::html::html_escape;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WeeklyRetrospective {
    pub period_start: String,
    pub period_end: String,
    pub total_spent_minor: i64,
    pub total_income_minor: i64,
    pub net_cashflow_minor: i64,
    pub currency: String,
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
    pool: &SqlitePool,
    owner_id: &str,
) -> anyhow::Result<WeeklyRetrospective> {
    let now = Utc::now();
    let today = now.date_naive();
    let seven_days_ago = today - Duration::days(7);
    let next_seven_days = today + Duration::days(7);

    let period_start = seven_days_ago.format("%Y-%m-%d").to_string();
    let period_end = today.format("%Y-%m-%d").to_string();
    let next_period_end = next_seven_days.format("%Y-%m-%d").to_string();

    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
    let currency = settings.currency.clone();

    // 1. Finance (last 7 days)
    let start_dt = seven_days_ago
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight")
        .and_utc();

    let tx_list = FinanceRepository::list(pool, owner_id, None, 200, 0).await?;

    let mut total_spent_minor: i64 = 0;
    let mut total_income_minor: i64 = 0;
    let mut category_totals: HashMap<String, i64> = HashMap::new();

    for tx in &tx_list {
        if tx.occurred_at >= start_dt {
            if tx.direction == "expense" {
                total_spent_minor += tx.amount_minor;
                let cat = tx.category_name.clone().unwrap_or_else(|| "Uncategorized".to_string());
                *category_totals.entry(cat).or_insert(0) += tx.amount_minor;
            } else if tx.direction == "income" {
                total_income_minor += tx.amount_minor;
            }
        }
    }

    let top_expense_category = category_totals
        .into_iter()
        .max_by_key(|(_, amount)| *amount)
        .map(|(cat, _)| cat);

    let net_cashflow_minor = total_income_minor - total_spent_minor;

    // 2. Tasks
    let all_tasks = TodoRepository::list_all_user(pool, owner_id, None, 200, 0).await?;
    let completed_tasks_count = all_tasks.iter().filter(|t| t.completed).count();
    let active_tasks_count = all_tasks.iter().filter(|t| !t.completed).count();

    // 3. Habits
    let habits_with_status = HabitsRepository::find_all_with_status(pool, owner_id, &period_end).await?;
    let active_habits_count = habits_with_status.len();
    let mut total_possible_habit_days = 0;
    let mut total_completed_habit_days = 0;

    for h in &habits_with_status {
        total_possible_habit_days += 7;
        let completed_in_week = h.last_7_days.iter().filter(|d| d.completed).count();
        total_completed_habit_days += completed_in_week;
    }

    let habits_adherence_percent = if total_possible_habit_days > 0 {
        ((total_completed_habit_days as f64) / (total_possible_habit_days as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    // 4. Upcoming Bills (next 7 days)
    let bills = RecurringBillRepository::list_by_owner(pool, owner_id).await?;
    let mut upcoming_bills_count = 0;
    let mut upcoming_bills_total_minor: i64 = 0;

    for b in &bills {
        if b.status == "active"
            && b.next_due_at.as_str() <= next_period_end.as_str()
        {
            upcoming_bills_count += 1;
            upcoming_bills_total_minor += b.amount_minor;
        }
    }

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
        let d = (seven_days_ago + Duration::days(i)).format("%Y-%m-%d").to_string();
        if let Ok(Some(log)) = LifeRepository::get_day_log(pool, owner_id, &d).await {
            days_logged_count += 1;
            *mood_counts.entry(log.mood.to_lowercase()).or_insert(0) += 1;
        }
    }

    let dominant_mood = mood_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(m, _)| m);

    // 7. Format Markdown
    let spent_dec = format!("{:.2}", (total_spent_minor as f64) / 100.0);
    let income_dec = format!("{:.2}", (total_income_minor as f64) / 100.0);
    let net_dec = format!("{:.2}", (net_cashflow_minor as f64) / 100.0);
    let bills_dec = format!("{:.2}", (upcoming_bills_total_minor as f64) / 100.0);

    let top_cat_display = top_expense_category.as_deref().unwrap_or("None");
    let mood_display = dominant_mood.as_deref().unwrap_or("Unrecorded");

    let formatted_markdown = format!(
        "### 🌟 Weekly Executive Retrospective ({period_start} to {period_end})\n\n\
         **💰 Financial Velocity**\n\
         - **Total Spent**: `{currency} {spent_dec}`\n\
         - **Total Income**: `{currency} {income_dec}`\n\
         - **Net Cashflow**: `{currency} {net_dec}`\n\
         - **Top Category**: {top_cat_display}\n\n\
         **📋 Productivity & Habits**\n\
         - **Tasks Completed**: {completed_tasks_count} (Active remaining: {active_tasks_count})\n\
         - **Habit Adherence**: {habits_adherence_percent}% across {active_habits_count} tracked habits\n\n\
         **🧬 Life & Wellness**\n\
         - **Days Logged**: {days_logged_count} of 7 days\n\
         - **Dominant Mood**: {mood_display}\n\n\
         **📅 Upcoming Focus (Next 7 Days)**\n\
         - **Due Bills**: {upcoming_bills_count} bills (`{currency} {bills_dec}`)\n\
         - **Pending Reminders**: {upcoming_reminders_count} reminders scheduled\n"
    );

    Ok(WeeklyRetrospective {
        period_start,
        period_end,
        total_spent_minor,
        total_income_minor,
        net_cashflow_minor,
        currency,
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

pub async fn handle_retrospective_command(pool: &SqlitePool, owner_id: &str) -> String {
    match generate_weekly_retrospective(pool, owner_id).await {
        Ok(retro) => {
            let spent_dec = format!("{:.2}", (retro.total_spent_minor as f64) / 100.0);
            let income_dec = format!("{:.2}", (retro.total_income_minor as f64) / 100.0);
            let net_dec = format!("{:.2}", (retro.net_cashflow_minor as f64) / 100.0);
            let bills_dec = format!("{:.2}", (retro.upcoming_bills_total_minor as f64) / 100.0);
            let top_cat = html_escape(retro.top_expense_category.as_deref().unwrap_or("None"));
            let mood = html_escape(retro.dominant_mood.as_deref().unwrap_or("Unrecorded"));

            let html = format!(
                "<h3>🌟 Weekly Executive Retrospective</h3>\
                 <p><strong>Period:</strong> {period_start} to {period_end}</p>\
                 <h4>💰 Financial Velocity</h4>\
                 <ul>\
                   <li><strong>Total Spent:</strong> <code>{curr} {spent_dec}</code></li>\
                   <li><strong>Total Income:</strong> <code>{curr} {income_dec}</code></li>\
                   <li><strong>Net Flow:</strong> <code>{curr} {net_dec}</code></li>\
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
                   <li><strong>Due Bills:</strong> {bills_cnt} (<code>{curr} {bills_dec}</code>)</li>\
                   <li><strong>Pending Reminders:</strong> {reminders_cnt}</li>\
                 </ul>",
                period_start = html_escape(&retro.period_start),
                period_end = html_escape(&retro.period_end),
                curr = html_escape(&retro.currency),
                spent_dec = spent_dec,
                income_dec = income_dec,
                net_dec = net_dec,
                top_cat = top_cat,
                completed = retro.completed_tasks_count,
                active = retro.active_tasks_count,
                adherence = retro.habits_adherence_percent,
                habits_cnt = retro.active_habits_count,
                days_logged = retro.days_logged_count,
                mood = mood,
                bills_cnt = retro.upcoming_bills_count,
                bills_dec = bills_dec,
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
