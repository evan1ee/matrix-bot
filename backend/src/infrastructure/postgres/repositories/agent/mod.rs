//! PostgreSQL repository for autonomous agent execution runs, granular steps,
//! approvals, and scheduled tasks.

pub mod approvals;
pub mod runs;
pub mod steps;
pub mod tasks;

pub use approvals::*;
pub use runs::*;
pub use steps::*;
pub use tasks::*;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    use crate::domain::accounts::AccountId;
    use crate::domain::agent::{
        AgentRun, AgentStep, ApprovalRequest, ApprovalStatus, RunStatus, ScheduledTaskDefinition,
        StepStatus, TaskId,
    };
    use crate::utils::time::now_at_storage_precision;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn agent_run_and_step_persistence_lifecycle() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_repo_run");
        crate::db::pool::seed_account(&pool, &owner).await;
        let now = now_at_storage_precision();

        let run = AgentRun::new(
            owner,
            "research_task",
            "Research Rust async runtime",
            Some("matrix:!room:server".to_string()),
            Some(10),
            now,
        )
        .unwrap();

        let created = create_run(&pool, &run).await.unwrap();
        assert_eq!(created.id, run.id);
        assert_eq!(created.status, RunStatus::Planning);

        let active = find_active_run_for_account(&pool, &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.id, run.id);

        // Record a step
        let step = AgentStep::new(
            run.id,
            1,
            Some("Need to search documentation".to_string()),
            Some("search_web".to_string()),
            Some(serde_json::json!({"query": "tokio runtime"})),
            Some(serde_json::json!({"result": "Found docs"})),
            StepStatus::Success,
            120,
            now,
        );
        let recorded = record_step(&pool, &step).await.unwrap();
        assert_eq!(recorded.step_number, 1);
        assert_eq!(recorded.tokens_used, 120);

        let steps = get_run_steps(&pool, run.id).await.unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].thought.as_deref(),
            Some("Need to search documentation")
        );

        // Update run status
        let updated = update_run_status(
            &pool,
            run.id,
            RunStatus::Completed,
            1,
            120,
            None,
            Some(now),
            now,
        )
        .await
        .unwrap();
        assert!(updated);

        // Run is no longer active
        let no_active = find_active_run_for_account(&pool, &owner).await.unwrap();
        assert!(no_active.is_none());
    }

    #[tokio::test]
    async fn approval_request_round_trip_and_resolution() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_repo_appr");
        crate::db::pool::seed_account(&pool, &owner).await;
        let now = now_at_storage_precision();

        let run = AgentRun::new(owner, "task", "Goal", None, None, now).unwrap();
        create_run(&pool, &run).await.unwrap();

        let approval = ApprovalRequest::new(
            run.id,
            owner,
            "portfolio_execute_trade",
            serde_json::json!({"symbol": "BTC", "amount": 1}),
            now,
        );
        let created = create_approval(&pool, &approval).await.unwrap();
        assert_eq!(created.status, ApprovalStatus::Pending);

        let found = find_pending_approval_for_account(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.tool_name, "portfolio_execute_trade");

        // Resolve
        let resolved = resolve_approval(&pool, created.id, ApprovalStatus::Approved, now)
            .await
            .unwrap();
        assert!(resolved);

        // No longer pending
        assert!(find_pending_approval(&pool, created.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn scheduled_task_due_sweep_and_owner_isolation() {
        let pool = setup_test_db().await;
        let owner1 = AccountId::from_seed("acct_agent_sched_1");
        let owner2 = AccountId::from_seed("acct_agent_sched_2");
        crate::db::pool::seed_account(&pool, &owner1).await;
        crate::db::pool::seed_account(&pool, &owner2).await;
        let now = now_at_storage_precision();

        let task1 = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: owner1,
            task_id: TaskId::new("weekly_digest").unwrap(),
            cron_expression: "0 21 * * SUN".to_string(),
            enabled: true,
            config: serde_json::json!({}),
            last_run_at: None,
            next_run_at: now - chrono::Duration::minutes(5), // due
            created_at: now,
            updated_at: now,
        };
        upsert_scheduled_task(&pool, &task1).await.unwrap();

        let task2 = ScheduledTaskDefinition {
            id: Uuid::new_v4(),
            account_id: owner2,
            task_id: TaskId::new("financial_audit").unwrap(),
            cron_expression: "0 8 1 * *".to_string(),
            enabled: false, // disabled
            config: serde_json::json!({}),
            last_run_at: None,
            next_run_at: now - chrono::Duration::minutes(10),
            created_at: now,
            updated_at: now,
        };
        upsert_scheduled_task(&pool, &task2).await.unwrap();

        // Due sweep only returns enabled due tasks
        let mut tx = pool.begin().await.unwrap();
        let due = list_due_scheduled_tasks(&mut *tx, now, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].task_id.as_str(), "weekly_digest");
        tx.rollback().await.unwrap();

        // Owner isolation: list tasks for owner1 returns only task1
        let list1 = list_scheduled_tasks_for_account(&pool, &owner1)
            .await
            .unwrap();
        assert_eq!(list1.len(), 1);
        assert_eq!(list1[0].account_id, owner1);

        let list2 = list_scheduled_tasks_for_account(&pool, &owner2)
            .await
            .unwrap();
        assert_eq!(list2.len(), 1);
        assert_eq!(list2[0].account_id, owner2);
    }

    /// Given a task, When a stranger toggles or deletes it, Then nothing changes;
    /// When the owner pauses with no next run, Then the schedule is kept.
    #[tokio::test]
    async fn task_state_and_delete_are_owner_scoped() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_agent_state_1");
        let stranger = AccountId::from_seed("acct_agent_state_2");
        crate::db::pool::seed_account(&pool, &owner).await;
        crate::db::pool::seed_account(&pool, &stranger).await;
        let now = now_at_storage_precision();
        let id = TaskId::new("weekly_digest").unwrap();
        let due = now + chrono::Duration::hours(3);
        upsert_scheduled_task(
            &pool,
            &ScheduledTaskDefinition {
                id: Uuid::new_v4(),
                account_id: owner,
                task_id: id.clone(),
                cron_expression: "0 21 * * SUN".to_string(),
                enabled: true,
                config: serde_json::json!({}),
                last_run_at: None,
                next_run_at: due,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();

        assert!(
            set_scheduled_task_state(&pool, &stranger, &id, false, None, now)
                .await
                .unwrap()
                .is_none()
        );
        assert!(!delete_scheduled_task(&pool, &stranger, &id).await.unwrap());
        let untouched = find_scheduled_task(&pool, &owner, &id)
            .await
            .unwrap()
            .unwrap();
        assert!(untouched.enabled);

        let paused = set_scheduled_task_state(&pool, &owner, &id, false, None, now)
            .await
            .unwrap()
            .unwrap();
        assert!(!paused.enabled);
        assert_eq!(paused.next_run_at, due);

        let later = now + chrono::Duration::days(2);
        let resumed = set_scheduled_task_state(&pool, &owner, &id, true, Some(later), now)
            .await
            .unwrap()
            .unwrap();
        assert!(resumed.enabled);
        assert_eq!(resumed.next_run_at, later);

        assert!(delete_scheduled_task(&pool, &owner, &id).await.unwrap());
        assert!(find_scheduled_task(&pool, &owner, &id)
            .await
            .unwrap()
            .is_none());
    }
}
