use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::{
    Direction, FinanceError, ListTransactionsFilter, NewTransaction, Transaction, TransactionPatch,
    MAX_CATEGORY_LEN, MAX_NOTE_LEN,
};
use crate::infrastructure::postgres::repositories::finance as repo;

use super::FinanceService;

impl FinanceService {
    pub async fn create_transaction(
        &self,
        owner: &AccountId,
        input: NewTransaction<'_>,
    ) -> Result<Transaction, AppError> {
        if let Some(note) = input.note {
            if note.len() > MAX_NOTE_LEN {
                return Err(FinanceError::NoteTooLong { max: MAX_NOTE_LEN }.into());
            }
        }

        let category_id = self
            .resolve_category(owner, input.category, input.create_missing_category)
            .await?;

        let tx = repo::create_transaction(&self.pool, owner, &input, category_id).await?;

        self.events
            .send(SystemEvent::TransactionCreated(tx.clone()));
        Ok(tx)
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

        let updated = repo::update(
            &self.pool,
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

        self.events
            .send(SystemEvent::TransactionUpdated(updated.clone()));
        Ok(updated)
    }

    pub async fn delete_transaction(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Transaction, AppError> {
        let deleted = repo::delete(&self.pool, owner, id)
            .await?
            .ok_or(FinanceError::TransactionNotFound)?;

        self.events.send(SystemEvent::TransactionDeleted {
            id,
            room_id: deleted.room_id.clone(),
            owner_id: *owner,
        });

        Ok(deleted)
    }
}
