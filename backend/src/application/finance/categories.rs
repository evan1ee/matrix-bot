use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::finance::{Category, FinanceError, MAX_CATEGORY_LEN};
use crate::events::SystemEvent;
use crate::infrastructure::postgres::repositories::finance as repo;

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

        let cat = repo::create_category(&self.pool, owner, trimmed)
            .await?
            .ok_or(FinanceError::CategoryExists)?;

        self.events.send(SystemEvent::CategoryCreated(cat.clone()));
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
        let orphaned = repo::delete_category(&self.pool, owner, id).await?;
        if orphaned.is_some() {
            self.events.send(SystemEvent::CategoryDeleted {
                id,
                owner_id: owner.to_string(),
            });
        }
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
}
