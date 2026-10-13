use super::{alerts, audit, cash, history, holdings, trades};
use crate::db::{migrate::run_migrations_unlocked, pool::seed_accounts, pool::test_pool};
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::alerts::AlertCondition;
use crate::domain::portfolio::models::{BuyOutcome, SellOutcome, WithdrawOutcome};
use crate::utils::time::now_at_storage_precision;
use chrono::Duration;

#[tokio::test]
async fn portfolio_repositories_cash_trades_holdings_and_audit_chain() {
    let pool = test_pool().await;
    run_migrations_unlocked(&pool).await;
    seed_accounts(&pool, &["port_repo_alice", "port_repo_bob"]).await;
    let alice = AccountId::from_seed("port_repo_alice");
    let bob = AccountId::from_seed("port_repo_bob");

    let base_time = now_at_storage_precision();
    let t0 = base_time - Duration::hours(4);
    let t1 = base_time - Duration::hours(3);
    let t2 = base_time - Duration::hours(2);
    let t3 = base_time - Duration::hours(1);

    // 1. Cash balance starts at zero; withdrawing from empty fails with InsufficientFunds
    let zero = cash::get_cash_or_zero(&pool, &alice, "USD", 2)
        .await
        .expect("get_cash_or_zero");
    assert_eq!(zero.balance_minor, 0);

    let mut tx = pool.begin().await.unwrap();
    let insufficient = cash::withdraw_cash_at(&mut tx, &alice, "USD", 1000, Some(t0))
        .await
        .expect("withdraw empty");
    assert!(matches!(
        insufficient,
        WithdrawOutcome::InsufficientFunds { available_minor: 0 }
    ));

    // Deposit $10,000.00 (1_000_000 minor units) at t0
    let deposited = cash::deposit_cash_at(&mut tx, &alice, "USD", 2, 1_000_000, Some(t0))
        .await
        .expect("deposit");
    assert_eq!(deposited.balance_minor, 1_000_000);
    tx.commit().await.unwrap();

    // 2. Trades: buy 10 AAPL (10 * 10^8 = 1_000_000_000 units) @ $150.00 (15_000 minor) = $1,500.00 (150_000 minor) at t1
    let mut tx = pool.begin().await.unwrap();
    let buy_res = trades::buy_at(
        &mut tx,
        &alice,
        "AAPL",
        1_000_000_000,
        15_000,
        "USD",
        2,
        Some("Initial lot"),
        Some(t1),
    )
    .await
    .expect("buy_at");
    let holding_id = match buy_res {
        BuyOutcome::Ok { holding, cash } => {
            assert_eq!(holding.symbol, "AAPL");
            assert_eq!(holding.quantity_minor, 1_000_000_000);
            assert_eq!(holding.avg_cost_minor, 15_000);
            assert_eq!(cash.balance_minor, 850_000);
            holding.id
        }
        other => panic!("unexpected buy outcome: {other:?}"),
    };

    // Currency mismatch on existing holding
    let mismatch = trades::buy_at(
        &mut tx,
        &alice,
        "AAPL",
        100_000_000,
        15_000,
        "EUR",
        2,
        None,
        Some(t1),
    )
    .await
    .expect("buy mismatch");
    assert!(matches!(
        mismatch,
        BuyOutcome::CurrencyMismatch {
            ref existing_currency
        } if existing_currency == "USD"
    ));

    // Record manual holding action (e.g. +10 AAPL @ $170.00 -> weighted avg $160.00) at t2
    let recorded = trades::record_holding_action(
        &mut tx,
        &alice,
        "AAPL",
        1_000_000_000,
        17_000,
        "USD",
        2,
        Some("Second lot"),
        "record",
        Some(t2),
    )
    .await
    .expect("record action")
    .expect("currency matches");
    assert_eq!(recorded.quantity_minor, 2_000_000_000);
    assert_eq!(recorded.avg_cost_minor, 16_000);

    // Sell 5 AAPL @ $200.00 -> reduced to 15 AAPL, proceeds $1,000.00 (100_000 minor) -> cash = 950_000 at t3
    // Realized P/L = 5 * ($200 - $160) = $200.00 (20_000 minor)
    let sell_res = trades::sell_at(&mut tx, &alice, "AAPL", 500_000_000, 20_000, Some(t3))
        .await
        .expect("sell_at");
    match sell_res {
        SellOutcome::Reduced {
            holding,
            realized_pl_minor,
            cash,
        } => {
            assert_eq!(holding.quantity_minor, 1_500_000_000);
            assert_eq!(realized_pl_minor, 20_000);
            assert_eq!(cash.balance_minor, 950_000);
        }
        other => panic!("unexpected sell outcome: {other:?}"),
    }

    // Holdings metadata & price updates
    holdings::set_price(&mut tx, &alice, "AAPL", 21_000)
        .await
        .expect("set_price")
        .expect("holding exists");
    holdings::set_note(&mut tx, &alice, "AAPL", Some("Core position"))
        .await
        .expect("set_note")
        .expect("holding exists");
    let renamed = holdings::update_symbol(&mut tx, &alice, holding_id, "AAPL.US")
        .await
        .expect("update_symbol")
        .expect("rename ok");
    assert_eq!(renamed.symbol, "AAPL.US");
    tx.commit().await.unwrap();

    // 3. Holdings queries & summary
    let owners = holdings::list_holding_owners(&pool)
        .await
        .expect("list_holding_owners");
    assert!(owners.contains(&alice));
    let summaries = holdings::summary(&pool, &alice).await.expect("summary");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].currency, "USD");

    // 4. Daily snapshots
    history::capture_current_snapshot_for_owner(&pool, &alice)
        .await
        .expect("capture snapshot");
    let snaps = history::get_snapshots(&pool, &alice, "USD", "2020-01-01")
        .await
        .expect("get_snapshots");
    assert_eq!(snaps.len(), 1);

    // 5. Audit chain verification & ledger absorption on account merge
    let first = audit::first_audit_entry_for_currency(&pool, &alice, "USD")
        .await
        .expect("first_audit_entry")
        .expect("has entry");
    assert_eq!(first.1, 1_000_000);

    let chain_ok = audit::verify_audit_chain(&pool, &alice)
        .await
        .expect("verify alice chain");
    assert!(chain_ok.is_valid);
    assert_eq!(chain_ok.total_verified, 4); // deposit, buy, record, sell

    // Bob records an entry, then absorbs Alice's ledger
    audit::record_audit_entry(
        &pool,
        &bob,
        "deposit",
        None,
        None,
        None,
        50_000,
        "USD",
        2,
        None,
        None,
        Some(base_time),
    )
    .await
    .expect("bob audit entry");

    let mut tx = pool.begin().await.unwrap();
    let absorbed = audit::absorb_ledger(&mut tx, &alice, &bob)
        .await
        .expect("absorb_ledger");
    assert_eq!(absorbed, 4);
    tx.commit().await.unwrap();

    let bob_chain = audit::verify_audit_chain(&pool, &bob)
        .await
        .expect("verify bob merged chain");
    assert!(bob_chain.is_valid);
    assert_eq!(bob_chain.total_verified, 6); // 1 bob + 4 alice + 1 merge marker
}

