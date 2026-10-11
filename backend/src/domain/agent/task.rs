//! Extensible task plugin framework for autonomous agent recurring jobs.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::run::USER_GOAL_TASK;
use super::{AgentError, AgentRun};
use crate::domain::accounts::AccountId;

/// Validated identifier for a scheduled or built-in task plugin (e.g. `weekly_bookmark_digest`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    /// Creates a validated `TaskId`.
    pub fn new(s: impl AsRef<str>) -> Result<Self, AgentError> {
        let s = s.as_ref().trim();
        if s.len() < 3 || s.len() > 64 {
            return Err(AgentError::InvalidTaskId(
                "task_id must be between 3 and 64 characters".to_string(),
            ));
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(AgentError::InvalidTaskId(
                "task_id must consist only of lowercase alphanumeric characters, '_', or '-'"
                    .to_string(),
            ));
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TaskId {
    type Err = AgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Definition for a scheduled recurring agent task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTaskDefinition {
    pub id: Uuid,
    pub account_id: AccountId,
    pub task_id: TaskId,
    pub cron_expression: String,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Trait defining a scheduled autonomous agent task plugin.
///
/// Built-in and custom extensions implement this trait to provide task identity,
/// default scheduling cadence, tool sandboxing whitelists, and goal formulation.
pub trait ScheduledAgentTask: Send + Sync {
    /// Unique validated identifier for this task (e.g. "weekly_bookmark_digest").
    fn task_id(&self) -> TaskId;

    /// Human-readable title of this task.
    fn title(&self) -> &'static str;

    /// Default cron expression (e.g. "0 9 * * 1" for Mondays at 9am).
    fn default_cron(&self) -> &'static str;

    /// Whitelist of tool names this task is permitted to invoke.
    /// Returning `None` permits all safe tools.
    fn allowed_tools(&self) -> Option<Vec<String>> {
        None
    }

    /// Formulates the agent goal prompt given user-supplied JSON configuration.
    fn formulate_goal(&self, config: &serde_json::Value) -> String;
}

/// In-memory registry of available `ScheduledAgentTask` plugins.
#[derive(Default, Clone)]
pub struct ScheduledTaskRegistry {
    tasks: BTreeMap<TaskId, Arc<dyn ScheduledAgentTask>>,
}

impl ScheduledTaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
        }
    }

    /// Creates a registry pre-loaded with built-in task plugin definitions.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(WeeklyBookmarkDigestTask));
        registry.register(Arc::new(MonthlyFinancialAuditTask));
        registry.register(Arc::new(PortfolioRebalanceAlertTask));
        registry.register(Arc::new(BillDueDigestTask));
        registry.register(Arc::new(HabitConsistencyReviewTask));
        registry
    }

    /// Registers a new task plugin definition.
    pub fn register(&mut self, task: Arc<dyn ScheduledAgentTask>) {
        self.tasks.insert(task.task_id(), task);
    }

    /// Fetches a task plugin definition by its ID.
    pub fn get(&self, task_id: &TaskId) -> Option<Arc<dyn ScheduledAgentTask>> {
        self.tasks.get(task_id).cloned()
    }

    /// The tools `run` may call.
    ///
    /// A user-started run is not restricted here (`None`): sensitive tools
    /// still need the user's approval. A user run is recognised by its name,
    /// not by a missing `scheduled_task_id`, because that link is nulled when
    /// the schedule row is deleted and must not turn a scheduled run into a
    /// user run. Every other run is unattended: it runs on
    /// prompts that can carry untrusted content, so it is held to its task's
    /// whitelist. If its task is no longer registered, or its stored name is
    /// not a valid id, it gets no tools at all rather than every tool.
    pub fn tool_policy(&self, run: &AgentRun) -> Option<Vec<String>> {
        if run.scheduled_task_id.is_none() && run.task_name == USER_GOAL_TASK {
            return None;
        }
        let task = TaskId::new(&run.task_name)
            .ok()
            .and_then(|id| self.get(&id));
        match task {
            Some(task) => task.allowed_tools(),
            None => Some(Vec::new()),
        }
    }

    /// Lists all registered task plugin definitions.
    pub fn list(&self) -> Vec<Arc<dyn ScheduledAgentTask>> {
        self.tasks.values().cloned().collect()
    }
}

// ===========================================================================
// Built-in task plugins. Each one is a goal prompt plus a tool whitelist.
// ===========================================================================

