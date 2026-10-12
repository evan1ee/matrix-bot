use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::{Category, FinanceError, MAX_CATEGORY_LEN};
use crate::infrastructure::postgres::repositories::finance as repo;
use crate::infrastructure::postgres::repositories::outbox;

use super::FinanceService;

impl FinanceService {
    pub async fn create_category(
        &self,
        owner: &AccountId,
        name: &str,
    ) -> Result<Category, AppError> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_CATEGORY_LEN {
            return Err(FinanceError::CategoryNameInvalid {
                max: MAX_CATEGORY_LEN,
            }
            .into());
        }

        let mut tx = self.pool.begin().await?;
        let cat = repo::create_category(&mut *tx, owner, trimmed)
            .await?
            .ok_or(FinanceError::CategoryExists)?;
        outbox::publish(&mut tx, &SystemEvent::CategoryCreated(cat.clone())).await?;
        tx.commit().await?;
        Ok(cat)
    }

    pub async fn list_categories(&self, owner: &AccountId) -> Result<Vec<Category>, AppError> {
        let cats = repo::list_categories(&self.pool, owner).await?;
        Ok(cats)
    }

    pub async fn delete_category(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Option<u64>, AppError> {
        let mut tx = self.pool.begin().await?;
        let orphaned = repo::delete_category(&mut tx, owner, id).await?;
        if orphaned.is_some() {
            outbox::publish(
                &mut tx,
                &SystemEvent::CategoryDeleted {
                    id,
                    owner_id: *owner,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(orphaned)
    }

    pub async fn resolve_category(
        &self,
        owner: &AccountId,
        name: Option<&str>,
        create_missing: bool,
    ) -> Result<Option<i64>, AppError> {
        let Some(name) = name else {
            return Ok(None);
        };
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_CATEGORY_LEN {
            return Err(FinanceError::CategoryNameInvalid {
                max: MAX_CATEGORY_LEN,
            }
            .into());
        }

        if let Some(cat) = repo::find_category_by_name(&self.pool, owner, trimmed).await? {
            return Ok(Some(cat.id));
        }

        if create_missing {
            let cat = self.create_category(owner, trimmed).await?;
            Ok(Some(cat.id))
        } else {
            Err(FinanceError::UnknownCategory {
                name: trimmed.to_string(),
            }
            .into())
        }
    }

    pub async fn resolve_category_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        owner: &AccountId,
        name: Option<&str>,
        create_missing: bool,
    ) -> Result<Option<i64>, AppError> {
        let Some(name) = name else {
            return Ok(None);
        };
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_CATEGORY_LEN {
            return Err(FinanceError::CategoryNameInvalid {
                max: MAX_CATEGORY_LEN,
            }
            .into());
        }

        if let Some(cat) = repo::find_category_by_name(&mut **tx, owner, trimmed).await? {
            return Ok(Some(cat.id));
        }

        if create_missing {
            if let Some(cat) = repo::create_category(&mut **tx, owner, trimmed).await? {
                outbox::publish(tx, &SystemEvent::CategoryCreated(cat.clone())).await?;
                return Ok(Some(cat.id));
            }
            if let Some(cat) = repo::find_category_by_name(&mut **tx, owner, trimmed).await? {
                return Ok(Some(cat.id));
            }
            Err(FinanceError::CategoryExists.into())
        } else {
            Err(FinanceError::UnknownCategory {
                name: trimmed.to_string(),
            }
            .into())
        }
    }
}
