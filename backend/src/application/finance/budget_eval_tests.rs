use chrono::{DateTime, Duration, Utc};

use super::budget_tests::{budget, setup};
use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::RecordedTransaction;
use crate::domain::finance::budget::{BudgetState, Month, NewBudget};
use crate::domain::finance::{Direction, NewTransaction, TransactionPatch};
use crate::infrastructure::postgres::repositories::finance::budgets as budgets_repo;
use crate::infrastructure::postgres::repositories::{self, outbox};
use crate::utils::time::now_at_storage_precision;

fn input<'a>(
    minor: i64,
    currency: &'a str,
    category: Option<&'a str>,
    occurred_at: DateTime<Utc>,
) -> NewTransaction<'a> {
    NewTransaction {
        direction: Direction::Expense,
        amount_minor: minor,
        currency,
        currency_exponent: 2,
        category,
        note: None,
        receipt_storage_key: None,
        room_id: "web",
        sender_name: "tester",
        occurred_at,
        source: None,
        create_missing_category: false,
    }
}

async fn record(
    services: &AppServices,
    owner: &AccountId,
    minor: i64,
    category: Option<&str>,
) -> RecordedTransaction {
    services
        .finance
        .record_transaction(
            owner,
            input(minor, "EUR", category, now_at_storage_precision()),
        )
        .await
        .unwrap()
}

async fn alerts(pool: &sqlx::PgPool) -> i64 {
    budgets_repo::count_alerts(pool).await.unwrap()
}

async fn alert_jobs(pool: &sqlx::PgPool) -> i64 {
    repositories::jobs::count_with_type(pool, "finance.budget_alert")
        .await
        .unwrap()
}

/// Only the budget's own creation event may remain after a rolled-back write.
async fn types_of(pool: &sqlx::PgPool, owner: &AccountId) -> Vec<String> {
    outbox::event_types_for(pool, owner).await.unwrap()
}

async fn crossed_events(pool: &sqlx::PgPool, owner: &AccountId) -> usize {
    outbox::event_types_for(pool, owner)
        .await
        .unwrap()
        .iter()
        .filter(|t| *t == "BudgetThresholdCrossed")
        .count()
}

async fn concurrent_setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
    let pool = crate::db::pool::test_pool_sized(4).await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let owner = AccountId::from_seed(seed);
    crate::db::pool::seed_account(&pool, &owner).await;
    let services = AppServices::new(&pool);
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    (pool, services, owner)
}

/// Given a 100.00 budget, When spending reaches 81.00, Then one 80 % alert, event and job exist.
#[tokio::test]
async fn crossing_80_records_alert_publishes_event_and_enqueues_job() {
    let (pool, services, owner) = setup("fin_eval_80").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();

    let first = record(&services, &owner, 5_000, None).await;
    let second = record(&services, &owner, 3_100, None).await;

    assert!(first.budgets.is_empty());
    assert_eq!(alerts(&pool).await, 1);
    assert_eq!(crossed_events(&pool, &owner).await, 1);
    assert_eq!(alert_jobs(&pool).await, 1);
    assert_eq!(second.budgets[0].state, BudgetState::Warning);
}

/// Given a budget, When one write crosses 80 and 100, Then both rows exist but one job (100).
#[tokio::test]
async fn one_write_crossing_both_records_both_and_enqueues_only_100() {
    let (pool, services, owner) = setup("fin_eval_both").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();

    let rec = record(&services, &owner, 10_500, None).await;

    assert_eq!(alerts(&pool).await, 2);
    assert_eq!(crossed_events(&pool, &owner).await, 1);
    assert_eq!(alert_jobs(&pool).await, 1);
    let keys = repositories::jobs::dedupe_keys_with_type(&pool, "finance.budget_alert")
        .await
        .unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].ends_with(":100"), "{keys:?}");
    assert_eq!(rec.budgets[0].state, BudgetState::Exceeded);
}

