use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::{
    Direction, FinanceError, ListTransactionsFilter, NewTransaction, Transaction, TransactionPatch,
    MAX_CATEGORY_LEN, MAX_NOTE_LEN,
};
use crate::infrastructure::postgres::repositories::finance as repo;
use crate::infrastructure::postgres::repositories::outbox;

use super::FinanceService;

impl FinanceService {
    pub async fn create_transaction(
        &self,
        owner: &AccountId,
        input: NewTransaction<'_>,
    ) -> Result<Transaction, AppError> {
        let mut dbtx = self.pool.begin().await?;
        let tx = self
            .create_transaction_in_tx(&mut dbtx, owner, input)
            .await?;
        dbtx.commit().await?;
        Ok(tx)
    }

    pub async fn create_transaction_in_tx(
        &self,
        dbtx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        owner: &AccountId,
        input: NewTransaction<'_>,
    ) -> Result<Transaction, AppError> {
        if let Some(note) = input.note {
            if note.len() > MAX_NOTE_LEN {
                return Err(FinanceError::NoteTooLong { max: MAX_NOTE_LEN }.into());
            }
        }

        let category_id = self
            .resolve_category_in_tx(dbtx, owner, input.category, input.create_missing_category)
            .await?;

        let tx = repo::create_transaction(dbtx, owner, &input, category_id).await?;
        outbox::publish(dbtx, &SystemEvent::TransactionCreated(tx.clone())).await?;
        Ok(tx)
    }

    /// Convenience helper to record an expense transaction, optionally linking a receipt storage key.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_expense(
        &self,
        owner: &AccountId,
        amount_minor: i64,
        currency: &str,
        currency_exponent: i64,
        category: Option<&str>,
        note: Option<&str>,
        receipt_storage_key: Option<crate::domain::storage::StorageKey>,
        room_id: &str,
        sender_name: &str,
    ) -> Result<Transaction, AppError> {
        self.create_transaction(
            owner,
            NewTransaction {
                direction: Direction::Expense,
                amount_minor,
                currency,
                currency_exponent,
                category,
                note,
                receipt_storage_key,
                room_id,
                sender_name,
                occurred_at: chrono::Utc::now(),
                source: None,
                create_missing_category: true,
            },
        )
        .await
    }

    pub async fn get_transaction(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Option<Transaction>, AppError> {
        let tx = repo::find_owned(&self.pool, owner, id).await?;
        Ok(tx)
    }

    pub async fn list_transactions(
        &self,
        owner: &AccountId,
        filter: &ListTransactionsFilter<'_>,
    ) -> Result<Vec<Transaction>, AppError> {
        let list = repo::list(&self.pool, owner, filter).await?;
        Ok(list)
    }

    pub async fn count_transactions(
        &self,
        owner: &AccountId,
        direction: Option<Direction>,
    ) -> Result<i64, AppError> {
        let count = repo::count(&self.pool, owner, direction).await?;
        Ok(count)
    }

    pub async fn update_transaction(
        &self,
        owner: &AccountId,
        id: i64,
        patch: TransactionPatch<'_>,
    ) -> Result<Transaction, AppError> {
        let existing = repo::find_owned(&self.pool, owner, id)
            .await?
            .ok_or(FinanceError::TransactionNotFound)?;

        if let Some(Some(note)) = patch.note {
            if note.len() > MAX_NOTE_LEN {
                return Err(FinanceError::NoteTooLong { max: MAX_NOTE_LEN }.into());
            }
        }

        let category_id = match patch.category {
            None => existing.category_id,
            Some(None) => None,
            Some(Some(cat_name)) => {
                let trimmed = cat_name.trim();
                if trimmed.is_empty() || trimmed.len() > MAX_CATEGORY_LEN {
                    return Err(FinanceError::CategoryNameInvalid {
                        max: MAX_CATEGORY_LEN,
                    }
                    .into());
                }
                let cat = repo::find_category_by_name(&self.pool, owner, trimmed)
                    .await?
                    .ok_or_else(|| FinanceError::UnknownCategory {
                        name: trimmed.to_string(),
                    })?;
                Some(cat.id)
            }
        };

        let direction = patch.direction.unwrap_or(existing.direction);
        let amount_minor = patch.amount_minor.unwrap_or(existing.amount_minor);
        let currency = patch.currency.unwrap_or(&existing.currency);
        let currency_exponent = patch
            .currency_exponent
            .unwrap_or(existing.currency_exponent);
        let note = match patch.note {
            Some(n) => n,
            None => existing.note.as_deref(),
        };
        let occurred_at = patch.occurred_at.unwrap_or(existing.occurred_at);

        let mut tx = self.pool.begin().await?;
        let updated = repo::update(
            &mut tx,
            owner,
            id,
            direction,
            amount_minor,
            currency,
            currency_exponent,
            category_id,
            note,
            occurred_at,
        )
        .await?
        .ok_or(FinanceError::TransactionNotFound)?;
        outbox::publish(&mut tx, &SystemEvent::TransactionUpdated(updated.clone())).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Attaches or updates a receipt storage key on an existing transaction owned by `owner`.
    pub async fn link_receipt(
        &self,
        owner: &AccountId,
        transaction_id: i64,
        storage_key: crate::domain::storage::StorageKey,
    ) -> Result<Transaction, AppError> {
        let mut tx = self.pool.begin().await?;
        let updated =
            repo::set_receipt_storage_key(&mut tx, owner, transaction_id, Some(&storage_key))
                .await?
                .ok_or(FinanceError::TransactionNotFound)?;
        outbox::publish(&mut tx, &SystemEvent::TransactionUpdated(updated.clone())).await?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn delete_transaction(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Transaction, AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repo::delete(&mut tx, owner, id)
            .await?
            .ok_or(FinanceError::TransactionNotFound)?;
        outbox::publish(
            &mut tx,
            &SystemEvent::TransactionDeleted {
                id,
                room_id: deleted.room_id.clone(),
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;

        Ok(deleted)
    }
}
