//! Cross-feature read models for Life: Horizon days and aggregated day activity.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::AccountId;
use crate::domain::life::{ActivityEvent, ActivityStats, DayActivityResponse, LifeHorizonDay};

fn extract_time(dt: &Option<DateTime<Utc>>) -> Option<String> {
    dt.map(|d| d.format("%H:%M:%S").to_string())
}

fn extract_time_from_str(s: &Option<String>) -> Option<String> {
    s.as_deref().and_then(|raw| {
        if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
            Some(dt.format("%H:%M:%S").to_string())
        } else if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") {
            Some(naive.format("%H:%M:%S").to_string())
        } else if let Some(time_part) = raw.split_whitespace().nth(1) {
            Some(time_part[..8.min(time_part.len())].to_string())
        } else if let Some(time_part) = raw.split('T').nth(1) {
            Some(time_part[..8.min(time_part.len())].to_string())
        } else {
            None
        }
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn push_event(
    events: &mut Vec<ActivityEvent>,
    time: Option<String>,
    module: &'static str,
    action: impl Into<String>,
    emoji: &'static str,
    summary: String,
    detail: Option<String>,
) {
    events.push(ActivityEvent {
        time,
        module: module.to_string(),
        action: action.into(),
        emoji: emoji.to_string(),
        summary,
        detail,
    });
}

/// Gathers the multi-year life horizon grid days, combining life day logs with
/// activity timestamps across todos, transactions, portfolio, habits, reminders, bookmarks, and footprints.
pub async fn get_horizon_days(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<Vec<LifeHorizonDay>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        WITH user_activities AS (
            SELECT CAST(date(created_at) AS TEXT) AS act_date FROM todos WHERE owner_id = $1 AND created_at IS NOT NULL
            UNION ALL
            SELECT CAST(date(due_at) AS TEXT) AS act_date FROM todos WHERE owner_id = $1 AND due_at IS NOT NULL
            UNION ALL
            SELECT CAST(date(occurred_at) AS TEXT) AS act_date FROM transactions WHERE owner_id = $1 AND occurred_at IS NOT NULL
            UNION ALL
            SELECT CAST(date(created_at) AS TEXT) AS act_date FROM portfolio_audit_log WHERE owner_id = $1 AND created_at IS NOT NULL
            UNION ALL
            SELECT CAST(log_date AS TEXT) AS act_date FROM habit_logs WHERE owner_id = $1 AND log_date IS NOT NULL
            UNION ALL
            SELECT CAST(date(trigger_at) AS TEXT) AS act_date FROM reminders WHERE owner_id = $1 AND trigger_at IS NOT NULL
            UNION ALL
            SELECT CAST(date(created_at) AS TEXT) AS act_date FROM bookmarks WHERE owner_id = $1 AND created_at IS NOT NULL
            UNION ALL
            SELECT CAST(arrival_date AS TEXT) AS act_date FROM footprints WHERE owner_id = $1 AND arrival_date IS NOT NULL
        ),
        activity_counts AS (
            SELECT act_date, COUNT(*) AS cnt 
            FROM user_activities 
            WHERE act_date IS NOT NULL AND act_date != ''
            GROUP BY act_date
        ),
        all_dates AS (
            SELECT log_date AS d_date FROM life_day_logs WHERE owner_id = $1
            UNION
            SELECT act_date AS d_date FROM activity_counts
        )
        SELECT 
            all_dates.d_date AS "date!",
            COALESCE(l.mood, '') AS "mood!",
            l.time_allocation AS "time_allocation",
            (COALESCE(ac.cnt, 0) > 0) AS "has_activity!",
            CAST(COALESCE(ac.cnt, 0) AS BIGINT) AS "activity_count!"
        FROM all_dates
        LEFT JOIN life_day_logs l ON l.owner_id = $1 AND l.log_date = all_dates.d_date
        LEFT JOIN activity_counts ac ON ac.act_date = all_dates.d_date
        WHERE all_dates.d_date IS NOT NULL AND all_dates.d_date != ''
        ORDER BY all_dates.d_date ASC
        "#,
        owner_id.as_str()
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| LifeHorizonDay {
            date: r.date,
            mood: r.mood,
            time_allocation: r.time_allocation,
            has_activity: r.has_activity,
            activity_count: r.activity_count,
        })
        .collect())
}

