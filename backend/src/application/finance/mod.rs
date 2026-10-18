//! Finance application service.
//!
//! Owns business use cases and transactional-outbox event publishing for transactions,
//! categories, spending analytics, and public sharing.

mod analytics;
mod budgets;
mod categories;
mod transactions;

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::finance::FinanceError;

impl From<FinanceError> for AppError {
    fn from(err: FinanceError) -> Self {
        match err {
            FinanceError::TransactionNotFound => {
                AppError::NotFound("Entry not found or access denied".to_string())
            }
            FinanceError::CategoryNotFound => {
                AppError::NotFound("Category not found or access denied".to_string())
            }
            FinanceError::ShareNotFound => {
                AppError::NotFound("Share token not found or disabled".to_string())
            }
            FinanceError::CategoryExists => {
                AppError::Conflict("You already have a category with that name".to_string())
            }
            FinanceError::BudgetNotFound => {
                AppError::NotFound("Budget not found or access denied".to_string())
            }
            FinanceError::BudgetExists => AppError::Conflict(
                "You already have a budget for that category and currency".to_string(),
            ),
            other => AppError::Validation(other.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct FinanceService {
    pool: PgPool,
}

impl FinanceService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod tests;