/// Given 70.00 spent, When two concurrent 12.00 expenses each cross 80 %, Then one alert.
#[tokio::test]
async fn concurrent_crossings_alert_once() {
    let (pool, services, owner) = concurrent_setup("fin_eval_concurrent").await;
    record(&services, &owner, 7_000, None).await;

    let (a, b) = tokio::join!(
        record(&services, &owner, 1_200, None),
        record(&services, &owner, 1_200, None)
    );

    assert!(!a.budgets.is_empty() && !b.budgets.is_empty());
    assert_eq!(alerts(&pool).await, 1);
    assert_eq!(alert_jobs(&pool).await, 1);
    assert_eq!(crossed_events(&pool, &owner).await, 1);
}

/// Given a 100.00 limit, When two concurrent 45.00 expenses (45 % alone, 90 % together)
/// commit, Then exactly one 80 % alert exists: the row lock serialises the sums.
#[tokio::test]
async fn concurrent_expenses_that_cross_together_alert() {
    let (pool, services, owner) = concurrent_setup("fin_eval_together").await;

    tokio::join!(
        record(&services, &owner, 4_500, None),
        record(&services, &owner, 4_500, None)
    );

    assert_eq!(alerts(&pool).await, 1);
    assert_eq!(alert_jobs(&pool).await, 1);
}

/// Given an over-limit expense dated last month, Then no alert, but last month shows Exceeded.
#[tokio::test]
async fn backdated_expense_never_alerts() {
    let (pool, services, owner) = setup("fin_eval_backdated").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    let when = now_at_storage_precision() - Duration::days(40);

    let rec = services
        .finance
        .record_transaction(&owner, input(12_000, "EUR", None, when))
        .await
        .unwrap();

    assert!(rec.budgets.is_empty());
    assert_eq!(alerts(&pool).await, 0);
    assert_eq!(alert_jobs(&pool).await, 0);
    let last = Month::containing(when, chrono_tz::UTC);
    let statuses = services
        .finance
        .budget_status(&owner, Some(last))
        .await
        .unwrap();
    assert_eq!(statuses[0].state, BudgetState::Exceeded);
}

/// Given a budget with alerts off, When it is exceeded, Then nothing is recorded or enqueued.
#[tokio::test]
async fn alerts_disabled_budget_never_alerts() {
    let (pool, services, owner) = setup("fin_eval_disabled").await;
    let silent = NewBudget::new(None, "EUR", 10_000, Some(false)).unwrap();
    services
        .finance
        .create_budget(&owner, silent)
        .await
        .unwrap();

    let rec = record(&services, &owner, 12_000, None).await;

    assert!(rec.budgets.is_empty());
    assert_eq!(alerts(&pool).await, 0);
    assert_eq!(alert_jobs(&pool).await, 0);
}

/// Given an overall and a category budget, When one expense hits both, Then each alerts once.
#[tokio::test]
async fn one_expense_can_alert_overall_and_category_budgets() {
    let (pool, services, owner) = setup("fin_eval_overall_cat").await;
    services
        .finance
        .create_category(&owner, "food")
        .await
        .unwrap();
    for cat in [None, Some("food")] {
        services
            .finance
            .create_budget(&owner, budget(cat, "EUR", 10_000))
            .await
            .unwrap();
    }

    let rec = record(&services, &owner, 8_500, Some("food")).await;

    assert_eq!(rec.budgets.len(), 2);
    assert_eq!(alerts(&pool).await, 2);
    assert_eq!(alert_jobs(&pool).await, 2);
    assert_eq!(crossed_events(&pool, &owner).await, 2);
}