/// Collects cross-module activity events and statistics for a single calendar day.
pub async fn collect_day_activity(
    pool: &PgPool,
    owner_id: &AccountId,
    date: &str,
) -> DayActivityResponse {
    let mut events: Vec<ActivityEvent> = Vec::new();
    let mut stats = ActivityStats::default();

    // 1. Todos
    if let Ok(todo_created) = sqlx::query!(
        r#"
        SELECT text, priority, completed, created_at
        FROM todos
        WHERE owner_id = $1 AND date(created_at)::text = $2
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in todo_created {
            push_event(
                &mut events,
                extract_time(&row.created_at),
                "todo",
                "created",
                "📝",
                truncate(&row.text, 80),
                Some(format!("priority: {}", row.priority)),
            );
            stats.todos_created += 1;

            if row.completed {
                push_event(
                    &mut events,
                    extract_time(&row.created_at),
                    "todo",
                    "completed",
                    "✔️",
                    truncate(&row.text, 80),
                    None,
                );
                stats.todos_completed += 1;
            }
        }
    }

    if let Ok(todo_due) = sqlx::query!(
        r#"
        SELECT text, priority, completed, created_at
        FROM todos
        WHERE owner_id = $1 AND date(due_at)::text = $2 AND (created_at IS NULL OR date(created_at)::text != $2)
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in todo_due {
            push_event(
                &mut events,
                None,
                "todo",
                "due",
                "📅",
                format!("Due: {}", truncate(&row.text, 70)),
                if row.completed {
                    Some("✓ completed".into())
                } else {
                    Some("pending".into())
                },
            );
            stats.todos_due += 1;
        }
    }

    // 2. Finance
    if let Ok(tx_rows) = sqlx::query!(
        r#"
        SELECT t.direction, t.amount_minor, t.currency, t.currency_exponent,
               c.name AS category_name, t.note, t.occurred_at
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.owner_id = $1 AND date(t.occurred_at)::text = $2
        ORDER BY t.occurred_at
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in tx_rows {
            let exp = row.currency_exponent as u32;
            let amount = row.amount_minor as f64 / 10f64.powi(exp as i32);
            let is_expense = row.direction == "expense";
            let sign = if is_expense { "-" } else { "+" };
            let summary = format!("{sign}{}{:.2}", row.currency, amount);

            let detail_parts: Vec<String> = [
                row.category_name.map(|c| format!("category: {c}")),
                row.note.map(|n| truncate(&n, 50)),
            ]
            .into_iter()
            .flatten()
            .collect();

            push_event(
                &mut events,
                extract_time(&Some(row.occurred_at)),
                "finance",
                &row.direction,
                if is_expense { "💸" } else { "💰" },
                summary,
                if detail_parts.is_empty() {
                    None
                } else {
                    Some(detail_parts.join(" · "))
                },
            );

            if is_expense {
                stats.expenses += 1;
            } else {
                stats.income += 1;
            }
        }
    }

    // 3. Portfolio
    if let Ok(port_rows) = sqlx::query!(
        r#"
        SELECT action, symbol, quantity_minor, amount_minor,
               currency, currency_exponent, created_at
        FROM portfolio_audit_log
        WHERE owner_id = $1 AND date(created_at)::text = $2
        ORDER BY created_at
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in port_rows {
            let exp = row.currency_exponent as u32;
            let divisor = 10f64.powi(exp as i32);
            let amount = row.amount_minor as f64 / divisor;
            let sym = row.symbol.as_deref().unwrap_or("?");
            let qty = row
                .quantity_minor
                .map(|q| format!("{}", q as f64 / divisor))
                .unwrap_or_default();

            let (emoji, summary) = match row.action.as_str() {
                "buy" => (
                    "📈",
                    format!("Bought {} {} for {}{:.2}", qty, sym, row.currency, amount),
                ),
                "sell" => (
                    "📉",
                    format!("Sold {} {} for {}{:.2}", qty, sym, row.currency, amount),
                ),
                "deposit" => ("🏦", format!("Deposited {}{:.2}", row.currency, amount)),
                "withdraw" => ("🏧", format!("Withdrew {}{:.2}", row.currency, amount)),
                other => ("📊", format!("{} {}{:.2}", other, row.currency, amount)),
            };

            push_event(
                &mut events,
                extract_time(&row.created_at),
                "portfolio",
                row.action,
                emoji,
                summary,
                None,
            );
            stats.portfolio_trades += 1;
        }
    }

    // 4. Habits
    if let Ok(habit_rows) = sqlx::query!(
        r#"
        SELECT h.name AS habit_name, hl.note, hl.created_at
        FROM habit_logs hl
        JOIN habits h ON h.id = hl.habit_id
        WHERE hl.owner_id = $1 AND hl.log_date = $2
        ORDER BY hl.created_at
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in habit_rows {
            push_event(
                &mut events,
                extract_time_from_str(&row.created_at),
                "habit",
                "checked",
                "✅",
                row.habit_name,
                row.note.map(|n| truncate(&n, 60)),
            );
            stats.habits_done += 1;
        }
    }

    // 5. Reminders
    if let Ok(rem_rows) = sqlx::query!(
        r#"
        SELECT message, trigger_at, status
        FROM reminders
        WHERE owner_id = $1 AND date(trigger_at)::text = $2
        ORDER BY trigger_at
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in rem_rows {
            let action = if row.status == "sent" {
                "triggered"
            } else {
                "scheduled"
            };
            push_event(
                &mut events,
                extract_time(&Some(row.trigger_at)),
                "reminder",
                action,
                "🔔",
                truncate(&row.message, 80),
                None,
            );
            stats.reminders_fired += 1;
        }
    }

    // 6. Bookmarks
    if let Ok(bm_rows) = sqlx::query!(
        r#"
        SELECT title, domain, created_at
        FROM bookmarks
        WHERE owner_id = $1 AND date(created_at)::text = $2
        ORDER BY created_at
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in bm_rows {
            push_event(
                &mut events,
                extract_time_from_str(&row.created_at),
                "bookmark",
                "saved",
                "🔖",
                truncate(&row.title, 60),
                Some(row.domain),
            );
            stats.bookmarks_saved += 1;
        }
    }

    // 7. Footprints
    if let Ok(fp_rows) = sqlx::query!(
        r#"
        SELECT title, city, country, created_at
        FROM footprints
        WHERE owner_id = $1 AND arrival_date = $2
        ORDER BY created_at
        "#,
        owner_id.as_str(),
        date
    )
    .fetch_all(pool)
    .await
    {
        for row in fp_rows {
            let loc_parts: Vec<&str> = [row.city.as_deref(), row.country.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            let detail = if loc_parts.is_empty() {
                None
            } else {
                Some(loc_parts.join(", "))
            };
            let action = "arrived";
            let time = row.created_at.and_then(|dt| {
                if dt.format("%Y-%m-%d").to_string() == date {
                    extract_time(&Some(dt))
                } else {
                    None
                }
            });
            push_event(
                &mut events,
                time,
                "footprint",
                action,
                "📍",
                row.title,
                detail,
            );
            stats.footprints += 1;
        }
    }

    // Sort chronologically (nulls last)
    events.sort_by(|a, b| a.time.cmp(&b.time));
    stats.total_events = events.len();

    DayActivityResponse {
        date: date.to_string(),
        events,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_day_returns_zero_events() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let acct = AccountId::new("user_act_1");
        let resp = collect_day_activity(&pool, &acct, "2026-09-06").await;
        assert_eq!(resp.date, "2026-09-06");
        assert_eq!(resp.events.len(), 0);
        assert_eq!(resp.stats.total_events, 0);
    }

    #[tokio::test]
    async fn test_todo_created_on_date_appears_in_feed() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let acct = AccountId::new("user_act_2");

        sqlx::query!(
            r#"
            INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed, created_at)
            VALUES ('!r', $1, 'Alice', 'Buy groceries', 'high', FALSE, '2026-09-06 09:15:00'::timestamptz)
            "#,
            acct.as_str()
        )
        .execute(&pool)
        .await
        .unwrap();

        let resp = collect_day_activity(&pool, &acct, "2026-09-06").await;
        assert_eq!(resp.stats.todos_created, 1);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(resp.events[0].module, "todo");
        assert_eq!(resp.events[0].action, "created");
        assert!(resp.events[0].summary.contains("Buy groceries"));
    }

    #[tokio::test]
    async fn test_finance_transaction_appears_in_feed() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let acct = AccountId::new("user_act_3");

        sqlx::query!(
            r#"
            INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, currency, currency_exponent, occurred_at)
            VALUES ('!r', $1, 'Alice', 'expense', 4250, 'USD', 2, '2026-09-06 10:30:00'::timestamptz)
            "#,
            acct.as_str()
        )
        .execute(&pool)
        .await
        .unwrap();

        let resp = collect_day_activity(&pool, &acct, "2026-09-06").await;
        assert_eq!(resp.stats.expenses, 1);
        assert!(resp.events[0].summary.contains("42.50"));
    }

    #[tokio::test]
    async fn test_footprints_only_appear_on_arrival_date() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let acct = AccountId::new("user_act_4");

        sqlx::query!(
            r#"
            INSERT INTO footprints (owner_id, title, latitude, longitude, city, arrival_date, departure_date, created_at)
            VALUES ($1, 'Danxia Mountain', 25.0, 113.0, 'Shaoguan', '2026-09-04', '2026-09-14', '2026-09-04 01:48:57'::timestamptz)
            "#,
            acct.as_str()
        )
        .execute(&pool)
        .await
        .unwrap();

        let res_arrival = collect_day_activity(&pool, &acct, "2026-09-04").await;
        assert_eq!(res_arrival.stats.footprints, 1);
        assert_eq!(res_arrival.events[0].action, "arrived");

        let res_mid = collect_day_activity(&pool, &acct, "2026-09-10").await;
        assert_eq!(res_mid.stats.footprints, 0);
        assert_eq!(res_mid.events.len(), 0);

        let res_dep = collect_day_activity(&pool, &acct, "2026-09-14").await;
        assert_eq!(res_dep.stats.footprints, 0);
        assert_eq!(res_dep.events.len(), 0);
    }
}
