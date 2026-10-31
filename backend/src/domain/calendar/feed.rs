//! Maps todos, recurring bills and reminders onto one iCalendar document.

use chrono::{DateTime, Utc};

use super::ics::{escape_text, fold_line, format_utc};
use crate::domain::finance::money::format_amount;
use crate::domain::recurring_bills::{parse_due_at, still_recurring, RecurringBill};
use crate::domain::reminders::{RecurrencePattern, Reminder, ReminderStatus};
use crate::domain::todos::{TodoItem, TodoPriority};

/// A feed never carries more than this many components.
pub const MAX_FEED_ITEMS: usize = 500;

const UID_DOMAIN: &str = "matrix-bot";

/// The agenda rows one account contributes; the caller has already scoped them
/// to the owner.
#[derive(Debug, Default, Clone)]
pub struct CalendarInput {
    pub todos: Vec<TodoItem>,
    pub bills: Vec<RecurringBill>,
    pub reminders: Vec<Reminder>,
}

/// A reminder tied to a todo or bill is announced by that parent, so only
/// standalone reminders get an entry of their own.
pub fn should_emit_reminder(reminder: &Reminder) -> bool {
    reminder.source_kind.is_none() && reminder.source_id.is_none()
}

struct Entry {
    start: DateTime<Utc>,
    lines: Vec<String>,
}

/// Builds the full `VCALENDAR` text (CRLF line endings, folded lines).
pub fn build_feed(input: &CalendarInput, now: DateTime<Utc>) -> String {
    let mut entries: Vec<Entry> = input
        .todos
        .iter()
        .filter_map(|t| todo_entry(t, now))
        .chain(input.bills.iter().filter_map(|b| bill_entry(b, now)))
        .chain(input.reminders.iter().filter_map(|r| reminder_entry(r, now)))
        .collect();
    entries.sort_by_key(|e| e.start);
    entries.truncate(MAX_FEED_ITEMS);

    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//matrix-bot//calendar//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "X-WR-CALNAME:matrix-bot".to_string(),
    ];
    lines.extend(entries.into_iter().flat_map(|e| e.lines));
    lines.push("END:VCALENDAR".to_string());

    let mut out = String::new();
    for line in lines {
        out.push_str(&fold_line(&line));
        out.push_str("\r\n");
    }
    out
}

fn todo_entry(todo: &TodoItem, now: DateTime<Utc>) -> Option<Entry> {
    let due = todo.due_at.filter(|_| !todo.completed)?;
    let mut lines = vec![
        "BEGIN:VTODO".to_string(),
        format!("UID:todo-{}@{UID_DOMAIN}", todo.id),
        format!("DTSTAMP:{}", format_utc(now)),
        format!("DUE:{}", format_utc(due)),
        format!("SUMMARY:{}", escape_text(&todo.text)),
        format!(
            "PRIORITY:{}",
            match todo.priority {
                TodoPriority::High => 1,
                TodoPriority::Medium => 5,
                TodoPriority::Low => 9,
            }
        ),
    ];
    if !todo.tags.is_empty() {
        let tags: Vec<String> = todo.tags.iter().map(|t| escape_text(t.as_str())).collect();
        lines.push(format!("CATEGORIES:{}", tags.join(",")));
    }
    if let Some(minutes) = todo.remind_before_minutes.filter(|m| *m > 0) {
        lines.extend(alarm(&format!("-PT{minutes}M"), &todo.text));
    }
    lines.push("END:VTODO".to_string());
    Some(Entry { start: due, lines })
}

fn bill_entry(bill: &RecurringBill, now: DateTime<Utc>) -> Option<Entry> {
    if !still_recurring(&bill.status) {
        return None;
    }
    let start = parse_due_at(&bill.next_due_at)?;
    let amount = format_amount(bill.amount_minor, bill.currency_exponent.clamp(0, 9) as u32);
    let mut lines = vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:bill-{}@{UID_DOMAIN}", bill.id),
        format!("DTSTAMP:{}", format_utc(now)),
        format!("DTSTART:{}", format_utc(start)),
        format!("SUMMARY:{}", escape_text(&bill.name)),
        format!(
            "DESCRIPTION:{}",
            escape_text(&format!("{amount} {} ({})", bill.currency, bill.category))
        ),
    ];
    if let Some(rule) = bill_rrule(bill) {
        lines.push(format!("RRULE:{rule}"));
    }
    lines.push("END:VEVENT".to_string());
    Some(Entry { start, lines })
}

