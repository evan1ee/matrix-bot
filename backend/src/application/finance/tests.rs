use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::finance::{Direction, NewTransaction};
use crate::infrastructure::postgres::repositories::{self, finance as repo, outbox};
use crate::utils::time::now_at_storage_precision;

async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let owner = AccountId::from_seed(seed);
    crate::db::pool::seed_account(&pool, &owner).await;
    let services = AppServices::new(&pool);
    (pool, services, owner)
}

fn expense(note: &str) -> NewTransaction<'_> {
    NewTransaction {
        direction: Direction::Expense,
        amount_minor: 1550,
        currency: "EUR",
        currency_exponent: 2,
        category: None,
        note: Some(note),
        receipt_storage_key: None,
        room_id: "web",
        sender_name: "tester",
        occurred_at: now_at_storage_precision(),
        source: None,
        create_missing_category: false,
    }
}

async fn types(pool: &sqlx::PgPool, owner: &AccountId) -> Vec<String> {
    repositories::outbox::event_types_for(pool, owner)
        .await
        .unwrap()
}

/// Given an owner, When a transaction is created, Then TransactionCreated is in the outbox.
#[tokio::test]
async fn create_transaction_writes_to_the_outbox() {
    let (pool, services, owner) = setup("fin_outbox_create_tx").await;

    services
        .finance
        .create_transaction(&owner, expense("lunch"))
        .await
        .unwrap();

    assert_eq!(types(&pool, &owner).await, vec!["TransactionCreated"]);
}

