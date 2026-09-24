//! What it means to record, change, or remove a financial entry, independent
//! of how the request arrived.
//!
//! Three transports write finance rows — the REST API, a Matrix command, and
//! an AI tool call — and each carried its own partial copy of the rules. They
//! had drifted: the AI path validated neither the note length nor the
//! category name, the Matrix path enforced a note limit but never announced
//! what it had done (every expense or income logged over Matrix was invisible
//! to an open browser until the page was reloaded), and the category-name
//! limit was a bare literal `64` typed out again in the Matrix path rather
//! than shared with the REST constant.
//!
//! So the decisions live here and the transports parse and render. This
//! module returns data and typed errors; it renders nothing.

use crate::db::DbPool;
use chrono::{DateTime, Utc};

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::models::{Category, Transaction};
use crate::modules::finance::repository::FinanceRepository;

/// Maximum allowed length for a transaction's note.
pub const MAX_NOTE_LEN: usize = 500;
/// Maximum allowed length for a category name.
pub const MAX_CATEGORY_LEN: usize = 64;

/// A transaction to create. The caller has already parsed it out of whatever
/// it arrived in and converted the amount to minor units.
pub struct NewTransaction<'a> {
    pub direction: &'a str,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub currency_exponent: i64,
    /// Resolved to a category id, creating it if it does not exist.
    pub category: Option<&'a str>,
    pub note: Option<&'a str>,
    pub room_id: &'a str,
    pub sender_name: &'a str,
    pub occurred_at: DateTime<Utc>,
    /// Set when a recurring bill produced this row.
    pub source: Option<(&'a str, i64)>,
    /// Whether an unknown category name may be created on the spot.
    ///
    /// The transports differ on purpose. A Matrix command refuses, because a
    /// typo would otherwise fragment one category into two silently. The AI
    /// tool creates, because it is filling in a category the user described in
    /// prose rather than one they picked. Do not unify these without deciding
    /// which behaviour is wanted.
    pub create_missing_category: bool,
}

/// Fields to change on an existing transaction.
///
/// `None` leaves a field alone. `note` and `category` are nullable columns,
/// so they are `Option<Option<T>>`: absent leaves it alone, `Some(None)`
/// clears it, `Some(Some(x))` sets it. The REST transport currently replaces
/// every field on every update (a PUT, not a PATCH), so it always sends
/// `Some(..)`, but the overlay-onto-the-existing-row shape is what lets a
/// partial caller change later without every other field having to restate
/// itself.
#[derive(Default)]
pub struct TransactionPatch<'a> {
    pub direction: Option<&'a str>,
    pub amount_minor: Option<i64>,
    pub currency: Option<&'a str>,
    pub currency_exponent: Option<i64>,
    pub category: Option<Option<&'a str>>,
    pub note: Option<Option<&'a str>>,
    pub occurred_at: Option<DateTime<Utc>>,
}

/// Why a write was refused.
///
/// Variants rather than a rendered string: the Matrix path answers in HTML
/// with `<code>` markup and the REST path answers in JSON, so whoever renders
/// has to be the one that knows which it is.
///
/// `TransactionNotFound` and `CategoryNotFound` are kept separate (rather than
/// one `NotFound`) because the REST responses they replace read differently —
/// "Entry not found or access denied" vs "Category not found or access
/// denied" — and collapsing them would be a wording change this refactor is
/// not meant to make.
#[derive(Debug)]
pub enum FinanceError {
    NoteTooLong {
        max: usize,
    },
    CategoryNameInvalid {
        max: usize,
    },
    /// The owner already has a category with that name.
    CategoryExists,
    TransactionNotFound,
    CategoryNotFound,
    /// A named category that does not exist, on a path that will not create one.
    UnknownCategory {
        name: String,
    },
    Db(anyhow::Error),
}

