//! Portfolio events are written to the outbox in the transaction of the state
//! change they describe.

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::alerts::AlertCondition;
use crate::domain::portfolio::models::{BuyOutcome, SellOutcome, WithdrawOutcome};
use crate::domain::portfolio::TradeRequest;
use crate::infrastructure::postgres::repositories::outbox;

async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let owner = AccountId::from_seed(seed);
    crate::db::pool::seed_account(&pool, &owner).await;
    let services = AppServices::new(&pool);
    (pool, services, owner)
}

async fn types(pool: &sqlx::PgPool, owner: &AccountId) -> Vec<String> {
    outbox::event_types_for(pool, owner).await.unwrap()
}

fn trade<'a>(symbol: &'a str, note: Option<&'a str>) -> TradeRequest<'a> {
    TradeRequest {
        symbol,
        quantity: "10",
        price: "150.00",
        currency: "USD",
        note,
        executed_at: None,
    }
}

/// Funds the owner and returns the outbox length after funding.
async fn fund(pool: &sqlx::PgPool, services: &AppServices, owner: &AccountId) -> usize {
    services
        .portfolio
        .deposit_cash(owner, "USD", 2, 1_000_000, None)
        .await
        .unwrap();
    types(pool, owner).await.len()
}

async fn holding_count(pool: &sqlx::PgPool, owner: &AccountId) -> usize {
    crate::infrastructure::postgres::repositories::portfolio::holdings::list(pool, owner)
        .await
        .unwrap()
        .len()
}

/// Given funded cash, When a buy creates a holding, Then the holding and cash events are
/// in the outbox, in the order they are emitted.
#[tokio::test]
async fn buy_writes_holding_and_cash_events_to_the_outbox() {
    let (pool, services, owner) = setup("pf_outbox_buy").await;
    let before = fund(&pool, &services, &owner).await;

    let outcome = services
        .portfolio
        .buy(&owner, trade("AAPL", None))
        .await
        .unwrap();

    assert!(matches!(outcome, BuyOutcome::Ok { .. }));
    assert_eq!(
        types(&pool, &owner).await[before..].to_vec(),
        vec!["PortfolioHoldingCreated", "PortfolioCashUpdated"]
    );
}

/// Given a held symbol, When it is bought again, Then the event is an update.
#[tokio::test]
async fn second_buy_writes_a_holding_updated_event() {
    let (pool, services, owner) = setup("pf_outbox_buy_twice").await;
    services
        .portfolio
        .deposit_cash(&owner, "USD", 2, 1_000_000, None)
        .await
        .unwrap();
    services
        .portfolio
        .buy(&owner, trade("AAPL", None))
        .await
        .unwrap();
    let before = types(&pool, &owner).await.len();

    services
        .portfolio
        .buy(&owner, trade("AAPL", None))
        .await
        .unwrap();

    assert_eq!(
        types(&pool, &owner).await[before..].to_vec(),
        vec!["PortfolioHoldingUpdated", "PortfolioCashUpdated"]
    );
}

/// Given too little cash, When a buy is attempted, Then it reports the existing outcome
/// and writes no event and no holding.
#[tokio::test]
async fn buy_without_enough_cash_writes_no_event() {
    let (pool, services, owner) = setup("pf_outbox_buy_poor").await;
    let before = types(&pool, &owner).await.len();

    let outcome = services
        .portfolio
        .buy(&owner, trade("AAPL", None))
        .await
        .unwrap();

    assert!(matches!(outcome, BuyOutcome::InsufficientCash { .. }));
    assert_eq!(types(&pool, &owner).await.len(), before);
    assert_eq!(holding_count(&pool, &owner).await, 0);
}

/// Given funded cash, When the note holds a NUL byte (rejected by PostgreSQL mid-transaction,
/// after the cash was debited), Then no holding, no cash change and no event remain.
#[tokio::test]
async fn buy_failing_mid_transaction_leaves_no_state_and_no_event() {
    let (pool, services, owner) = setup("pf_outbox_buy_nul").await;
    let before = fund(&pool, &services, &owner).await;

    let res = services
        .portfolio
        .buy(&owner, trade("AAPL", Some("bad\0note")))
        .await;

    assert!(res.is_err());
    assert_eq!(holding_count(&pool, &owner).await, 0);
    assert_eq!(types(&pool, &owner).await.len(), before);
    let cash = services.portfolio.get_cash(&owner, "USD", 2).await.unwrap();
    assert_eq!(cash.balance_minor, 1_000_000);
}

/// Given a holding, When it is partly sold and then closed, Then reduce and close events follow.
#[tokio::test]
async fn sell_writes_holding_and_cash_events() {
    let (pool, services, owner) = setup("pf_outbox_sell").await;
    services
        .portfolio
        .deposit_cash(&owner, "USD", 2, 1_000_000, None)
        .await
        .unwrap();
    services
        .portfolio
        .buy(&owner, trade("AAPL", None))
        .await
        .unwrap();
    let before = types(&pool, &owner).await.len();

    let reduced = services
        .portfolio
        .sell(&owner, "AAPL", "4", "160.00", None)
        .await
        .unwrap();
    let closed = services
        .portfolio
        .sell(&owner, "AAPL", "6", "160.00", None)
        .await
        .unwrap();
    let exceeds = services
        .portfolio
        .sell(&owner, "AAPL", "1", "160.00", None)
        .await
        .unwrap();

    assert!(matches!(reduced, SellOutcome::Reduced { .. }));
    assert!(matches!(closed, SellOutcome::Closed { .. }));
    assert!(matches!(exceeds, SellOutcome::NotFound));
    assert_eq!(
        types(&pool, &owner).await[before..].to_vec(),
        vec![
            "PortfolioHoldingUpdated",
            "PortfolioCashUpdated",
            "PortfolioHoldingDeleted",
            "PortfolioCashUpdated"
        ]
    );
}

