/// Builds the contextual prompt feeding OCR-extracted text and optional caption into the AI agent.
pub fn build_ocr_prompt(caption: Option<&str>, ocr_text: &str) -> String {
    let caption_section = match caption {
        Some(c) if !c.trim().is_empty() => format!("User Note / Caption: \"{}\"\n", c.trim()),
        _ => String::new(),
    };

    format!(
        "[Image OCR Ingestion]\n\
        The user sent an image attachment. The OCR engine recognized the following text:\n\
        \"\"\"\n\
        {}\n\
        \"\"\"\n\
        {}\n\
        Please analyze this content and perform the appropriate actions:\n\
        1. Receipt / Invoice / Payment: If this is a receipt, ticket, invoice, or restaurant bill:\n\
           - Extract merchant, total amount (note: minor units in cents), currency (USD, CNY, EUR, etc.), date, and category.\n\
           - Call `record_finance_entry` (or `create_recurring_bill` for recurring subscriptions).\n\
        2. To-Do List / Notes: If this is a task list, whiteboard photo, or action items:\n\
           - Extract each task and call `add_todo`.\n\
        3. Appointments / Deadlines: If this mentions an appointment, event, or deadline:\n\
           - Call `create_reminder` with the appropriate trigger time.\n\
        4. General Information: If none of the above tools apply, provide a succinct summary.\n\
        \n\
        Always confirm with friendly bullet points detailing the extracted items and executed tools.",
        ocr_text.trim(),
        caption_section
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ocr_prompt_with_caption() {
        let prompt = build_ocr_prompt(Some("dinner with colleagues"), "SHAKE SHACK\nTotal: $16.50");
        assert!(prompt.contains("dinner with colleagues"));
        assert!(prompt.contains("SHAKE SHACK"));
        assert!(prompt.contains("record_finance_entry"));
    }

    #[test]
    fn test_build_ocr_prompt_without_caption() {
        let prompt = build_ocr_prompt(None, "Meeting notes:\n- Review PR\n- Deploy to prod");
        assert!(prompt.contains("Meeting notes"));
        assert!(prompt.contains("add_todo"));
        assert!(!prompt.contains("User Note / Caption"));
    }
}
