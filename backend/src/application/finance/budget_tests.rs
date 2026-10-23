use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::{BudgetScope, BudgetState, Month, NewBudget};
use crate::domain::finance::{Direction, NewTransaction};
use crate::infrastructure::postgres::repositories::{self, finance as repo};
use crate::utils::time::now_at_storage_precision;

pub(super) async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let owner = AccountId::from_seed(seed);
    crate::db::pool::seed_account(&pool, &owner).await;
    let services = AppServices::new(&pool);
    (pool, services, owner)
}

pub(super) fn budget(category: Option<&str>, currency: &str, limit_minor: i64) -> NewBudget {
    NewBudget::new(category, currency, limit_minor, true).unwrap()
}

async fn types(pool: &sqlx::PgPool, owner: &AccountId) -> Vec<String> {
    repositories::outbox::event_types_for(pool, owner)
        .await
        .unwrap()
}

async fn spend(
    services: &AppServices,
    owner: &AccountId,
    direction: Direction,
    minor: i64,
    currency: &str,
    category: Option<&str>,
) {
    let input = NewTransaction {
        direction,
        amount_minor: minor,
        currency,
        currency_exponent: 2,
        category,
        note: None,
        receipt_storage_key: None,
        room_id: "web",
        sender_name: "tester",
        occurred_at: now_at_storage_precision(),
        source: None,
        create_missing_category: false,
    };
    services
        .finance
        .create_transaction(owner, input)
        .await
        .unwrap();
}

/// Given a new budget, When it is created, Then BudgetCreated is in the outbox.
#[tokio::test]
async fn create_budget_publishes_budget_created() {
    let (pool, services, owner) = setup("fin_budget_create").await;

    let created = services
        .finance
        .create_budget(&owner, budget(None, "EUR", 100_000))
        .await
        .unwrap();

    assert_eq!(created.limit_minor, 100_000);
    assert_eq!(types(&pool, &owner).await, vec!["BudgetCreated"]);
}

/// Given an overall EUR budget, When another is created, Then it conflicts.
#[tokio::test]
async fn create_budget_twice_is_conflict() {
    let (_pool, services, owner) = setup("fin_budget_twice").await;
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 100_000))
        .await
        .unwrap();

    let err = services
        .finance
        .create_budget(&owner, budget(None, "EUR", 5_000))
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
}

/// Given no such category, When a budget names it, Then validation fails and no category appears.
#[tokio::test]
async fn create_budget_for_unknown_category_is_validation_and_creates_no_category() {
    let (pool, services, owner) = setup("fin_budget_unknown_cat").await;

    let err = services
        .finance
        .create_budget(&owner, budget(Some("ghost"), "EUR", 1_000))
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::Validation(_)), "{err:?}");
    assert!(repo::list_categories(&pool, &owner)
        .await
        .unwrap()
        .is_empty());
}

/// Given an existing budget, When set again, Then its limit changes and BudgetUpdated follows.
#[tokio::test]
async fn set_budget_updates_existing_scope() {
    let (pool, services, owner) = setup("fin_budget_set").await;

    let (first, created) = services
        .finance
        .set_budget(&owner, budget(None, "EUR", 100_000))
        .await
        .unwrap();
    let (second, created_again) = services
        .finance
        .set_budget(&owner, budget(None, "EUR", 250_000))
        .await
        .unwrap();

    assert!(created);
    assert!(!created_again);
    assert_eq!(first.id, second.id);
    assert_eq!(second.limit_minor, 250_000);
    assert_eq!(
        types(&pool, &owner).await,
        vec!["BudgetCreated", "BudgetUpdated"]
    );
}

/// Given overall budgets in two currencies, When resolved without a currency, Then it is ambiguous.
#[tokio::test]
async fn resolve_budget_ambiguous_without_currency() {
    let (_pool, services, owner) = setup("fin_budget_ambiguous").await;
    for cur in ["EUR", "USD"] {
        services
            .finance
            .create_budget(&owner, budget(None, cur, 1_000))
            .await
            .unwrap();
    }

    let err = services
        .finance
        .resolve_budget(&owner, None, None)
        .await
        .unwrap_err();
    let ok = services
        .finance
        .resolve_budget(&owner, None, Some("usd"))
        .await
        .unwrap();

    assert!(matches!(err, AppError::Validation(_)), "{err:?}");
    assert_eq!(ok.currency, "USD");
}

/// Given mixed expenses, income and currencies, When status is read, Then only matching
/// expenses count and the overall budget comes first.
#[tokio::test]
async fn budget_status_sums_category_and_overall() {
    let (_pool, services, owner) = setup("fin_budget_status").await;
    services
        .finance
        .create_category(&owner, "food")
        .await
        .unwrap();
    services
        .finance
        .create_budget(&owner, budget(Some("food"), "EUR", 30_000))
        .await
        .unwrap();
    services
        .finance
        .create_budget(&owner, budget(None, "EUR", 100_000))
        .await
        .unwrap();
    spend(
        &services,
        &owner,
        Direction::Expense,
        12_000,
        "EUR",
        Some("food"),
    )
    .await;
    spend(&services, &owner, Direction::Expense, 5_000, "EUR", None).await;
    spend(&services, &owner, Direction::Income, 99_900, "EUR", None).await;
    spend(&services, &owner, Direction::Expense, 4_000, "USD", None).await;

    let statuses = services.finance.budget_status(&owner, None).await.unwrap();

    assert_eq!(statuses.len(), 2);
    assert!(matches!(statuses[0].budget.scope, BudgetScope::Overall));
    assert_eq!(statuses[0].spent_minor, 17_000);
    assert_eq!(statuses[1].spent_minor, 12_000);
    assert_eq!(statuses[1].percent_used, 40);
    assert_eq!(statuses[1].state, BudgetState::Ok);
}

/// Given another owner's budget, When its status is requested, Then it is None.
#[tokio::test]
async fn budget_status_for_other_owner_budget_is_none() {
    let (pool, services, owner) = setup("fin_budget_status_owner_a").await;
    let other = AccountId::from_seed("fin_budget_status_owner_b");
    crate::db::pool::seed_account(&pool, &other).await;
    let theirs = services
        .finance
        .create_budget(&other, budget(None, "EUR", 1_000))
        .await
        .unwrap();
    let month = Month::containing(chrono::Utc::now(), chrono_tz::UTC);

    let got = services
        .finance
        .status_for_budget(&owner, theirs.id, month)
        .await
        .unwrap();

    assert!(got.is_none());
}
