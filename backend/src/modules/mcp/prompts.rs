use std::collections::HashMap;

use super::protocol::{GetPromptResult, McpPrompt, PromptContent, PromptMessage};

/// Returns the standard prompt templates available to MCP clients.
pub fn list_prompts() -> Vec<McpPrompt> {
    vec![
        McpPrompt {
            name: "daily-briefing".to_string(),
            description: Some("Generates an executive morning briefing covering pending tasks, habits, and upcoming bills".to_string()),
            arguments: None,
        },
        McpPrompt {
            name: "weekly-retrospective".to_string(),
            description: Some("Conducts a comprehensive retrospective of spending, completed todos, and habit streaks".to_string()),
            arguments: None,
        },
        McpPrompt {
            name: "portfolio-review".to_string(),
            description: Some("Audits investment holdings, cash positions, and alerts".to_string()),
            arguments: None,
        },
    ]
}

/// Formulates the prompt messages for a requested prompt template.
pub fn get_prompt(
    name: &str,
    _arguments: Option<HashMap<String, String>>,
) -> Result<GetPromptResult, String> {
    match name {
        "daily-briefing" => Ok(GetPromptResult {
            description: Some("Daily Morning Executive Briefing".to_string()),
            messages: vec![PromptMessage {
                role: "user".to_string(),
                content: PromptContent::Text {
                    text: "Please inspect my pending tasks (matrixbot://todos/pending), today's habits (matrixbot://habits/today), and recent financial transactions (matrixbot://finance/recent). Provide a clear, prioritized morning briefing highlighting top actions for today and any urgent items.".to_string(),
                },
            }],
        }),
        "weekly-retrospective" => Ok(GetPromptResult {
            description: Some("Weekly Productivity & Financial Review".to_string()),
            messages: vec![PromptMessage {
                role: "user".to_string(),
                content: PromptContent::Text {
                    text: "Review my financial activity for this week and my completed task velocity. Highlight what went well, areas where budget spending was high, and suggestions to optimize my habits next week.".to_string(),
                },
            }],
        }),
        "portfolio-review" => Ok(GetPromptResult {
            description: Some("Portfolio Asset Allocation Audit".to_string()),
            messages: vec![PromptMessage {
                role: "user".to_string(),
                content: PromptContent::Text {
                    text: "Please read my current investment portfolio summary (matrixbot://portfolio/summary) and review my asset allocation, cash balances, and risk diversification.".to_string(),
                },
            }],
        }),
        _ => Err(format!("Unknown prompt: {}", name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_and_get_prompt() {
        let prompts = list_prompts();
        assert_eq!(prompts.len(), 3);
        let res = get_prompt("daily-briefing", None).unwrap();
        assert!(!res.messages.is_empty());
    }
}
