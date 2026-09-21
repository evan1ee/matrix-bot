//! Per-module query collectors for life day activity feed.

use crate::db::DbPool;
use chrono::{DateTime, Utc};

use crate::modules::life::activity::{ActivityEvent, ActivityStats};

/// Helper to extract HH:MM:SS from a TIMESTAMP column.
pub(crate) fn extract_time(dt: &Option<DateTime<Utc>>) -> Option<String> {
    dt.map(|d| d.format("%H:%M:%S").to_string())
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

pub(crate) fn push_event(
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
        module,
        action: action.into(),
        emoji,
        summary,
        detail,
    });
}

pub(crate) async fn query_todos(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct TodoRow {
        text: String,
        priority: String,
        completed: bool,
        created_at: Option<DateTime<Utc>>,
    }

    let rows = crate::db_fetch_all!(
        pool,
        TodoRow,
        "SELECT text, priority, completed, created_at \
         FROM todos WHERE owner_id = ? AND CAST(date(created_at) AS TEXT) = ?",
        owner,
        date
    )
    .unwrap_or_default();

    for row in &rows {
        push_event(
            events,
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
                events,
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

    // Todos due on this date (but created on a different date)
    let due_rows = crate::db_fetch_all!(
        pool,
        TodoRow,
        "SELECT text, priority, completed, created_at \
         FROM todos WHERE owner_id = ? AND CAST(date(due_at) AS TEXT) = ? AND CAST(date(created_at) AS TEXT) != ?",
        owner,
        date,
        date
    )
    .unwrap_or_default();

    for row in due_rows {
        push_event(
            events,
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

pub(crate) async fn query_finance(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct TxRow {
        direction: String,
        amount_minor: i64,
        currency: String,
        currency_exponent: i64,
        category_name: Option<String>,
        note: Option<String>,
        occurred_at: DateTime<Utc>,
    }

    let rows = crate::db_fetch_all!(
        pool,
        TxRow,
        "SELECT t.direction, t.amount_minor, t.currency, t.currency_exponent, \
                c.name AS category_name, t.note, t.occurred_at \
         FROM transactions t \
         LEFT JOIN finance_categories c ON c.id = t.category_id \
         WHERE t.owner_id = ? AND CAST(date(t.occurred_at) AS TEXT) = ? \
         ORDER BY t.occurred_at",
        owner,
        date
    )
    .unwrap_or_default();

    for row in rows {
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
            events,
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

pub(crate) async fn query_portfolio(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct AuditRow {
        action: String,
        symbol: Option<String>,
        quantity_minor: Option<i64>,
        amount_minor: i64,
        currency: String,
        currency_exponent: i64,
        created_at: Option<DateTime<Utc>>,
    }

    let rows = crate::db_fetch_all!(
        pool,
        AuditRow,
        "SELECT action, symbol, quantity_minor, amount_minor, \
                currency, currency_exponent, created_at \
         FROM portfolio_audit_log \
         WHERE owner_id = ? AND CAST(date(created_at) AS TEXT) = ? \
         ORDER BY created_at",
        owner,
        date
    )
    .unwrap_or_default();

    for row in rows {
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
            events,
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

pub(crate) async fn query_habits(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct HabitRow {
        habit_name: String,
        note: Option<String>,
        created_at: Option<DateTime<Utc>>,
    }

    let rows = crate::db_fetch_all!(
        pool,
        HabitRow,
        "SELECT h.name AS habit_name, hl.note, hl.created_at \
         FROM habit_logs hl \
         JOIN habits h ON h.id = hl.habit_id \
         WHERE hl.owner_id = ? AND hl.log_date = ? \
         ORDER BY hl.created_at",
        owner,
        date
    )
    .unwrap_or_default();

    for row in rows {
        push_event(
            events,
            extract_time(&row.created_at),
            "habit",
            "checked",
            "✅",
            row.habit_name,
            row.note.map(|n| truncate(&n, 60)),
        );
        stats.habits_done += 1;
    }
}

pub(crate) async fn query_reminders(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct ReminderRow {
        message: String,
        trigger_at: DateTime<Utc>,
        status: String,
    }

    let rows = crate::db_fetch_all!(
        pool,
        ReminderRow,
        "SELECT message, trigger_at, status \
         FROM reminders \
         WHERE owner_id = ? AND CAST(date(trigger_at) AS TEXT) = ? \
         ORDER BY trigger_at",
        owner,
        date
    )
    .unwrap_or_default();

    for row in rows {
        let action = if row.status == "sent" {
            "triggered"
        } else {
            "scheduled"
        };
        push_event(
            events,
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

pub(crate) async fn query_bookmarks(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct BookmarkRow {
        title: String,
        domain: String,
        created_at: Option<DateTime<Utc>>,
    }

    let rows = crate::db_fetch_all!(
        pool,
        BookmarkRow,
        "SELECT title, domain, created_at \
         FROM bookmarks \
         WHERE owner_id = ? AND CAST(date(created_at) AS TEXT) = ? \
         ORDER BY created_at",
        owner,
        date
    )
    .unwrap_or_default();

    for row in rows {
        push_event(
            events,
            extract_time(&row.created_at),
            "bookmark",
            "saved",
            "🔖",
            truncate(&row.title, 60),
            Some(row.domain),
        );
        stats.bookmarks_saved += 1;
    }
}

pub(crate) async fn query_footprints(
    pool: &DbPool,
    owner: &str,
    date: &str,
    events: &mut Vec<ActivityEvent>,
    stats: &mut ActivityStats,
) {
    #[derive(sqlx::FromRow)]
    struct FootprintRow {
        title: String,
        city: Option<String>,
        country: Option<String>,
        created_at: Option<DateTime<Utc>>,
    }

    let rows = crate::db_fetch_all!(
        pool,
        FootprintRow,
        "SELECT title, city, country, created_at \
         FROM footprints \
         WHERE owner_id = ? AND arrival_date = ? \
         ORDER BY created_at",
        owner,
        date
    )
    .unwrap_or_default();

    for row in rows {
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
        push_event(events, time, "footprint", action, "📍", row.title, detail);
        stats.footprints += 1;
    }
}
