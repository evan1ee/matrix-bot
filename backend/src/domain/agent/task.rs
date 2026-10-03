//! Extensible task plugin framework for autonomous agent recurring jobs.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::TaskId;

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

    /// Lists all registered task plugin definitions.
    pub fn list(&self) -> Vec<Arc<dyn ScheduledAgentTask>> {
        self.tasks.values().cloned().collect()
    }
}

// ===========================================================================
// Built-in Task Plugins (Stubbed as TODO Extensions)
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
}
