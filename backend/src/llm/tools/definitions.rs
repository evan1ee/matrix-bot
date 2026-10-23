use super::{
    bills, bookmarks, finance, habits, knowledge, life, portfolio, reminder, settings, todo,
    todo_steps, webhooks,
};
use crate::infrastructure::inference::llm::ToolDefinition;

/// Aggregates all domain-grouped AI function calling tool definitions.
pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    [
        todo::tool_definitions(),
        todo_steps::tool_definitions(),
        finance::tool_definitions(),
        reminder::tool_definitions(),
        portfolio::tool_definitions(),
        bills::tool_definitions(),
        habits::tool_definitions(),
        life::tool_definitions(),
        bookmarks::tool_definitions(),
        knowledge::tool_definitions(),
        webhooks::tool_definitions(),
        settings::tool_definitions(),
    ]
    .concat()
}