fn bill_rrule(bill: &RecurringBill) -> Option<String> {
    let base = match bill.frequency.trim().to_ascii_lowercase().as_str() {
        "daily" => "FREQ=DAILY",
        "weekly" => "FREQ=WEEKLY",
        "biweekly" => "FREQ=WEEKLY;INTERVAL=2",
        "monthly" => "FREQ=MONTHLY",
        "quarterly" => "FREQ=MONTHLY;INTERVAL=3",
        "yearly" => "FREQ=YEARLY",
        _ => return None,
    };
    let remaining = bill
        .total_installments
        .map(|total| total - bill.paid_installments)
        .filter(|left| *left > 0);
    Some(match remaining {
        Some(count) => format!("{base};COUNT={count}"),
        None => base.to_string(),
    })
}

fn reminder_entry(reminder: &Reminder, now: DateTime<Utc>) -> Option<Entry> {
    if reminder.status != ReminderStatus::Pending || !should_emit_reminder(reminder) {
        return None;
    }
    let mut lines = vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:reminder-{}@{UID_DOMAIN}", reminder.id),
        format!("DTSTAMP:{}", format_utc(now)),
        format!("DTSTART:{}", format_utc(reminder.trigger_at)),
        format!("SUMMARY:{}", escape_text(&reminder.message)),
    ];
    let freq = match reminder.recurrence {
        RecurrencePattern::None => None,
        RecurrencePattern::Daily => Some("DAILY"),
        RecurrencePattern::Weekly => Some("WEEKLY"),
        RecurrencePattern::Monthly => Some("MONTHLY"),
        RecurrencePattern::Yearly => Some("YEARLY"),
    };
    if let Some(freq) = freq {
        lines.push(format!("RRULE:FREQ={freq}"));
    }
    lines.extend(alarm("PT0S", &reminder.message));
    lines.push("END:VEVENT".to_string());
    Some(Entry {
        start: reminder.trigger_at,
        lines,
    })
}

