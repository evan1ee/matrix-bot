pub mod alerts;
pub mod audit;
pub mod cash;
pub mod history;
pub mod holdings;
pub mod prices;
pub mod scheduling;
pub mod trades;

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::portfolio::PortfolioError;

pub use alerts::TriggeredAlert;
pub use prices::RefreshSummary;

#[derive(Clone)]
pub struct PortfolioService {
    pub(crate) pool: PgPool,
}

impl PortfolioService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl From<PortfolioError> for AppError {
    fn from(err: PortfolioError) -> Self {
        AppError::Validation(err.to_string())
    }
}

#[cfg(test)]
mod tests;
