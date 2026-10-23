//! Budget use cases: CRUD and month status. Threshold alerting is layered on top of the
//! transaction paths separately.

use chrono::Utc;
use chrono_tz::Tz;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::budget::{
    status, Budget, BudgetPatch, BudgetScope, BudgetStatus, Month, NewBudget,
};
use crate::domain::finance::money::normalise_currency;
use crate::domain::finance::FinanceError;
use crate::infrastructure::postgres::repositories::finance::budgets as repo;
use crate::infrastructure::postgres::repositories::{outbox, settings};

use super::FinanceService;

/// A duplicate (owner, scope, currency) is a conflict, not a database failure.
fn map_insert_error(err: sqlx::Error) -> AppError {
    match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => FinanceError::BudgetExists.into(),
        _ => err.into(),
    }
}

/// What `budget` has spent, given a month's spend grouped by (category, currency).
pub(super) fn spent_for(budget: &Budget, rows: &[repo::SpendRow]) -> i64 {
    match &budget.scope {
        BudgetScope::Overall => rows
            .iter()
            .filter(|r| r.currency == budget.currency)
            .fold(0_i64, |acc, r| acc.saturating_add(r.spent_minor)),
        BudgetScope::Category { id, .. } => rows
            .iter()
            .filter(|r| r.category_id == Some(*id) && r.currency == budget.currency)
            .fold(0_i64, |acc, r| acc.saturating_add(r.spent_minor)),
    }
}

impl FinanceService {
    pub async fn create_budget(&self, owner: &AccountId, b: NewBudget) -> Result<Budget, AppError> {
        let mut tx = self.pool.begin().await?;
        let category_id = self
            .resolve_category_in_tx(&mut tx, owner, b.category(), false)
            .await?;
        let created = repo::insert(&mut *tx, owner, category_id, &b)
            .await
            .map_err(map_insert_error)?;
        outbox::publish(&mut tx, &SystemEvent::BudgetCreated(created.clone())).await?;
        tx.commit().await?;
        Ok(created)
    }

    /// Creates the budget for its (category, currency) scope, or changes the limit and
    /// alert flag of the one already there. `true` means it was created.
    pub async fn set_budget(
        &self,
        owner: &AccountId,
        b: NewBudget,
    ) -> Result<(Budget, bool), AppError> {
        let mut tx = self.pool.begin().await?;
        let category_id = self
            .resolve_category_in_tx(&mut tx, owner, b.category(), false)
            .await?;
        let existing =
            repo::find_by_scope_for_update(&mut *tx, owner, category_id, b.currency()).await?;
        let (budget, created) = match existing {
            Some(found) => {
                let patch = BudgetPatch {
                    limit_minor: Some(b.limit_minor()),
                    alerts_enabled: b.alerts_enabled(),
                };
                let updated = repo::update(&mut *tx, owner, found.id, &patch)
                    .await?
                    .ok_or(FinanceError::BudgetNotFound)?;
                outbox::publish(&mut tx, &SystemEvent::BudgetUpdated(updated.clone())).await?;
                (updated, false)
            }
            None => {
                let inserted = repo::insert(&mut *tx, owner, category_id, &b)
                    .await
                    .map_err(map_insert_error)?;
                outbox::publish(&mut tx, &SystemEvent::BudgetCreated(inserted.clone())).await?;
                (inserted, true)
            }
        };
        tx.commit().await?;
        Ok((budget, created))
    }

    pub async fn update_budget(
        &self,
        owner: &AccountId,
        id: i64,
        patch: BudgetPatch,
    ) -> Result<Budget, AppError> {
        patch.validate()?;
        let mut tx = self.pool.begin().await?;
        let updated = repo::update(&mut *tx, owner, id, &patch)
            .await?
            .ok_or(FinanceError::BudgetNotFound)?;
        outbox::publish(&mut tx, &SystemEvent::BudgetUpdated(updated.clone())).await?;
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn delete_budget(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        if !repo::delete(&mut *tx, owner, id).await? {
            return Err(FinanceError::BudgetNotFound.into());
        }
        outbox::publish(
            &mut tx,
            &SystemEvent::BudgetDeleted {
                id,
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_budgets(&self, owner: &AccountId) -> Result<Vec<Budget>, AppError> {
        Ok(repo::list(&self.pool, owner).await?)
    }

    /// The single budget for a scope (`category: None` is overall). Without a currency the
    /// scope must have exactly one budget.
    pub async fn resolve_budget(
        &self,
        owner: &AccountId,
        category: Option<&str>,
        currency: Option<&str>,
    ) -> Result<Budget, AppError> {
        let category = category.map(str::trim).filter(|c| !c.is_empty());
        let category_id = self.resolve_category(owner, category, false).await?;
        let currency = currency
            .map(normalise_currency)
            .transpose()
            .map_err(FinanceError::from)?;
        let mut matches = repo::list_for_scope(&self.pool, owner, category_id).await?;
        if let Some(currency) = &currency {
            matches.retain(|b| &b.currency == currency);
        }
        match matches.len() {
            0 => Err(FinanceError::BudgetNotFound.into()),
            1 => Ok(matches.remove(0)),
            _ => Err(FinanceError::BudgetAmbiguous {
                scope: category.unwrap_or("overall").to_string(),
            }
            .into()),
        }
    }

    /// Every budget measured against `month` (default: the owner's current month), overall
    /// budgets first, then by share of the limit used, highest first.
    pub async fn budget_status(
        &self,
        owner: &AccountId,
        month: Option<Month>,
    ) -> Result<Vec<BudgetStatus>, AppError> {
        let tz = self.owner_tz(owner).await?;
        let now = Utc::now();
        let month = month.unwrap_or_else(|| Month::containing(now, tz));
        let budgets = repo::list(&self.pool, owner).await?;
        let rows = repo::month_spend(
            &self.pool,
            owner,
            month.start_utc(tz),
            month.end_utc(tz),
            None,
        )
        .await?;
        let mut statuses: Vec<BudgetStatus> = budgets
            .into_iter()
            .map(|b| {
                let spent = spent_for(&b, &rows);
                status(b, spent, month, now, tz)
            })
            .collect();
        statuses.sort_by_key(|s| {
            (
                !matches!(s.budget.scope, BudgetScope::Overall),
                std::cmp::Reverse(s.percent_used),
            )
        });
        Ok(statuses)
    }

    /// One budget measured against `month`; `None` when it is not this owner's.
    pub async fn status_for_budget(
        &self,
        owner: &AccountId,
        id: i64,
        month: Month,
    ) -> Result<Option<BudgetStatus>, AppError> {
        let Some(budget) = repo::find_owned(&self.pool, owner, id).await? else {
            return Ok(None);
        };
        let tz = self.owner_tz(owner).await?;
        let rows = repo::month_spend(
            &self.pool,
            owner,
            month.start_utc(tz),
            month.end_utc(tz),
            Some(&budget.currency),
        )
        .await?;
        let spent = spent_for(&budget, &rows);
        Ok(Some(status(budget, spent, month, Utc::now(), tz)))
    }

    async fn owner_tz(&self, owner: &AccountId) -> Result<Tz, AppError> {
        Ok(settings::get(&self.pool, owner)
            .await?
            .unwrap_or_default()
            .tz())
    }
}