fn alarm(trigger: &str, description: &str) -> [String; 5] {
    [
        "BEGIN:VALARM".to_string(),
        "ACTION:DISPLAY".to_string(),
        format!("TRIGGER:{trigger}"),
        format!("DESCRIPTION:{}", escape_text(description)),
        "END:VALARM".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::ReminderSource;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap()
    }

    fn todo(id: i64, due_in_days: Option<i64>) -> TodoItem {
        TodoItem {
            id,
            room_id: "web".into(),
            owner_id: AccountId::from_seed("cal"),
            sender_name: "a".into(),
            text: format!("Task {id}"),
            priority: TodoPriority::High,
            due_at: due_in_days.map(|d| now() + Duration::days(d)),
            remind_before_minutes: Some(30),
            completed: false,
            created_at: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
            next_todo_id: None,
        }
    }

    fn bill(id: i64, frequency: &str, next_due_at: &str) -> RecurringBill {
        RecurringBill {
            id,
            room_id: "web".into(),
            owner_id: AccountId::from_seed("cal"),
            sender_name: "a".into(),
            name: "Rent".into(),
            category: "Housing".into(),
            amount_minor: 120050,
            currency: "USD".into(),
            currency_exponent: 2,
            frequency: frequency.into(),
            due_day: None,
            next_due_at: next_due_at.into(),
            total_installments: None,
            paid_installments: 0,
            auto_pay: false,
            status: "active".into(),
            notes: None,
            remind_before_minutes: None,
            remind_room_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn reminder(id: i64) -> Reminder {
        Reminder {
            id,
            room_id: "web".into(),
            owner_id: AccountId::from_seed("cal"),
            sender_name: "a".into(),
            message: "Call mum".into(),
            trigger_at: now() + Duration::days(1),
            recurrence: RecurrencePattern::None,
            status: ReminderStatus::Pending,
            retry_count: 0,
            source_kind: None,
            source_id: None,
            created_at: None,
        }
    }

    fn feed(input: CalendarInput) -> String {
        build_feed(&input, now())
    }

    #[test]
    fn reminder_with_source_is_suppressed() {
        let mut r = reminder(1);
        assert!(should_emit_reminder(&r));
        r.source_kind = Some(ReminderSource::Todo);
        assert!(!should_emit_reminder(&r));
        r.source_kind = None;
        r.source_id = Some(1);
        assert!(!should_emit_reminder(&r));
    }

    #[test]
    fn todo_becomes_vtodo_with_stable_uid_and_alarm() {
        let out = feed(CalendarInput {
            todos: vec![todo(7, Some(2))],
            ..Default::default()
        });
        assert!(out.contains("BEGIN:VTODO\r\nUID:todo-7@matrix-bot\r\n"));
        assert!(out.contains("DUE:20260303T000000Z"));
        assert!(out.contains("PRIORITY:1"));
        assert!(out.contains("TRIGGER:-PT30M"));
    }

    #[test]
    fn completed_todo_and_undated_todo_omitted() {
        let mut done = todo(1, Some(1));
        done.completed = true;
        let out = feed(CalendarInput {
            todos: vec![done, todo(2, None)],
            ..Default::default()
        });
        assert!(!out.contains("VTODO"));
    }

    #[test]
    fn monthly_bill_has_rrule_and_stable_uid() {
        let out = feed(CalendarInput {
            bills: vec![bill(3, "monthly", "2026-03-15 10:00:00")],
            ..Default::default()
        });
        assert!(out.contains("UID:bill-3@matrix-bot"));
        assert!(out.contains("DTSTART:20260315T100000Z"));
        assert!(out.contains("RRULE:FREQ=MONTHLY\r\n"));
        assert!(out.contains("DESCRIPTION:1200.50 USD (Housing)"));
    }

    #[test]
    fn finite_bill_gets_count() {
        let mut b = bill(3, "monthly", "2026-03-15 10:00:00");
        b.total_installments = Some(12);
        b.paid_installments = 4;
        let out = feed(CalendarInput {
            bills: vec![b],
            ..Default::default()
        });
        assert!(out.contains("RRULE:FREQ=MONTHLY;COUNT=8"));
    }

    #[test]
    fn unknown_frequency_has_no_rrule() {
        let out = feed(CalendarInput {
            bills: vec![bill(3, "whenever", "2026-03-15 10:00:00")],
            ..Default::default()
        });
        assert!(out.contains("BEGIN:VEVENT"));
        assert!(!out.contains("RRULE"));
    }

    #[test]
    fn bad_due_date_and_inactive_bill_skipped() {
        let mut paused = bill(2, "monthly", "2026-03-15 10:00:00");
        paused.status = "paused".into();
        let out = feed(CalendarInput {
            bills: vec![bill(1, "monthly", "not a date"), paused],
            ..Default::default()
        });
        assert!(!out.contains("VEVENT"));
    }

    #[test]
    fn non_pending_reminder_omitted() {
        let mut r = reminder(1);
        r.status = ReminderStatus::Triggered;
        let out = feed(CalendarInput {
            reminders: vec![r, reminder(2)],
            ..Default::default()
        });
        assert!(!out.contains("reminder-1@"));
        assert!(out.contains("reminder-2@"));
    }

    #[test]
    fn special_characters_survive_escape_and_fold() {
        let mut t = todo(1, Some(1));
        t.text = format!("Buy milk, eggs; and \\ more {}", "日本語😀".repeat(30));
        let out = feed(CalendarInput {
            todos: vec![t.clone()],
            ..Default::default()
        });
        for line in out.split("\r\n") {
            assert!(line.len() <= 75);
        }
        let unfolded = out.replace("\r\n ", "");
        let summary = unfolded
            .split("\r\n")
            .find(|l| l.starts_with("SUMMARY:"))
            .unwrap();
        assert_eq!(summary, format!("SUMMARY:{}", escape_text(&t.text)));
    }

    #[test]
    fn feed_caps_at_500_earliest_first() {
        let todos: Vec<_> = (0..600).map(|i| todo(i, Some(600 - i))).collect();
        let out = feed(CalendarInput {
            todos,
            ..Default::default()
        });
        assert_eq!(out.matches("BEGIN:VTODO").count(), MAX_FEED_ITEMS);
        let first_uid = out.lines().find(|l| l.starts_with("UID:")).unwrap();
        assert_eq!(first_uid, "UID:todo-599@matrix-bot");
    }

    #[test]
    fn feed_uses_crlf_and_balanced_begin_end() {
        let out = feed(CalendarInput {
            todos: vec![todo(1, Some(1))],
            bills: vec![bill(1, "weekly", "2026-03-15 10:00:00")],
            reminders: vec![reminder(1)],
        });
        assert!(out.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(out.ends_with("END:VCALENDAR\r\n"));
        assert!(!out.replace("\r\n", "").contains('\n'));
        assert_eq!(out.matches("BEGIN:").count(), out.matches("END:").count());
    }

    #[test]
    fn empty_input_is_a_valid_empty_calendar() {
        let out = feed(CalendarInput::default());
        assert!(out.starts_with("BEGIN:VCALENDAR") && out.ends_with("END:VCALENDAR\r\n"));
    }
}
