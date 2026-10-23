//! Recurring todos: the series anchor a patch implies and the next occurrence
//! a completed todo spawns.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use super::{NewTodo, TodoItem};
use crate::domain::reminders::{next_occurrence_in_tz, RecurrencePattern};

/// The anchor `merged` should carry after a patch was applied to `current`:
/// recurrence turned on, or the due date of a recurring todo moved, re-anchors
/// to the new due date; recurrence `none` clears it; anything else keeps it.
pub fn anchor_after_patch(current: &TodoItem, merged: &TodoItem) -> Option<DateTime<Utc>> {
    if merged.recurrence == RecurrencePattern::None {
        None
    } else if current.recurrence == RecurrencePattern::None || merged.due_at != current.due_at {
        merged.due_at
    } else {
        current.recurrence_anchor
    }
}

/// The todo a completed recurring `todo` spawns, or `None` when it does not
/// recur or has no due date.
///
/// The due date steps from the series anchor (so Jan 31 → Feb 28 → Mar 31,
/// never drifting to the 28th) in the owner's zone (so wall-clock time holds
/// across DST), to the first occurrence after both `now` and the current due
/// date: missed periods are skipped and an early completion still moves on.
pub fn successor(todo: &TodoItem, now: DateTime<Utc>, tz: Tz) -> Option<NewTodo> {
    let due = todo.due_at?;
    let anchor = todo.recurrence_anchor.unwrap_or(due);
    let next_due = next_occurrence_in_tz(anchor, todo.recurrence, now.max(due), tz)?;
    Some(NewTodo {
        room_id: todo.room_id.clone(),
        sender_name: todo.sender_name.clone(),
        text: todo.text.clone(),
        priority: todo.priority,
        due_at: Some(next_due),
        remind_before_minutes: todo.remind_before_minutes,
        remind_room_id: None,
        tags: todo.tags.clone(),
        recurrence: todo.recurrence,
        recurrence_anchor: Some(anchor),
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::tags::Tag;
    use crate::domain::todos::TodoPriority;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn todo(recurrence: RecurrencePattern, due: DateTime<Utc>) -> TodoItem {
        TodoItem {
            id: 7,
            room_id: "!room:x".to_string(),
            owner_id: AccountId::from_seed("recurrence"),
            sender_name: "alice".to_string(),
            text: "pay rent".to_string(),
            priority: TodoPriority::High,
            due_at: Some(due),
            remind_before_minutes: Some(30),
            completed: true,
            created_at: None,
            tags: vec![Tag::parse("home").unwrap()],
            recurrence,
            recurrence_anchor: Some(due),
            next_todo_id: None,
        }
    }

    /// Successor's NewTodo turned back into an item, for chaining.
    fn as_item(new: NewTodo) -> TodoItem {
        TodoItem {
            due_at: new.due_at,
            recurrence_anchor: new.recurrence_anchor,
            ..todo(new.recurrence, new.due_at.unwrap())
        }
    }

    #[test]
    fn non_recurring_has_no_successor() {
        let t = todo(RecurrencePattern::None, utc(2026, 1, 1, 9));
        assert!(successor(&t, utc(2026, 1, 3, 0), Tz::UTC).is_none());
        let no_due = TodoItem {
            due_at: None,
            ..todo(RecurrencePattern::Daily, utc(2026, 1, 1, 9))
        };
        assert!(successor(&no_due, utc(2026, 1, 3, 0), Tz::UTC).is_none());
    }

    #[test]
    fn monthly_successor_keeps_fields_and_advances_due() {
        let t = todo(RecurrencePattern::Monthly, utc(2026, 1, 1, 9));
        let next = successor(&t, utc(2026, 1, 3, 0), Tz::UTC).unwrap();
        assert_eq!(next.due_at, Some(utc(2026, 2, 1, 9)));
        assert_eq!(next.tags, t.tags);
        assert_eq!(next.priority, TodoPriority::High);
        assert_eq!(next.remind_before_minutes, Some(30));
        assert_eq!(next.text, "pay rent");
        assert_eq!(next.room_id, "!room:x");
        assert_eq!(next.sender_name, "alice");
        assert_eq!(next.recurrence, RecurrencePattern::Monthly);
        assert_eq!(next.remind_room_id, None);
    }

    #[test]
    fn missed_periods_are_skipped() {
        let t = todo(RecurrencePattern::Daily, utc(2026, 1, 1, 9));
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap();
        let next = successor(&t, now, Tz::UTC).unwrap();
        assert_eq!(next.due_at, Some(utc(2026, 1, 6, 9)));
    }

    #[test]
    fn month_end_keeps_anchor_day() {
        let t = todo(RecurrencePattern::Monthly, utc(2026, 1, 31, 9));
        let feb = successor(&t, utc(2026, 2, 1, 0), Tz::UTC).unwrap();
        assert_eq!(feb.due_at, Some(utc(2026, 2, 28, 9)));
        assert_eq!(feb.recurrence_anchor, Some(utc(2026, 1, 31, 9)));

        let mar = successor(&as_item(feb), utc(2026, 3, 1, 0), Tz::UTC).unwrap();
        assert_eq!(mar.due_at, Some(utc(2026, 3, 31, 9)));
    }

    #[test]
    fn early_completion_moves_past_current_due() {
        let t = todo(RecurrencePattern::Daily, utc(2026, 1, 5, 9));
        let next = successor(&t, utc(2026, 1, 3, 0), Tz::UTC).unwrap();
        assert_eq!(next.due_at, Some(utc(2026, 1, 6, 9)));
    }

    #[test]
    fn dst_keeps_local_wall_time() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        // 09:00 CET (UTC+1) on Monday 2026-03-23; DST starts 2026-03-29.
        let due = berlin
            .with_ymd_and_hms(2026, 3, 23, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(due, utc(2026, 3, 23, 8));
        let t = todo(RecurrencePattern::Weekly, due);
        let next = successor(&t, due + chrono::Duration::minutes(1), berlin).unwrap();
        let expected = berlin
            .with_ymd_and_hms(2026, 3, 30, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(next.due_at, Some(expected));
        assert_eq!(expected, utc(2026, 3, 30, 7));
    }

    #[test]
    fn anchor_follows_recurrence_and_due_changes() {
        let due = utc(2026, 1, 1, 9);
        let later = utc(2026, 1, 4, 9);
        let plain = TodoItem {
            recurrence_anchor: None,
            ..todo(RecurrencePattern::None, due)
        };
        let on = TodoItem {
            recurrence: RecurrencePattern::Weekly,
            ..plain.clone()
        };
        assert_eq!(anchor_after_patch(&plain, &on), Some(due));

        let recurring = todo(RecurrencePattern::Weekly, due);
        let renamed = TodoItem {
            text: "x".to_string(),
            ..recurring.clone()
        };
        assert_eq!(anchor_after_patch(&recurring, &renamed), Some(due));
        let moved = TodoItem {
            due_at: Some(later),
            ..recurring.clone()
        };
        assert_eq!(anchor_after_patch(&recurring, &moved), Some(later));
        let off = TodoItem {
            recurrence: RecurrencePattern::None,
            ..recurring.clone()
        };
        assert_eq!(anchor_after_patch(&recurring, &off), None);
    }
}
