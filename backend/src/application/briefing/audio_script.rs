//! The spoken version of the morning briefing, built from the same data as
//! the text so the two never disagree. Written in the briefing's language.

use chrono::{Datelike, NaiveDate};

use crate::domain::briefing::BriefingAudioScript;
use crate::domain::habits::HabitWithStatus;
use crate::domain::recurring_bills::RecurringBill;
use crate::domain::todos::TodoItem;

/// Tasks named aloud; the rest are only counted.
const SPOKEN_TASKS: usize = 3;

pub fn build_audio_script(
    local_date: NaiveDate,
    todos: &[TodoItem],
    habits: &[HabitWithStatus],
    bills: &[&RecurringBill],
    budgets_needing_attention: usize,
) -> Option<BriefingAudioScript> {
    let mut script = format!(
        "早安。今天是{}月{}日。",
        local_date.month(),
        local_date.day()
    );
    if !todos.is_empty() {
        let names: Vec<String> = todos
            .iter()
            .take(SPOKEN_TASKS)
            .map(|t| t.text.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        script.push_str(&format!(
            "你有{}项待办，最先是：{}。",
            todos.len(),
            names.join("，")
        ));
    }
    if !habits.is_empty() {
        let done = habits.iter().filter(|h| h.completed_today).count();
        script.push_str(&format!("习惯打卡已完成{}项，共{}项。", done, habits.len()));
    }
    if !bills.is_empty() {
        let names: Vec<&str> = bills
            .iter()
            .take(SPOKEN_TASKS)
            .map(|b| b.name.as_str())
            .collect();
        script.push_str(&format!(
            "近期有{}笔账单到期，包括{}。",
            bills.len(),
            names.join("，")
        ));
    }
    if budgets_needing_attention > 0 {
        script.push_str(&format!(
            "有{budgets_needing_attention}项预算接近或超出上限。"
        ));
    }
    script.push_str("祝你今天顺利。");
    BriefingAudioScript::new(&script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
    }

    #[test]
    fn an_empty_day_is_a_greeting_and_a_goodbye() {
        let script = build_audio_script(date(), &[], &[], &[], 0).unwrap();
        assert_eq!(script.as_str(), "早安。今天是10月31日。祝你今天顺利。");
    }

    #[test]
    fn budgets_are_counted_aloud() {
        let script = build_audio_script(date(), &[], &[], &[], 2).unwrap();
        assert!(script.as_str().contains("有2项预算接近或超出上限"));
    }
}