/// Given a transaction, When it is updated and then deleted, Then both events follow the create.
#[tokio::test]
async fn update_and_delete_transaction_write_to_the_outbox() {
    let (pool, services, owner) = setup("fin_outbox_upd_del_tx").await;
    let tx = services
        .finance
        .create_transaction(&owner, expense("lunch"))
        .await
        .unwrap();

    services
        .finance
        .update_transaction(
            &owner,
            tx.id,
            crate::domain::finance::TransactionPatch {
                amount_minor: Some(900),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    services
        .finance
        .delete_transaction(&owner, tx.id)
        .await
        .unwrap();

    assert_eq!(
        types(&pool, &owner).await,
        vec![
            "TransactionCreated",
            "TransactionUpdated",
            "TransactionDeleted"
        ]
    );
}

/// Given an owner, When a category is created, Then CategoryCreated is in the outbox.
#[tokio::test]
async fn create_category_writes_to_the_outbox() {
    let (pool, services, owner) = setup("fin_outbox_create_cat").await;

    services
        .finance
        .create_category(&owner, "food")
        .await
        .unwrap();

    assert_eq!(types(&pool, &owner).await, vec!["CategoryCreated"]);
}

/// Given a category used by a transaction, When it is deleted, Then the entry is
/// uncategorised and CategoryDeleted is in the outbox; deleting a missing one publishes nothing.
#[tokio::test]
async fn delete_category_writes_to_the_outbox() {
    let (pool, services, owner) = setup("fin_outbox_delete_cat").await;
    let cat = services
        .finance
        .create_category(&owner, "food")
        .await
        .unwrap();
    let mut input = expense("lunch");
    input.category = Some("food");
    services
        .finance
        .create_transaction(&owner, input)
        .await
        .unwrap();

    assert_eq!(
        services
            .finance
            .delete_category(&owner, cat.id)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        services
            .finance
            .delete_category(&owner, cat.id)
            .await
            .unwrap(),
        None
    );

    let events = types(&pool, &owner).await;
    assert_eq!(
        events.iter().filter(|t| *t == "CategoryDeleted").count(),
        1,
        "only the successful delete publishes: {events:?}"
    );
}

/// Given the category delete's multi-statement transaction, When the publish
/// fails after the writes, Then the category and its transaction link survive
/// and no event exists.
#[tokio::test]
async fn a_failed_publish_rolls_back_the_category_delete() {
    let (pool, services, owner) = setup("fin_outbox_delete_cat_atomic").await;
    let cat = services
        .finance
        .create_category(&owner, "food")
        .await
        .unwrap();
    let mut input = expense("lunch");
    input.category = Some("food");
    let created = services
        .finance
        .create_transaction(&owner, input)
        .await
        .unwrap();
    outbox::fail_every_publish(&pool).await;

    assert!(services
        .finance
        .delete_category(&owner, cat.id)
        .await
        .is_err());

    assert_eq!(
        services
            .finance
            .list_categories(&owner)
            .await
            .unwrap()
            .len(),
        1
    );
    let still = repo::find_owned(&pool, &owner, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.category_id, Some(cat.id));
    assert!(!types(&pool, &owner)
        .await
        .contains(&"CategoryDeleted".to_string()));
}

#[tokio::test]
async fn record_expense_with_receipt_and_link_receipt() {
    use crate::domain::storage::{StorageCategory, StorageKey};

    let (pool, services, owner) = setup("fin_record_expense_receipt").await;
    let key1 = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");

    let tx = services
        .finance
        .record_expense(
            &owner,
            2500,
            "USD",
            2,
            None,
            Some("dinner"),
            Some(key1.clone()),
            "web",
            "tester",
        )
        .await
        .unwrap();

    assert_eq!(tx.receipt_storage_key, Some(key1));
    assert!(types(&pool, &owner)
        .await
        .contains(&"TransactionCreated".to_string()));

    let key2 = StorageKey::generate(&owner, StorageCategory::Receipts, "png");
    let updated = services
        .finance
        .link_receipt(&owner, tx.id, key2.clone())
        .await
        .unwrap();

    assert_eq!(updated.receipt_storage_key, Some(key2));
    assert!(types(&pool, &owner)
        .await
        .contains(&"TransactionUpdated".to_string()));

    // Other owner cannot link receipt to another owner's transaction
    let other_owner = AccountId::from_seed("other_fin_owner");
    let err = services
        .finance
        .link_receipt(
            &other_owner,
            tx.id,
            StorageKey::generate(&other_owner, StorageCategory::Receipts, "jpg"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn create_rejects_foreign_or_non_receipt_key() {
    use crate::domain::storage::{StorageCategory, StorageKey};

    let (pool, services, owner) = setup("fin_receipt_key_rules").await;
    let stranger = AccountId::from_seed("fin_receipt_key_stranger");
    let bad_keys = [
        StorageKey::generate(&stranger, StorageCategory::Receipts, "jpg"),
        StorageKey::generate(&owner, StorageCategory::Audio, "ogg"),
    ];

    for key in bad_keys {
        let err = services
            .finance
            .record_expense(
                &owner,
                100,
                "USD",
                2,
                None,
                None,
                Some(key),
                "web",
                "tester",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{err:?}");
    }
    let rows = repo::list(&pool, &owner, &Default::default())
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn link_receipt_rejects_foreign_key() {
    use crate::domain::storage::{StorageCategory, StorageKey};

    let (_pool, services, owner) = setup("fin_link_foreign_key").await;
    let stranger = AccountId::from_seed("fin_link_foreign_key_other");
    let tx = services
        .finance
        .record_expense(&owner, 100, "USD", 2, None, None, None, "web", "tester")
        .await
        .unwrap();

    let err = services
        .finance
        .link_receipt(
            &owner,
            tx.id,
            StorageKey::generate(&stranger, StorageCategory::Receipts, "jpg"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)), "{err:?}");
}

#[tokio::test]
async fn detach_receipt_clears_key_and_publishes_update() {
    use crate::domain::storage::{StorageCategory, StorageKey};

    let (pool, services, owner) = setup("fin_detach_receipt").await;
    let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");
    let tx = services
        .finance
        .record_expense(&owner, 100, "USD", 2, None, None, Some(key), "web", "t")
        .await
        .unwrap();

    let updated = services
        .finance
        .detach_receipt(&owner, tx.id)
        .await
        .unwrap();

    assert_eq!(updated.receipt_storage_key, None);
    assert!(types(&pool, &owner)
        .await
        .contains(&"TransactionUpdated".to_string()));
}

#[tokio::test]
async fn detach_receipt_without_receipt_or_other_owner_is_not_found() {
    use crate::domain::storage::{StorageCategory, StorageKey};

    let (_pool, services, owner) = setup("fin_detach_missing").await;
    let stranger = AccountId::from_seed("fin_detach_missing_other");
    let bare = services
        .finance
        .record_expense(&owner, 100, "USD", 2, None, None, None, "web", "t")
        .await
        .unwrap();
    let with = services
        .finance
        .record_expense(
            &owner,
            100,
            "USD",
            2,
            None,
            None,
            Some(StorageKey::generate(
                &owner,
                StorageCategory::Receipts,
                "png",
            )),
            "web",
            "t",
        )
        .await
        .unwrap();

    let err = services
        .finance
        .detach_receipt(&owner, bare.id)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
    let err = services
        .finance
        .detach_receipt(&stranger, with.id)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
    let still = services
        .finance
        .get_transaction(&owner, with.id)
        .await
        .unwrap()
        .unwrap();
    assert!(still.receipt_storage_key.is_some());
}
