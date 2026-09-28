pub mod alerts;
pub mod audit;
pub mod cash;
pub mod history;
pub mod holdings;
pub mod prices;
pub mod trades;

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::portfolio::PortfolioError;
use crate::events::EventBroadcaster;

pub use prices::RefreshSummary;

#[derive(Clone)]
pub struct PortfolioService {
    pub(crate) pool: PgPool,
    pub(crate) events: EventBroadcaster,
}

impl PortfolioService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn events(&self) -> &EventBroadcaster {
        &self.events
    }
}

impl From<PortfolioError> for AppError {
    fn from(err: PortfolioError) -> Self {
        AppError::Validation(err.to_string())
    }
}
