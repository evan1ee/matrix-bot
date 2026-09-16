/// Builds the contextual prompt feeding voice-transcribed text into the AI agent.
pub fn build_voice_prompt(transcribed_text: &str, duration_secs: f32) -> String {
    format!(
        "[Voice Note Transcription - Duration: {:.1}s]\n\
        The user sent a voice message. The speech-to-text engine transcribed the following speech:\n\
        \"\"\"\n\
        {}\n\
        \"\"\"\n\
        LANGUAGE MATCHING (CRITICAL): Always reply in the EXACT SAME language as the transcribed speech. If the speech is in Chinese, you MUST reply entirely in Chinese (简体中文). Do NOT translate into English. If English, reply in English.\n\
        \n\
        Please interpret the user's intent and execute the appropriate actions:\n\
        1. Expense / Financial Record: If the user mentions spending money, buying something, or a payment:\n\
           - Extract description/merchant, amount (in minor units/cents), currency (USD, CNY, EUR, etc.), and category.\n\
           - Call `record_finance_entry` (or `create_recurring_bill` for recurring subscriptions).\n\
        2. Tasks / To-Do Items: If the user mentions something they need to do or a task to track:\n\
           - Call `add_todo`.\n\
        3. Reminders / Deadlines: If the user asks for a reminder at a specific time or date:\n\
           - Call `create_reminder` with the calculated trigger time.\n\
        4. Habit Logging: If the user mentions completing a habit (e.g. read, workout):\n\
           - Call `log_habit`.\n\
        5. General Conversation / Question: If the user is asking a question or making an inquiry:\n\
           - Answer concisely, helpfully, and naturally.\n\
        \n\
        CRITICAL: You must invoke the actual tool function via the API. Do NOT output text claiming you called a tool without actually calling it.\n\
        \n\
        FIXED CONFIRMATION TEMPLATES: Once the tool executes, use strictly this fixed confirmation template without introductory conversational filler:\n\
        - Expense in Chinese:\n\
          ✅ **支出已记录**\n\n- **商家**: {{商家}}\n- **金额**: {{金额}} {{币种}}\n- **日期**: {{YYYY-MM-DD}}\n- **分类**: {{分类}}\n- **描述**: {{商品或备注}}\n\
        - Expense in English:\n\
          ✅ **Expense recorded**\n\n- **Merchant**: {{Merchant}}\n- **Amount**: {{Amount}} {{Currency}}\n- **Date**: {{YYYY-MM-DD}}\n- **Category**: {{Category}}\n- **Description**: {{Description}}\n\
        - Task in Chinese:\n\
          ✅ **任务已添加**\n\n- **任务**: {{任务}}\n- **截止时间**: {{截止时间或无}}\n- **优先级**: {{普通/高/低}}\n\
        - Task in English:\n\
          ✅ **Task added**\n\n- **Task**: {{Task}}\n- **Due**: {{Due date or None}}\n- **Priority**: {{Priority}}\n\
        - Reminder in Chinese:\n\
          ✅ **提醒已设定**\n\n- **内容**: {{内容}}\n- **时间**: {{时间}}\n\
        - Reminder in English:\n\
          ✅ **Reminder set**\n\n- **Content**: {{Content}}\n- **Time**: {{Time}}",
        duration_secs,
        transcribed_text.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_voice_prompt_formatting() {
        let prompt = build_voice_prompt("Bought coffee for 4 dollars and 50 cents", 3.2);
        assert!(prompt.contains("Duration: 3.2s"));
        assert!(prompt.contains("Bought coffee for 4 dollars and 50 cents"));
        assert!(prompt.contains("record_finance_entry"));
        assert!(prompt.contains("add_todo"));
        assert!(prompt.contains("create_reminder"));
    }
}