/// Given an EUR budget, When an expense is edited into a large USD one, Then nothing alerts.
#[tokio::test]
async fn update_moving_expense_to_other_currency_does_not_alert_old_budget() {
    let (pool, services, owner) = setup("fin_eval_update_move").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    let rec = record(&services, &owner, 5_000, None).await;

    services
        .finance
        .update_transaction(
            &owner,
            rec.transaction.id,
            TransactionPatch {
                amount_minor: Some(9_500),
                currency: Some("USD"),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(alerts(&pool).await, 0);
    assert_eq!(alert_jobs(&pool).await, 0);
}

/// Given an edit that pushes spend over 80 %, Then the update itself alerts.
#[tokio::test]
async fn update_raising_amount_alerts() {
    let (pool, services, owner) = setup("fin_eval_update_raise").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    let rec = record(&services, &owner, 5_000, None).await;

    services
        .finance
        .update_transaction(
            &owner,
            rec.transaction.id,
            TransactionPatch {
                amount_minor: Some(8_200),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(alerts(&pool).await, 1);
    assert_eq!(alert_jobs(&pool).await, 1);
}

/// A 100.00 budget, then a 90.00 expense that must fail; asserts nothing of the
/// write survived: no transaction, alert row, job or threshold event.
async fn assert_crossing_write_rolls_back(
    pool: &sqlx::PgPool,
    services: &AppServices,
    owner: &AccountId,
) {
    let err = services
        .finance
        .record_transaction(owner, input(9_000, "EUR", None, now_at_storage_precision()))
        .await
        .unwrap_err();

    assert!(!matches!(err, AppError::Validation(_)), "{err:?}");
    assert_eq!(
        services
            .finance
            .count_transactions(owner, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(alerts(pool).await, 0);
    assert_eq!(alert_jobs(pool).await, 0);
    assert_eq!(crossed_events(pool, owner).await, 0);
}

/// Given every publish fails, When an expense is written, Then the very first publish
/// (TransactionCreated) aborts the write. Evaluation never runs in this case.
#[tokio::test]
async fn failed_first_outbox_publish_rolls_back_expense() {
    let (pool, services, owner) = setup("fin_eval_rollback_first").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    outbox::fail_every_publish(&pool).await;

    assert_crossing_write_rolls_back(&pool, &services, &owner).await;
}

/// Given only BudgetThresholdCrossed fails to publish, When an expense crosses 80 %,
/// Then the expense, its TransactionCreated event and the alert row all roll back.
#[tokio::test]
async fn failed_threshold_event_publish_rolls_back_expense_and_alert() {
    let (pool, services, owner) = setup("fin_eval_rollback_event").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    outbox::fail_threshold_crossed_publish(&pool).await;

    assert_crossing_write_rolls_back(&pool, &services, &owner).await;
    assert_eq!(types_of(&pool, &owner).await, vec!["BudgetCreated"]);
}

/// Given the alert job insert errors, When an expense crosses 80 %, Then the expense,
/// the alert row and the already published events roll back.
#[tokio::test]
async fn failed_alert_job_enqueue_rolls_back_expense_and_alert() {
    let (pool, services, owner) = setup("fin_eval_rollback_job").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 10_000))
        .await
        .unwrap();
    repositories::jobs::fail_every_enqueue(&pool).await;

    assert_crossing_write_rolls_back(&pool, &services, &owner).await;
    assert_eq!(types_of(&pool, &owner).await, vec!["BudgetCreated"]);
}

/// Given a budget, When a recurring bill is marked paid across it, Then the alert job exists.
#[tokio::test]
async fn recurring_bill_payment_triggers_evaluation() {
    use crate::domain::recurring_bills::NewRecurringBill;
    let (pool, services, owner) = setup("fin_eval_bill").await;
    services
        .finance
        .create_budget(&owner, budget(None, "USD", 10_000))
        .await
        .unwrap();
    let (bill, _) = services
        .recurring_bills
        .create(
            &owner,
            NewRecurringBill {
                room_id: "web".to_string(),
                sender_name: "alice".to_string(),
                name: "Rent".to_string(),
                category: "housing".to_string(),
                amount_minor: 9_000,
                currency: "USD".to_string(),
                currency_exponent: 2,
                frequency: "monthly".to_string(),
                due_day: Some(1),
                next_due_at: "2026-01-01 00:00:00".to_string(),
                total_installments: None,
                auto_pay: false,
                notes: None,
                remind_before_minutes: None,
                remind_room_id: None,
            },
        )
        .await
        .unwrap();

    services
        .recurring_bills
        .mark_paid(&owner, bill.id)
        .await
        .unwrap();

    assert_eq!(alert_jobs(&pool).await, 1);
}