/// Given no holding, When record runs (and again), Then created then updated events are stored;
/// a NUL note writes nothing.
#[tokio::test]
async fn record_writes_holding_events_and_is_atomic() {
    let (pool, services, owner) = setup("pf_outbox_record").await;

    services
        .portfolio
        .record(&owner, trade("MSFT", None))
        .await
        .unwrap()
        .unwrap();
    services
        .portfolio
        .record(&owner, trade("MSFT", None))
        .await
        .unwrap()
        .unwrap();
    let res = services
        .portfolio
        .record(&owner, trade("GOOG", Some("bad\0note")))
        .await;

    assert!(res.is_err());
    assert_eq!(
        types(&pool, &owner).await,
        vec!["PortfolioHoldingCreated", "PortfolioHoldingUpdated"]
    );
    assert_eq!(holding_count(&pool, &owner).await, 1);
}

/// Given cash movements, When depositing and withdrawing, Then each writes PortfolioCashUpdated
/// (deposit also mirrors into finance); an over-withdrawal writes nothing.
#[tokio::test]
async fn deposit_and_withdraw_write_cash_events() {
    let (pool, services, owner) = setup("pf_outbox_cash").await;

    services
        .portfolio
        .deposit_cash(&owner, "USD", 2, 5_000, None)
        .await
        .unwrap();
    let after_deposit = types(&pool, &owner).await;
    assert!(after_deposit.contains(&"PortfolioCashUpdated".to_string()));
    let cash_events = |v: &[String]| v.iter().filter(|t| *t == "PortfolioCashUpdated").count();
    assert_eq!(cash_events(&after_deposit), 1);

    let ok = services
        .portfolio
        .withdraw_cash(&owner, "USD", 2, 1_000, None)
        .await
        .unwrap();
    assert!(matches!(ok, WithdrawOutcome::Ok(_)));
    assert_eq!(cash_events(&types(&pool, &owner).await), 2);

    let short = services
        .portfolio
        .withdraw_cash(&owner, "USD", 2, 999_999, None)
        .await
        .unwrap();
    assert!(matches!(short, WithdrawOutcome::InsufficientFunds { .. }));
    assert_eq!(cash_events(&types(&pool, &owner).await), 2);
}

/// Given a NUL currency, When depositing, Then no balance and no event exist.
#[tokio::test]
async fn deposit_failing_mid_transaction_leaves_nothing() {
    let (pool, services, owner) = setup("pf_outbox_cash_nul").await;

    let res = services
        .portfolio
        .deposit_cash(&owner, "US\0D", 2, 5_000, None)
        .await;

    assert!(res.is_err());
    assert!(types(&pool, &owner).await.is_empty());
    assert!(services
        .portfolio
        .list_cash(&owner)
        .await
        .unwrap()
        .is_empty());
}

/// Given a holding, When price, note and symbol change and it is deleted, Then each writes its event;
/// a NUL note writes nothing and leaves the note untouched.
#[tokio::test]
async fn holding_edits_and_delete_write_events() {
    let (pool, services, owner) = setup("pf_outbox_holding_edit").await;
    let holding = services
        .portfolio
        .record(&owner, trade("AAPL", None))
        .await
        .unwrap()
        .unwrap();
    let before = types(&pool, &owner).await.len();

    services
        .portfolio
        .set_price(&owner, "AAPL", 16_000)
        .await
        .unwrap()
        .unwrap();
    services
        .portfolio
        .set_note(&owner, "AAPL", Some("core"))
        .await
        .unwrap()
        .unwrap();
    services
        .portfolio
        .update_symbol(&owner, holding.id, "AAPL2")
        .await
        .unwrap()
        .unwrap();
    assert!(services
        .portfolio
        .set_note(&owner, "AAPL2", Some("bad\0note"))
        .await
        .is_err());
    assert!(services
        .portfolio
        .delete_holding(&owner, "AAPL2")
        .await
        .unwrap());
    assert!(!services
        .portfolio
        .delete_holding(&owner, "AAPL2")
        .await
        .unwrap());

    assert_eq!(
        types(&pool, &owner).await[before..].to_vec(),
        vec![
            "PortfolioHoldingUpdated",
            "PortfolioHoldingUpdated",
            "PortfolioHoldingUpdated",
            "PortfolioHoldingDeleted"
        ]
    );
}

/// Given alerts, When one is created, triggered and another deleted, Then each writes its event;
/// a NUL symbol writes nothing.
#[tokio::test]
async fn alert_lifecycle_writes_events() {
    let (pool, services, owner) = setup("pf_outbox_alerts").await;

    let alert = services
        .portfolio
        .create_alert(&owner, "AAPL", 20_000, "USD", 2, AlertCondition::Above)
        .await
        .unwrap();
    assert!(services
        .portfolio
        .mark_alert_triggered(&alert)
        .await
        .unwrap());
    let other = services
        .portfolio
        .create_alert(&owner, "MSFT", 20_000, "USD", 2, AlertCondition::Below)
        .await
        .unwrap();
    assert!(services
        .portfolio
        .delete_alert(&owner, other.id)
        .await
        .unwrap());
    assert!(!services
        .portfolio
        .delete_alert(&owner, other.id)
        .await
        .unwrap());
    assert!(services
        .portfolio
        .create_alert(&owner, "BA\0D", 1, "USD", 2, AlertCondition::Above)
        .await
        .is_err());

    assert_eq!(
        types(&pool, &owner).await,
        vec![
            "PortfolioAlertCreated",
            "PortfolioAlertTriggered",
            "PortfolioAlertCreated",
            "PortfolioAlertDeleted"
        ]
    );
}