/// [TODO Extension] Weekly digest summarizing unread bookmarks with AI metadata.
pub struct WeeklyBookmarkDigestTask;

impl ScheduledAgentTask for WeeklyBookmarkDigestTask {
    fn task_id(&self) -> TaskId {
        TaskId::new("weekly_bookmark_digest").expect("valid task id")
    }

    fn title(&self) -> &'static str {
        "Weekly Bookmark Digest"
    }

    fn default_cron(&self) -> &'static str {
        // Every Monday at 09:00 UTC
        "0 9 * * 1"
    }

    fn allowed_tools(&self) -> Option<Vec<String>> {
        Some(vec![
            "get_my_bookmarks".to_string(),
            "toggle_bookmark_favorite".to_string(),
            "archive_bookmark".to_string(),
        ])
    }

    fn formulate_goal(&self, config: &serde_json::Value) -> String {
        let limit = config.get("limit").and_then(|v| v.as_i64()).unwrap_or(10);
        let tag_filter = config.get("tag").and_then(|v| v.as_str()).unwrap_or("");

        if tag_filter.is_empty() {
            format!(
                "Review the user's unread bookmarks (up to {} items). \
                 Synthesize an insightful weekly digest categorizing articles by topic \
                 and highlighting top key takeaways.",
                limit
            )
        } else {
            format!(
                "Review the user's unread bookmarks tagged with '{}' (up to {} items). \
                 Synthesize an insightful weekly digest highlighting top key takeaways.",
                tag_filter, limit
            )
        }
    }
}

/// [TODO Extension] Monthly financial spending audit and budget review.
pub struct MonthlyFinancialAuditTask;

impl ScheduledAgentTask for MonthlyFinancialAuditTask {
    fn task_id(&self) -> TaskId {
        TaskId::new("monthly_financial_audit").expect("valid task id")
    }

    fn title(&self) -> &'static str {
        "Monthly Financial Audit"
    }

    fn default_cron(&self) -> &'static str {
        // 1st day of every month at 08:00 UTC
        "0 8 1 * *"
    }

    fn allowed_tools(&self) -> Option<Vec<String>> {
        Some(vec![
            "list_finance_transactions".to_string(),
            "get_spending_by_category".to_string(),
            "get_my_recurring_bills".to_string(),
        ])
    }

    fn formulate_goal(&self, _config: &serde_json::Value) -> String {
        "Analyze the user's financial transactions and spending by category for the past month. \
         Identify spending spikes, compare against recurring bills, and highlight savings opportunities."
            .to_string()
    }
}

/// [TODO Extension] Portfolio holdings drift and price alert evaluation.
pub struct PortfolioRebalanceAlertTask;

impl ScheduledAgentTask for PortfolioRebalanceAlertTask {
    fn task_id(&self) -> TaskId {
        TaskId::new("portfolio_rebalance_check").expect("valid task id")
    }

    fn title(&self) -> &'static str {
        "Portfolio Rebalance Evaluation"
    }

    fn default_cron(&self) -> &'static str {
        // Daily at market close (21:00 UTC)
        "0 21 * * *"
    }

    fn allowed_tools(&self) -> Option<Vec<String>> {
        Some(vec![
            "get_portfolio_holding".to_string(),
            "get_portfolio_performance_history".to_string(),
            "get_portfolio_price_alerts".to_string(),
        ])
    }

    fn formulate_goal(&self, _config: &serde_json::Value) -> String {
        "Inspect all portfolio holdings, calculate current asset allocation weights, \
         and evaluate target allocation drift. Do NOT execute any trades directly without explicit approval."
            .to_string()
    }
}

/// [TODO Extension] Recurring bill due date digest and payment preparation.
pub struct BillDueDigestTask;

impl ScheduledAgentTask for BillDueDigestTask {
    fn task_id(&self) -> TaskId {
        TaskId::new("bill_due_digest").expect("valid task id")
    }

    fn title(&self) -> &'static str {
        "Upcoming Bills Digest"
    }

    fn default_cron(&self) -> &'static str {
        // Every Friday at 10:00 UTC
        "0 10 * * 5"
    }

    fn allowed_tools(&self) -> Option<Vec<String>> {
        Some(vec![
            "get_my_recurring_bills".to_string(),
            "list_finance_transactions".to_string(),
        ])
    }

    fn formulate_goal(&self, _config: &serde_json::Value) -> String {
        "Check upcoming recurring bills due in the next 14 days. \
         List due dates, amounts, and verify sufficient account cash balance to cover them."
            .to_string()
    }
}

