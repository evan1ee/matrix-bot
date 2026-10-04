//! System prompts and scratchpad formatting for autonomous ReAct execution.

use crate::domain::agent::AgentStep;
use crate::infrastructure::inference::llm::{ChatCompletionMessage, ToolDefinition};

pub const AGENT_SYSTEM_PROMPT: &str = "\
You are an autonomous AI execution agent for matrix-bot.
Your goal is to accomplish the user's objective systematically and autonomously.

At each step, follow the ReAct (Reasoning + Acting) loop:
1. Reason carefully about your current progress and the next action needed.
   Start with: Thought: <your reasoning>
2. If you need more information or need to take an action:
   Output:
   Action: <tool_name>
   Action Input: <valid json arguments>
3. If you have gathered all necessary information or completed the task:
   Output:
   Thought: I have completed the goal.
   Final Answer: <concise, structured summary of findings and actions taken>

Rules:
- Only call tools relevant to the user's goal.
- Sensitive operations (financial transactions, trades, deletions) will require human confirmation before execution.
- Be concise, accurate, and objective. Never invent data.";

pub const MAX_TOOL_OUTPUT_PROMPT_CHARS: usize = 2000;

/// Formats a tool output for insertion into the ReAct prompt observation,
/// truncating at [`MAX_TOOL_OUTPUT_PROMPT_CHARS`] to avoid blowing up the context window (R169).
pub fn format_tool_output(output: &serde_json::Value) -> String {
    let raw = match output {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if raw.chars().count() > MAX_TOOL_OUTPUT_PROMPT_CHARS {
        let truncated: String = raw.chars().take(MAX_TOOL_OUTPUT_PROMPT_CHARS).collect();
        format!(
            "{}\n... [output truncated for prompt length; full output stored in step record]",
            truncated
        )
    } else {
        raw
    }
}

/// Parsed output from a single ReAct step evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedReact {
    Action {
        thought: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    FinalAnswer {
        thought: Option<String>,
        final_answer: String,
    },
}

/// Builds the message list for the LLM chat completion request.
pub fn build_messages(
    goal: &str,
    steps: &[AgentStep],
    allowed_tools: &[ToolDefinition],
) -> Vec<ChatCompletionMessage> {
    let mut messages = Vec::new();

    // 1. System prompt
    let tools_list: Vec<String> = allowed_tools
        .iter()
        .map(|t| format!("- {}: {}", t.function.name, t.function.description))
        .collect();

    let system_content = if tools_list.is_empty() {
        AGENT_SYSTEM_PROMPT.to_string()
    } else {
        format!(
            "{}\n\nAvailable tools:\n{}",
            AGENT_SYSTEM_PROMPT,
            tools_list.join("\n")
        )
    };

    messages.push(ChatCompletionMessage {
        role: "system".to_string(),
        content: Some(system_content),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    // 2. Initial user goal
    messages.push(ChatCompletionMessage {
        role: "user".to_string(),
        content: Some(format!("Goal: {}", goal)),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    // 3. Conversation history from past steps
    for step in steps {
        let mut assistant_parts = Vec::new();
        if let Some(thought) = &step.thought {
            assistant_parts.push(format!("Thought: {}", thought));
        }
        if let Some(tool_name) = &step.tool_name {
            assistant_parts.push(format!("Action: {}", tool_name));
            if let Some(input) = &step.tool_input {
                assistant_parts.push(format!("Action Input: {}", input));
            }
        }

        if !assistant_parts.is_empty() {
            messages.push(ChatCompletionMessage {
                role: "assistant".to_string(),
                content: Some(assistant_parts.join("\n")),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        if let Some(output) = &step.tool_output {
            messages.push(ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(format!("Observation: {}", format_tool_output(output))),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }
    }

    // 4. Prompt to proceed with the next step
    messages.push(ChatCompletionMessage {
        role: "user".to_string(),
        content: Some(
            "Proceed to the next step. State your Thought and either an Action or Final Answer."
                .to_string(),
        ),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });

    messages
}

/// Parses the model text response into a structured `ParsedReact`.
pub fn parse_react_response(text: &str) -> ParsedReact {
    let trimmed = text.trim();

    // Check for Final Answer:
    if let Some(idx) = trimmed.find("Final Answer:") {
        let before = trimmed[..idx].trim();
        let after = trimmed[idx + "Final Answer:".len()..].trim();

        let thought = extract_thought(before);
        return ParsedReact::FinalAnswer {
            thought,
            final_answer: after.to_string(),
        };
    }

    // Check for Action: and Action Input:
    if let Some(action_idx) = trimmed.find("Action:") {
        let before_action = trimmed[..action_idx].trim();
        let thought = extract_thought(before_action);

        let after_action = &trimmed[action_idx + "Action:".len()..];
        let (tool_name, tool_input) = if let Some(input_idx) = after_action.find("Action Input:") {
            let name = after_action[..input_idx].trim();
            let input_str = after_action[input_idx + "Action Input:".len()..].trim();
            let input_val = parse_json_or_value(input_str);
            (name, input_val)
        } else {
            let name = after_action.lines().next().unwrap_or("").trim();
            (name, serde_json::json!({}))
        };

        if !tool_name.is_empty() {
            return ParsedReact::Action {
                thought,
                tool_name: tool_name.to_string(),
                tool_input,
            };
        }
    }

    // Fallback: entire text is treated as final answer
    let thought = extract_thought(trimmed);
    ParsedReact::FinalAnswer {
        thought: None,
        final_answer: if let Some(t) = thought {
            t
        } else {
            trimmed.to_string()
        },
    }
}

fn extract_thought(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(stripped) = trimmed.strip_prefix("Thought:") {
        let t = stripped.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_json_or_value(s: &str) -> serde_json::Value {
    let trimmed = s.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        v
    } else {
        // Strip markdown code fences if present
        let clean = if trimmed.starts_with("```json") {
            trimmed
                .trim_start_matches("```json")
                .trim_end_matches("```")
                .trim()
        } else if trimmed.starts_with("```") {
            trimmed
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim()
        } else {
            trimmed
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(clean) {
            v
        } else {
            serde_json::json!({ "query": trimmed })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_action_and_thought_correctly() {
        let text = "\
Thought: I need to check the user's bookmarks for rust articles.
Action: get_my_bookmarks
Action Input: {\"query\": \"rust\", \"limit\": 5}";

        let parsed = parse_react_response(text);
        assert_eq!(
            parsed,
            ParsedReact::Action {
                thought: Some(
                    "I need to check the user's bookmarks for rust articles.".to_string()
                ),
                tool_name: "get_my_bookmarks".to_string(),
                tool_input: serde_json::json!({"query": "rust", "limit": 5}),
            }
        );
    }

    #[test]
    fn parses_final_answer_and_thought() {
        let text = "\
Thought: I have summarized all findings.
Final Answer: Here is your weekly digest of 3 articles.";

        let parsed = parse_react_response(text);
        assert_eq!(
            parsed,
            ParsedReact::FinalAnswer {
                thought: Some("I have summarized all findings.".to_string()),
                final_answer: "Here is your weekly digest of 3 articles.".to_string(),
            }
        );
    }

    #[test]
    fn parses_action_with_markdown_json_block() {
        let text = "\
Thought: Adding todo task.
Action: add_todo
Action Input:
```json
{\"title\": \"Review PR\"}
```";

        let parsed = parse_react_response(text);
        assert_eq!(
            parsed,
            ParsedReact::Action {
                thought: Some("Adding todo task.".to_string()),
                tool_name: "add_todo".to_string(),
                tool_input: serde_json::json!({"title": "Review PR"}),
            }
        );
    }

    #[test]
    fn truncates_oversized_tool_output_in_messages() {
        use crate::domain::agent::{AgentRunId, StepStatus};
        use chrono::Utc;

        let big_payload = "A".repeat(3000);
        let step = AgentStep::new(
            AgentRunId::generate(),
            1,
            Some("Calling big tool".into()),
            Some("fetch_data".into()),
            Some(serde_json::json!({})),
            Some(serde_json::json!(big_payload)),
            StepStatus::Success,
            10,
            Utc::now(),
        );

        let messages = build_messages("test goal", &[step], &[]);
        let observation = messages
            .iter()
            .find(|m| {
                m.role == "user"
                    && m.content
                        .as_deref()
                        .unwrap_or("")
                        .starts_with("Observation:")
            })
            .expect("observation message found");

        let content = observation.content.as_ref().unwrap();
        assert!(content.contains(
            "... [output truncated for prompt length; full output stored in step record]"
        ));
        assert!(content.starts_with(&format!("Observation: {}", "A".repeat(100))));
        assert!(content.len() < 2200);
    }
}
