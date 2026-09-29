use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::audit::{AuditVerificationResult, PortfolioAuditEntry};
use crate::infrastructure::postgres::repositories::portfolio::audit as repo;

impl PortfolioService {
    pub async fn list_audit_entries(
        &self,
        owner: &AccountId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PortfolioAuditEntry>, AppError> {
        let list = repo::list_audit_entries(&self.pool, owner, limit, offset).await?;
        Ok(list)
    }

    pub async fn verify_audit_chain(
        &self,
        owner: &AccountId,
    ) -> Result<AuditVerificationResult, AppError> {
        let res = repo::verify_audit_chain(&self.pool, owner).await?;
        Ok(res)
    }
}