#[tokio::test]
async fn portfolio_repositories_alerts_lifecycle() {
    let pool = test_pool().await;
    run_migrations_unlocked(&pool).await;
    seed_accounts(&pool, &["port_alert_alice", "port_alert_bob"]).await;
    let alice = AccountId::from_seed("port_alert_alice");
    let bob = AccountId::from_seed("port_alert_bob");

    let alert = alerts::create(
        &pool,
        &alice,
        "btc",
        10_000_000,
        "usd",
        2,
        AlertCondition::Above,
    )
    .await
    .expect("create alert");
    assert_eq!(alert.symbol, "BTC");
    assert_eq!(alert.currency, "USD");
    assert!(!alert.is_triggered);

    let active_syms = alerts::list_active_symbols(&pool)
        .await
        .expect("active symbols");
    assert_eq!(active_syms, vec!["BTC".to_string()]);

    let active_for_btc = alerts::list_active_for_symbol(&pool, "BTC")
        .await
        .expect("active for symbol");
    assert_eq!(active_for_btc.len(), 1);

    assert!(alerts::mark_triggered(&pool, alert.id)
        .await
        .expect("mark_triggered"));
    assert!(alerts::list_active_for_symbol(&pool, "BTC")
        .await
        .expect("none active after trigger")
        .is_empty());

    let owner_list = alerts::list_for_owner(&pool, &alice)
        .await
        .expect("list_for_owner");
    assert_eq!(owner_list.len(), 1);
    assert!(owner_list[0].is_triggered);

    // Owner isolation on delete
    assert!(!alerts::delete(&pool, alert.id, &bob)
        .await
        .expect("bob delete"));
    assert!(alerts::delete(&pool, alert.id, &alice)
        .await
        .expect("alice delete"));
    assert!(alerts::find_by_id(&pool, alert.id, &alice)
        .await
        .expect("find after delete")
        .is_none());
}