/// [TODO Extension] Weekly habit completion streak and consistency review.
pub struct HabitConsistencyReviewTask;

impl ScheduledAgentTask for HabitConsistencyReviewTask {
    fn task_id(&self) -> TaskId {
        TaskId::new("habit_consistency_review").expect("valid task id")
    }

    fn title(&self) -> &'static str {
        "Habit Consistency Review"
    }

    fn default_cron(&self) -> &'static str {
        // Every Sunday at 20:00 UTC
        "0 20 * * 0"
    }

    fn allowed_tools(&self) -> Option<Vec<String>> {
        Some(vec![
            "get_my_habits".to_string(),
            "get_daily_log".to_string(),
        ])
    }

    fn formulate_goal(&self, _config: &serde_json::Value) -> String {
        "Review the user's habit completion logs and streaks for the past 7 days. \
         Celebrate wins, identify slipping habits, and propose a lightweight improvement strategy."
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_validation_rules() {
        assert!(TaskId::new("weekly_digest").is_ok());
        assert!(TaskId::new("financial-audit-1").is_ok());
        assert!(TaskId::new("ab").is_err()); // too short
        assert!(TaskId::new("Uppercase").is_err()); // no uppercase
        assert!(TaskId::new("has space").is_err());
        assert!(TaskId::new("special!").is_err());
    }

    #[test]
    fn builtin_registry_contains_planned_tasks() {
        let registry = ScheduledTaskRegistry::with_builtins();
        let tasks = registry.list();
        assert_eq!(tasks.len(), 5);

        let bookmark_task_id = TaskId::new("weekly_bookmark_digest").unwrap();
        let bookmark_task = registry.get(&bookmark_task_id).expect("task found");
        assert_eq!(bookmark_task.title(), "Weekly Bookmark Digest");
        assert_eq!(bookmark_task.default_cron(), "0 9 * * 1");

        let goal = bookmark_task.formulate_goal(&serde_json::json!({"limit": 5, "tag": "rust"}));
        assert!(goal.contains("tagged with 'rust'"));
        assert!(goal.contains("5 items"));
    }

    use crate::domain::agent::AgentRun;

    fn run_for(task_name: &str, scheduled: bool) -> AgentRun {
        let mut run = AgentRun::new(
            AccountId::from_seed("acct_task_policy"),
            task_name,
            "Goal",
            None,
            None,
            Utc::now(),
        )
        .unwrap();
        run.scheduled_task_id = scheduled.then(Uuid::new_v4);
        run
    }

    #[test]
    fn user_run_is_not_restricted() {
        let registry = ScheduledTaskRegistry::with_builtins();
        assert_eq!(registry.tool_policy(&run_for(USER_GOAL_TASK, false)), None);
    }

    #[test]
    fn scheduled_run_is_held_to_its_task_whitelist() {
        let registry = ScheduledTaskRegistry::with_builtins();
        assert_eq!(
            registry.tool_policy(&run_for("habit_consistency_review", true)),
            Some(vec![
                "get_my_habits".to_string(),
                "get_daily_log".to_string()
            ])
        );
    }

    #[test]
    fn a_run_that_lost_its_schedule_link_stays_restricted() {
        // `scheduled_task_id` is ON DELETE SET NULL (an account merge drops
        // the absorbed account's schedule rows), so it cannot tell a user run
        // from an orphaned scheduled one.
        let registry = ScheduledTaskRegistry::with_builtins();
        let mut orphan = run_for("habit_consistency_review", true);
        orphan.scheduled_task_id = None;
        assert_eq!(
            registry.tool_policy(&orphan),
            Some(vec![
                "get_my_habits".to_string(),
                "get_daily_log".to_string()
            ])
        );
    }

    #[test]
    fn unknown_scheduled_task_gets_no_tools() {
        let registry = ScheduledTaskRegistry::with_builtins();
        assert_eq!(
            registry.tool_policy(&run_for("removed_task", true)),
            Some(Vec::new())
        );
        assert_eq!(
            registry.tool_policy(&run_for("Not A Valid Id!", true)),
            Some(Vec::new())
        );
    }
}
