use super::{
    bills, bookmarks, finance, habits, life, portfolio, reminder, settings, todo, webhooks,
};
use crate::infrastructure::inference::llm::ToolDefinition;

/// Aggregates all 47 domain-grouped AI function calling tool definitions.
pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    [
        todo::tool_definitions(),
        finance::tool_definitions(),
        reminder::tool_definitions(),
        portfolio::tool_definitions(),
        bills::tool_definitions(),
        habits::tool_definitions(),
        life::tool_definitions(),
        bookmarks::tool_definitions(),
        webhooks::tool_definitions(),
        settings::tool_definitions(),
    ]
    .concat()
}
