/// Builds the contextual prompt feeding OCR-extracted text and optional caption into the AI agent.
pub fn build_ocr_prompt(caption: Option<&str>, ocr_text: &str) -> String {
    let caption_section = match caption {
        Some(c) if !c.trim().is_empty() => {
            let trimmed = c.trim();
            let lower = trimmed.to_ascii_lowercase();
            if lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".png")
                || lower.ends_with(".webp")
                || lower.ends_with(".gif")
            {
                String::new()
            } else {
                format!("User Note / Caption: \"{}\"\n", trimmed)
            }
        }
        _ => String::new(),
    };

    format!(
        "[Image OCR Ingestion]\n\
        The user sent an image attachment. The OCR engine recognized the following text:\n\
        \"\"\"\n\
        {}\n\
        \"\"\"\n\
        {}\n\
        LANGUAGE MATCHING (CRITICAL): Always reply in the EXACT SAME language as the recognized text or user note. If the receipt, invoice, or note contains Chinese text, you MUST reply entirely in Chinese (简体中文). Do NOT translate into English. If English, reply in English.\n\
        \n\
        Please analyze this content and perform the appropriate actions:\n\
        1. Receipt / Invoice / Payment: If this is a receipt, ticket, invoice, or restaurant bill:\n\
           - Extract merchant, total amount (note: minor units in cents), currency (USD, CNY, EUR, etc.), date, and category.\n\
           - You MUST execute `record_finance_entry` (or `create_recurring_bill` for recurring subscriptions) via function calling.\n\
        2. To-Do List / Notes: If this is a task list, whiteboard photo, or action items:\n\
           - Extract each task and call `add_todo`.\n\
        3. Appointments / Deadlines: If this mentions an appointment, event, or deadline:\n\
           - Call `create_reminder` with the appropriate trigger time.\n\
        4. General Information: If none of the above tools apply, provide a succinct summary.\n\
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
          ✅ **提醒已设定**\n\n- **内容**: {{内容}}\n- **时间**: {{时间（使用用户本地默认时区，格式如 YYYY-MM-DD HH:mm，切勿输出 UTC 或带 Z）}}\n\
        - Reminder in English:\n\
          ✅ **Reminder set**\n\n- **Content**: {{Content}}\n- **Time**: {{Time in user's default timezone, e.g. YYYY-MM-DD HH:mm (never show UTC or Z)}}",
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
