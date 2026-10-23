//! AI tools for todo checklist steps. The owner always comes from the
//! authenticated caller; no argument can name one.

use crate::domain::accounts::AccountId;
use crate::infrastructure::inference::llm::ToolDefinition;
use serde_json::json;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    owner_id: &AccountId,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "add_todo_step" => Some(add_step(services, owner_id, args).await),
        "set_todo_step_done" => Some(set_step_done(services, owner_id, args).await),
        _ => None,
    }
}

fn todo_id(args: &serde_json::Value) -> Result<i64, String> {
    args.get("todo_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "Field 'todo_id' is required".to_string())
}

async fn add_step(
    services: &crate::application::AppServices,
    owner_id: &AccountId,
    args: &serde_json::Value,
) -> Result<String, String> {
    let id = todo_id(args)?;
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or("Field 'text' is required")?;
    let with_steps = services
        .todos
        .add_step(owner_id, id, text)
        .await
        .map_err(|e| e.to_string())?;
    step_view(&with_steps)
}

async fn set_step_done(
    services: &crate::application::AppServices,
    owner_id: &AccountId,
    args: &serde_json::Value,
) -> Result<String, String> {
    let id = todo_id(args)?;
    let position = args
        .get("position")
        .and_then(|v| v.as_u64())
        .ok_or("Field 'position' is required (1-based integer)")?;
    let done = args
        .get("done")
        .and_then(|v| v.as_bool())
        .ok_or("Field 'done' is required")?;
    let current = services
        .todos
        .get_with_steps(owner_id, id)
        .await
        .map_err(|e| e.to_string())?;
    let step = usize::try_from(position)
        .ok()
        .and_then(|p| current.step_at(p))
        .ok_or_else(|| format!("No step {position} on task #{id}"))?;
    let updated = services
        .todos
        .update_step(owner_id, id, step.id, None, Some(done))
        .await
        .map_err(|e| e.to_string())?;
    step_view(&updated)
}

/// The todo with its steps numbered from 1, the way the model addresses them.
fn step_view(t: &crate::domain::todos::steps::TodoWithSteps) -> Result<String, String> {
    let steps: Vec<_> = t
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| json!({ "position": i + 1, "text": s.text, "done": s.done }))
        .collect();
    serde_json::to_string(&json!({ "todo": t.todo, "steps": steps })).map_err(|e| e.to_string())
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "add_todo_step",
            "Append a checklist step to an existing task.",
            json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "integer", "description": "The numeric ID of the task" },
                    "text": { "type": "string", "description": "The step description" }
                },
                "required": ["todo_id", "text"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "set_todo_step_done",
            "Mark a checklist step of a task done or not done, addressed by its 1-based position.",
            json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "integer", "description": "The numeric ID of the task" },
                    "position": { "type": "integer", "minimum": 1, "description": "1-based position of the step in the checklist" },
                    "done": { "type": "boolean", "description": "true to check the step off, false to reopen it" }
                },
                "required": ["todo_id", "position", "done"],
                "additionalProperties": false
            }),
        ),
    ]
}
