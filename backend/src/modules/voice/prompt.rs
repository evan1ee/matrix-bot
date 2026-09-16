/// Builds the contextual prompt feeding voice-transcribed text into the AI agent.
pub fn build_voice_prompt(transcribed_text: &str, duration_secs: f32) -> String {
    format!(
        "[Voice Note Transcription - Duration: {:.1}s]\n\
        The user sent a voice message. The speech-to-text engine transcribed the following speech:\n\
        \"\"\"\n\
        {}\n\
        \"\"\"\n\
        Please interpret the user's intent and execute the appropriate actions:\n\
        1. Expense / Financial Record: If the user mentions spending money, buying something, or a payment:\n\
           - Extract description/merchant, amount (in minor units/cents), currency (USD, CNY, EUR, etc.), and category.\n\
           - Call `add_expense` (or `add_bill` for recurring subscriptions).\n\
        2. Tasks / To-Do Items: If the user mentions something they need to do or a task to track:\n\
           - Call `add_todo`.\n\
        3. Reminders / Deadlines: If the user asks for a reminder at a specific time or date:\n\
           - Call `create_reminder` with the calculated trigger time.\n\
        4. Habit Logging: If the user mentions completing a habit (e.g. read, workout):\n\
           - Call `log_habit`.\n\
        5. General Conversation / Question: If the user is asking a question or making an inquiry:\n\
           - Answer concisely, helpfully, and naturally.\n\
        \n\
        Always confirm executed actions clearly with friendly formatting.",
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
        assert!(prompt.contains("add_expense"));
        assert!(prompt.contains("add_todo"));
        assert!(prompt.contains("create_reminder"));
    }
}
