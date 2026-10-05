//! One round of the chat tool loop: runs the model's tool calls concurrently
//! and refuses sensitive ones while the turn is tainted.

use tracing::info;

use super::context::LlmTurnContext;
use super::guard;
use super::tools::ToolRegistry;
use crate::infrastructure::inference::llm::ToolCall;

/// What running one tool call produced.
pub struct ToolOutcome {
    pub call_id: String,
    pub name: String,
    /// The raw result, or the error text; never wrapped.
    pub result: String,
    /// The tool ran and returned `Ok`.
    pub succeeded: bool,
    /// Not run: a sensitive tool in a tainted turn.
    pub refused: bool,
}

/// Runs every call of one round concurrently, in the model's order.
pub async fn run_calls(
    ctx: &LlmTurnContext<'_>,
    tool_calls: &[ToolCall],
    tainted: bool,
    round: usize,
    max_rounds: usize,
) -> Vec<ToolOutcome> {
    let futures = tool_calls.iter().map(|call| async move {
        let name = call.function.name.clone();
        if guard::must_confirm(&name, tainted) {
            info!(
                "LLM turn [{round}/{max_rounds}] refused sensitive tool {name} for owner {} until the user confirms",
                ctx.owner_id
            );
            return ToolOutcome {
                call_id: call.id.clone(),
                result: guard::confirmation_required_result(&name),
                name,
                succeeded: false,
                refused: true,
            };
        }
        info!(
            "LLM turn [{round}/{max_rounds}] executing tool: {name} for owner {}",
            ctx.owner_id
        );
        let outcome = ToolRegistry::execute_tool(
            ctx.services,
            ctx.pool,
            ctx.owner_id,
            ctx.room_id,
            &name,
            &call.function.arguments,
        )
        .await;
        let (result, succeeded) = match outcome {
            Ok(res) => (res, true),
            Err(err) => (format!("Error executing tool: {err}"), false),
        };
        ToolOutcome {
            call_id: call.id.clone(),
            name,
            result,
            succeeded,
            refused: false,
        }
    });
    futures_util::future::join_all(futures).await
}
