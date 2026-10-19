//! Checklist step chat commands: `!todo show <id>` and `!todo step <id> ...`.

use chrono_tz::Tz;

use super::tags::{failure, tags_code};
use crate::application::todos::TodoService;
use crate::domain::accounts::AccountId;
use crate::domain::reminders::RecurrencePattern;
use crate::domain::todos::steps::TodoWithSteps;
use crate::utils::{format_datetime, html_escape, priority_badge};

#[derive(Debug, PartialEq, Eq)]
pub enum StepCmd {
    Add(String),
    Done(usize),
    Undo(usize),
    Remove(usize),
}

pub const STEP_USAGE: &str = "⚠️ Usage: <code>!todo step &lt;id&gt; add &lt;text&gt;</code>,      <code>done &lt;n&gt;</code>, <code>undo &lt;n&gt;</code> or <code>rm &lt;n&gt;</code>";
const SHOW_USAGE: &str = "⚠️ Usage: <code>!todo show &lt;id&gt;</code>";

/// The words after `!todo step <id>`. Steps are addressed by the 1-based
/// position `show` displays.
pub fn parse_step_args(rest: &str) -> Result<StepCmd, &'static str> {
    let rest = rest.trim();
    let (verb, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let arg = arg.trim();
    let position = || {
        arg.parse::<usize>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or(STEP_USAGE)
    };
    match verb.to_lowercase().as_str() {
        "add" if !arg.is_empty() => Ok(StepCmd::Add(arg.to_string())),
        "done" => position().map(StepCmd::Done),
        "undo" => position().map(StepCmd::Undo),
        "rm" => position().map(StepCmd::Remove),
        _ => Err(STEP_USAGE),
    }
}

/// A todo with its checklist, numbered as the step commands address it.
pub fn format_with_steps(view: &TodoWithSteps, tz: &Tz) -> String {
    let todo = &view.todo;
    let mut html = format!(
        "📌 <b>#{}</b> {} {}",
        todo.id,
        html_escape(&todo.text),
        priority_badge(todo.priority.as_str())
    );
    if let Some(due) = &todo.due_at {
        html.push_str(&format!(
            " 📅 <i>(Due: {})</i>",
            html_escape(&format_datetime(due, tz))
        ));
    }
    if todo.recurrence != RecurrencePattern::None {
        html.push_str(&format!(" ↻ {}", todo.recurrence.as_str()));
    }
    if !todo.tags.is_empty() {
        html.push_str(&format!(" {}", tags_code(&todo.tags)));
    }
    if view.steps.is_empty() {
        html.push_str(&format!(
            "<br/>No steps yet. Add one with <code>!todo step {} add &lt;text&gt;</code>.",
            todo.id
        ));
        return html;
    }
    let done = view.steps.iter().filter(|s| s.done).count();
    html.push_str(&format!("<br/>Steps {}/{}:", done, view.steps.len()));
    for (i, step) in view.steps.iter().enumerate() {
        let text = html_escape(&step.text);
        if step.done {
            html.push_str(&format!("<br/>☑ {}. <del>{}</del>", i + 1, text));
        } else {
            html.push_str(&format!("<br/>☐ {}. {}", i + 1, text));
        }
    }
    html
}

pub async fn handle_show(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
    tz: &Tz,
) -> Option<String> {
    let Ok(id) = args_body[4..].trim().parse::<i64>() else {
        return Some(SHOW_USAGE.to_string());
    };
    match todos.get_with_steps(owner_id, id).await {
        Ok(view) => Some(format_with_steps(&view, tz)),
        Err(err) => Some(failure(err, id)),
    }
}

pub async fn handle_step(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
    tz: &Tz,
) -> Option<String> {
    let rest = args_body[4..].trim();
    let (id_str, action) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let Ok(id) = id_str.parse::<i64>() else {
        return Some(STEP_USAGE.to_string());
    };
    let cmd = match parse_step_args(action) {
        Ok(cmd) => cmd,
        Err(usage) => return Some(usage.to_string()),
    };
    let result = match cmd {
        StepCmd::Add(text) => todos
            .add_step(owner_id, id, &text)
            .await
            .map_err(StepFailure::from),
        StepCmd::Done(n) => change_step(todos, owner_id, id, n, Some(true)).await,
        StepCmd::Undo(n) => change_step(todos, owner_id, id, n, Some(false)).await,
        StepCmd::Remove(n) => change_step(todos, owner_id, id, n, None).await,
    };
    match result {
        Ok(view) => Some(format_with_steps(&view, tz)),
        Err(StepFailure::Missing(n)) => {
            Some(format!("⚠️ Task <b>#{id}</b> has no step <b>{n}</b>."))
        }
        Err(StepFailure::Service(err)) => Some(failure(err, id)),
    }
}

enum StepFailure {
    Missing(usize),
    Service(crate::application::AppError),
}

impl From<crate::application::AppError> for StepFailure {
    fn from(err: crate::application::AppError) -> Self {
        StepFailure::Service(err)
    }
}

/// Resolves the displayed `position` to a step, then marks it done or undone
/// (`Some`) or removes it (`None`).
async fn change_step(
    todos: &TodoService,
    owner_id: &AccountId,
    todo_id: i64,
    position: usize,
    done: Option<bool>,
) -> Result<TodoWithSteps, StepFailure> {
    let view = todos.get_with_steps(owner_id, todo_id).await?;
    let step = view
        .step_at(position)
        .ok_or(StepFailure::Missing(position))?;
    Ok(match done {
        Some(done) => {
            todos
                .update_step(owner_id, todo_id, step.id, None, Some(done))
                .await?
        }
        None => todos.remove_step(owner_id, todo_id, step.id).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_args_parse() {
        assert_eq!(parse_step_args("done 2"), Ok(StepCmd::Done(2)));
        assert_eq!(parse_step_args("undo 1"), Ok(StepCmd::Undo(1)));
        assert_eq!(parse_step_args("rm 3"), Ok(StepCmd::Remove(3)));
        assert_eq!(
            parse_step_args("add buy milk"),
            Ok(StepCmd::Add("buy milk".to_string()))
        );
        assert_eq!(parse_step_args("rm x"), Err(STEP_USAGE));
        assert_eq!(parse_step_args("done 0"), Err(STEP_USAGE));
        assert_eq!(parse_step_args("add"), Err(STEP_USAGE));
        assert_eq!(parse_step_args("fly 1"), Err(STEP_USAGE));
        assert_eq!(parse_step_args(""), Err(STEP_USAGE));
    }
}
