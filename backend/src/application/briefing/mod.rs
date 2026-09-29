use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::domain::settings::UserSettings;
use anyhow::Result;

pub mod generator;
pub mod models;
pub mod ticker;

pub use models::{BriefingPayload, BriefingSummary, TickerPerformance};

#[derive(Clone)]
pub struct BriefingService {
    pool: DbPool,
}

impl BriefingService {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn generate_briefing(
        &self,
        owner_id: &AccountId,
        settings: &UserSettings,
    ) -> Result<BriefingPayload> {
        generator::generate_briefing(&self.pool, owner_id, settings).await
    }
}