impl std::fmt::Display for FinanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinanceError::NoteTooLong { max } => {
                write!(f, "Note exceeds {} character limit", max)
            }
            FinanceError::CategoryNameInvalid { max } => {
                write!(f, "Category name must be 1\u{2013}{} characters", max)
            }
            FinanceError::CategoryExists => {
                write!(f, "You already have a category with that name")
            }
            FinanceError::TransactionNotFound => write!(f, "Entry not found or access denied"),
            FinanceError::CategoryNotFound => write!(f, "Category not found or access denied"),
            FinanceError::UnknownCategory { name } => write!(f, "No category named {name}"),
            FinanceError::Db(_) => write!(f, "Internal error"),
        }
    }
}

fn validate_note(note: Option<&str>) -> Result<(), FinanceError> {
    if let Some(n) = note {
        if n.len() > MAX_NOTE_LEN {
            return Err(FinanceError::NoteTooLong { max: MAX_NOTE_LEN });
        }
    }
    Ok(())
}

/// Resolves a category name to an id. `None` in, `None` out — a transaction
/// is allowed to carry no category at all.
///
/// When `create_missing` is true, an unknown name is created on the spot
/// (the AI tool's behaviour). When false, an unknown name is refused with
/// `UnknownCategory` instead (the Matrix command's anti-typo rule) — see
/// `NewTransaction::create_missing_category` for why the transports differ.
async fn resolve_category(
    pool: &DbPool,
    owner_id: &str,
    name: Option<&str>,
    create_missing: bool,
) -> Result<Option<i64>, FinanceError> {
    let Some(name) = name else {
        return Ok(None);
    };
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_CATEGORY_LEN {
        return Err(FinanceError::CategoryNameInvalid {
            max: MAX_CATEGORY_LEN,
        });
    }

    if create_missing {
        let category = FinanceRepository::find_or_create_category(pool, owner_id, trimmed)
            .await
            .map_err(FinanceError::Db)?
            .ok_or_else(|| {
                FinanceError::Db(anyhow::anyhow!(
                    "category '{}' vanished between find and create",
                    trimmed
                ))
            })?;

        Ok(Some(category.id))
    } else {
        let category = FinanceRepository::find_category_by_name(pool, owner_id, trimmed)
            .await
            .map_err(FinanceError::Db)?
            .ok_or_else(|| FinanceError::UnknownCategory {
                name: trimmed.to_string(),
            })?;

        Ok(Some(category.id))
    }
}

/// Records a transaction: validates, resolves its category, writes, then
/// broadcasts.
pub async fn create_transaction(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    input: NewTransaction<'_>,
) -> Result<Transaction, FinanceError> {
    validate_note(input.note)?;
    let category_id = resolve_category(
        pool,
        owner_id,
        input.category,
        input.create_missing_category,
    )
    .await?;

    let (source_kind, source_id) = match input.source {
        Some((kind, id)) => (Some(kind), Some(id)),
        None => (None, None),
    };

    let tx = FinanceRepository::create_transaction(
        pool,
        input.room_id,
        owner_id,
        input.sender_name,
        input.direction,
        input.amount_minor,
        input.currency,
        input.currency_exponent,
        category_id,
        input.note,
        source_kind,
        source_id,
        input.occurred_at,
    )
    .await
    .map_err(FinanceError::Db)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::TransactionCreated(tx.clone()));
    }

    Ok(tx)
}

/// Applies a patch to an owned transaction, then broadcasts.
///
/// `FinanceRepository::update` takes every column rather than individual
/// setters, so the existing row is read, the patch is overlaid onto it, and
/// the merged values are what get written — mirroring what
/// `api/finance.rs::update_transaction` did before this existed.
pub async fn update_transaction(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
    patch: TransactionPatch<'_>,
) -> Result<Transaction, FinanceError> {
    let existing = FinanceRepository::find_owned(pool, id, owner_id)
        .await
        .map_err(FinanceError::Db)?
        .ok_or(FinanceError::TransactionNotFound)?;

    if let Some(Some(note)) = patch.note {
        validate_note(Some(note))?;
    }

    let category_id = match patch.category {
        None => existing.category_id,
        Some(None) => None,
        // The REST transport is the only caller and it always resolves the
        // category by id first (`api/finance.rs::owned_category`), so the row
        // is already known to exist — never create on this path.
        Some(Some(name)) => resolve_category(pool, owner_id, Some(name), false).await?,
    };

    let direction = patch.direction.unwrap_or(existing.direction.as_str());
    let amount_minor = patch.amount_minor.unwrap_or(existing.amount_minor);
    let currency = patch.currency.unwrap_or(existing.currency.as_str());
    let currency_exponent = patch
        .currency_exponent
        .unwrap_or(existing.currency_exponent);
    let note = match patch.note {
        None => existing.note.clone(),
        Some(None) => None,
        Some(Some(n)) => Some(n.to_string()),
    };
    let occurred_at = patch.occurred_at.unwrap_or(existing.occurred_at);

    let tx = FinanceRepository::update(
        pool,
        id,
        direction,
        amount_minor,
        currency,
        currency_exponent,
        category_id,
        note.as_deref(),
        occurred_at,
    )
    .await
    .map_err(FinanceError::Db)?
    .ok_or(FinanceError::TransactionNotFound)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::TransactionUpdated(tx.clone()));
    }

    Ok(tx)
}

/// Deletes an owned transaction, then broadcasts.
pub async fn delete_transaction(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
) -> Result<Transaction, FinanceError> {
    let tx = FinanceRepository::find_owned(pool, id, owner_id)
        .await
        .map_err(FinanceError::Db)?
        .ok_or(FinanceError::TransactionNotFound)?;

    FinanceRepository::delete(pool, id)
        .await
        .map_err(FinanceError::Db)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::TransactionDeleted {
            id,
            room_id: tx.room_id.clone(),
            owner_id: owner_id.to_string(),
        });
    }

    Ok(tx)
}

/// Creates a category, then broadcasts.
pub async fn create_category(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
) -> Result<Category, FinanceError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_CATEGORY_LEN {
        return Err(FinanceError::CategoryNameInvalid {
            max: MAX_CATEGORY_LEN,
        });
    }

    let category = FinanceRepository::create_category(pool, owner_id, trimmed)
        .await
        .map_err(FinanceError::Db)?
        .ok_or(FinanceError::CategoryExists)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::CategoryCreated(category.clone()));
    }

    Ok(category)
}

/// Deletes an owned category, un-categorising its transactions, then
/// broadcasts. Returns how many transactions were left uncategorised.
pub async fn delete_category(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
) -> Result<u64, FinanceError> {
    let uncategorised = FinanceRepository::delete_category(pool, owner_id, id)
        .await
        .map_err(FinanceError::Db)?
        .ok_or(FinanceError::CategoryNotFound)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::CategoryDeleted {
            id,
            owner_id: owner_id.to_string(),
        });
    }

    Ok(uncategorised)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    fn new_tx<'a>(note: Option<&'a str>, category: Option<&'a str>) -> NewTransaction<'a> {
        NewTransaction {
            direction: "expense",
            amount_minor: 1000,
            currency: "USD",
            currency_exponent: 2,
            category,
            note,
            room_id: "!room:1",
            sender_name: "alice",
            occurred_at: Utc::now(),
            source: None,
            create_missing_category: true,
        }
    }

    /// Every rejection `create_transaction` can produce, in one place — this
    /// replaces per-transport validation tests. Each transport keeps only a
    /// smoke test that a rejection surfaces in its own format.
    #[tokio::test]
    async fn test_create_rejection_parity_table() {
        let pool = setup().await;
        let over_limit_note = "x".repeat(MAX_NOTE_LEN + 1);
        let over_limit_category = "x".repeat(MAX_CATEGORY_LEN + 1);

        struct Case<'a> {
            name: &'a str,
            input: NewTransaction<'a>,
            expect: fn(&FinanceError) -> bool,
        }

        let cases = vec![
            Case {
                name: "note one character over the limit",
                input: new_tx(Some(&over_limit_note), None),
                expect: |e| matches!(e, FinanceError::NoteTooLong { .. }),
            },
            Case {
                name: "empty category name",
                input: new_tx(None, Some("   ")),
                expect: |e| matches!(e, FinanceError::CategoryNameInvalid { .. }),
            },
            Case {
                name: "category name one character over its limit",
                input: new_tx(None, Some(&over_limit_category)),
                expect: |e| matches!(e, FinanceError::CategoryNameInvalid { .. }),
            },
        ];

        for case in cases {
            let err = create_transaction(&pool, None, "@alice:a", case.input)
                .await
                .expect_err(&format!("{} must be rejected", case.name));
            assert!(
                (case.expect)(&err),
                "{}: unexpected error variant {:?}",
                case.name,
                err
            );
        }
    }

    /// The bug this whole module exists to fix, for the Matrix path: an entry
    /// logged over Matrix never reached an open browser because the module
    /// never broadcast anything.
    #[tokio::test]
    async fn test_creating_a_transaction_announces_itself() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let tx = create_transaction(&pool, Some(&broadcaster), "@alice:a", new_tx(None, None))
            .await
            .unwrap();

        let event = rx.try_recv().expect("create must broadcast an event");
        match event {
            SystemEvent::TransactionCreated(created) => assert_eq!(created.id, tx.id),
            other => panic!("unexpected event: {:?}", other),
        }
    }

    /// A category named the same twice for the same owner must resolve to the
    /// same row rather than fragmenting into two — this is the behaviour the
    /// AI path already had and the Matrix path now shares.
    #[tokio::test]
    async fn test_category_resolution_reuses_an_existing_category_by_name() {
        let pool = setup().await;

        let first = create_transaction(&pool, None, "@alice:a", new_tx(None, Some("Food")))
            .await
            .unwrap();
        let second = create_transaction(&pool, None, "@alice:a", new_tx(None, Some("food")))
            .await
            .unwrap();

        assert_eq!(first.category_id, second.category_id);
        assert_eq!(
            FinanceRepository::list_categories(&pool, "@alice:a")
                .await
                .unwrap()
                .len(),
            1,
            "a second create with the same name must not fragment the category"
        );
    }

    /// The regression this module exists to fix: `resolve_category` used to
    /// create an unknown category unconditionally, which silently dropped the
    /// Matrix path's anti-typo rule (see `modules/finance/handler.rs`). With
    /// `create_missing_category: false`, an unknown name must be refused and,
    /// crucially, must leave no row behind — the point is the side effect, not
    /// just the error variant. With `true`, the same input must create the
    /// category and let the transaction through.
    #[tokio::test]
    async fn test_create_missing_category_flag_controls_the_side_effect() {
        let pool = setup().await;

        let mut refused = new_tx(None, Some("Groceries"));
        refused.create_missing_category = false;
        let err = create_transaction(&pool, None, "@alice:a", refused)
            .await
            .expect_err("an unknown category must be refused when creation is disabled");
        assert!(matches!(
            err,
            FinanceError::UnknownCategory { name } if name == "Groceries"
        ));
        assert_eq!(
            FinanceRepository::list_categories(&pool, "@alice:a")
                .await
                .unwrap()
                .len(),
            0,
            "a refused category must not be created as a side effect"
        );

        let mut allowed = new_tx(None, Some("Groceries"));
        allowed.create_missing_category = true;
        create_transaction(&pool, None, "@alice:a", allowed)
            .await
            .expect("an unknown category must be created when creation is allowed");
        assert_eq!(
            FinanceRepository::list_categories(&pool, "@alice:a")
                .await
                .unwrap()
                .len(),
            1,
            "the category must now exist"
        );
    }

    /// Deleting an owned category must announce itself, and must never touch
    /// another owner's category of the same id space.
    #[tokio::test]
    async fn test_deleting_a_category_announces_itself_and_checks_ownership() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let cat = create_category(&pool, None, "@alice:a", "Food")
            .await
            .unwrap();

        let err = delete_category(&pool, None, "@bob:a", cat.id)
            .await
            .unwrap_err();
        assert!(matches!(err, FinanceError::CategoryNotFound));

        let orphaned = delete_category(&pool, Some(&broadcaster), "@alice:a", cat.id)
            .await
            .unwrap();
        assert_eq!(orphaned, 0);

        let event = rx.try_recv().expect("delete must broadcast an event");
        assert!(matches!(event, SystemEvent::CategoryDeleted { .. }));
    }
}
