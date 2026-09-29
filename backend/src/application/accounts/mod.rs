//! Account application service: identity, authentication, linking, recovery, and security events.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub mod merge;
mod passkeys;
mod principal;
pub use merge::MergeService;

use crate::application::AppError;
use crate::domain::accounts::{
    Account, AccountId, AccountsError, LinkOutcome, MaturedRecovery, MergeOutcome, MergePreview,
    SecurityEvent, SecurityEventKind, Session,
};
use crate::events::EventBroadcaster;
use crate::infrastructure::postgres::repositories::accounts::{
    core, linking, recovery, security_events, sessions,
};

impl From<AccountsError> for AppError {
    fn from(err: AccountsError) -> Self {
        match err {
            AccountsError::AccountNotFound => AppError::NotFound("Account not found".to_string()),
            AccountsError::WebOnlyCommand(msg) => AppError::Forbidden(msg.to_string()),
            AccountsError::FirstPasskeyOnly => AppError::Forbidden(
                "An account that already has a passkey cannot add another via Matrix".to_string(),
            ),
            AccountsError::NotADirectMessage => {
                AppError::Validation("Send this command in a direct message".to_string())
            }
            AccountsError::InvalidCode => {
                AppError::Validation("Invalid or expired code".to_string())
            }
            AccountsError::AlreadyLinked => AppError::Conflict(
                "This Matrix account is already linked to another account".to_string(),
            ),
            AccountsError::InvalidRecoveryCode => {
                AppError::Validation("Invalid recovery code".to_string())
            }
            AccountsError::RecoveryNotMatured => {
                AppError::Forbidden("Recovery is not yet matured".to_string())
            }
            AccountsError::Validation(msg) => AppError::Validation(msg),
        }
    }
}

#[derive(Clone)]
pub struct AccountService {
    pool: PgPool,
    #[allow(dead_code)]
    events: EventBroadcaster,
}

impl AccountService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // --- Core Account Operations ---

    pub async fn resolve_or_provision(&self, mxid: &str) -> Result<AccountId, AppError> {
        Ok(core::resolve_or_provision(&self.pool, mxid).await?)
    }

    pub async fn find(&self, account_id: &AccountId) -> Result<Option<Account>, AppError> {
        Ok(core::find(&self.pool, account_id).await?)
    }

    pub async fn find_by_mxid(&self, mxid: &str) -> Result<Option<AccountId>, AppError> {
        Ok(core::find_by_mxid(&self.pool, mxid).await?)
    }

    pub async fn linked_mxids(&self, account_id: &AccountId) -> Result<Vec<String>, AppError> {
        Ok(core::linked_mxids(&self.pool, account_id).await?)
    }

    pub async fn create(&self, display_name: &str, status: &str) -> Result<AccountId, AppError> {
        Ok(core::create(&self.pool, display_name, status).await?)
    }

    pub async fn create_with_id(
        &self,
        account_id: &AccountId,
        display_name: &str,
        status: &str,
    ) -> Result<(), AppError> {
        Ok(core::create_with_id(&self.pool, account_id, display_name, status).await?)
    }

    pub async fn activate_or_create(
        &self,
        account_id: &AccountId,
        display_name: &str,
    ) -> Result<(), AppError> {
        Ok(core::activate_or_create(&self.pool, account_id, display_name).await?)
    }

    pub async fn count(&self) -> Result<i64, AppError> {
        Ok(core::count(&self.pool).await?)
    }

    pub async fn list_active_ids(&self) -> Result<Vec<AccountId>, AppError> {
        Ok(core::list_active_ids(&self.pool).await?)
    }

    pub async fn delete(&self, account_id: &AccountId) -> Result<(), AppError> {
        Ok(core::delete(&self.pool, account_id).await?)
    }

    // --- Sessions ---

    pub async fn create_session(
        &self,
        account_id: &AccountId,
        user_agent: Option<&str>,
        ip: Option<&str>,
        restricted_until: Option<DateTime<Utc>>,
    ) -> Result<String, AppError> {
        let token =
            sessions::create(&self.pool, account_id, user_agent, ip, restricted_until).await?;
        Ok(token)
    }

    pub async fn resolve_session(&self, token: &str) -> Result<Option<Session>, AppError> {
        Ok(sessions::resolve(&self.pool, token).await?)
    }

    pub async fn mark_session_verified(&self, id_hash: &str) -> Result<(), AppError> {
        Ok(sessions::mark_verified(&self.pool, id_hash).await?)
    }

    /// Stamps every session of the account as freshly verified. Records nothing.
    pub async fn update_last_verified_at(
        &self,
        account_id: &AccountId,
        verified_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        Ok(sessions::update_last_verified_at(&self.pool, account_id, verified_at).await?)
    }

    pub async fn revoke_session(
        &self,
        account_id: &AccountId,
        id_hash: &str,
    ) -> Result<(), AppError> {
        sessions::revoke(&self.pool, id_hash).await?;
        self.record_security_event(account_id, SecurityEventKind::SessionsRevoked, None)
            .await;
        Ok(())
    }

    pub async fn revoke_all_sessions(
        &self,
        account_id: &AccountId,
        except: Option<&str>,
    ) -> Result<u64, AppError> {
        let count = sessions::revoke_all_for_account(&self.pool, account_id, except).await?;
        self.record_security_event(account_id, SecurityEventKind::SessionsRevoked, None)
            .await;
        Ok(count)
    }

    pub async fn list_sessions(&self, account_id: &AccountId) -> Result<Vec<Session>, AppError> {
        Ok(sessions::list_for_account(&self.pool, account_id).await?)
    }

    // --- Linking ---

    pub async fn issue_link_challenge(&self, account_id: &AccountId) -> Result<String, AppError> {
        Ok(linking::issue(&self.pool, account_id).await?)
    }

    pub async fn claim_link(
        &self,
        code: &str,
        mxid: &str,
        is_direct: bool,
    ) -> Result<LinkOutcome, AppError> {
        let outcome = linking::claim(&self.pool, code, mxid, is_direct).await?;
        if let LinkOutcome::Linked { ref account_id } = outcome {
            self.record_security_event(account_id, SecurityEventKind::MatrixLinked, Some(mxid))
                .await;
        }
        Ok(outcome)
    }

    pub async fn unlink(&self, account_id: &AccountId, mxid: &str) -> Result<bool, AppError> {
        let ok = linking::unlink(&self.pool, account_id, mxid).await?;
        if ok {
            self.record_security_event(account_id, SecurityEventKind::MatrixUnlinked, Some(mxid))
                .await;
        }
        Ok(ok)
    }

    // --- Recovery ---

    pub async fn issue_recovery_codes(
        &self,
        account_id: &AccountId,
    ) -> Result<Vec<String>, AppError> {
        let codes = recovery::issue_codes(&self.pool, account_id).await?;
        self.record_security_event(account_id, SecurityEventKind::RecoveryCodesIssued, None)
            .await;
        Ok(codes)
    }

    pub async fn remaining_recovery_codes(&self, account_id: &AccountId) -> Result<i64, AppError> {
        Ok(recovery::remaining(&self.pool, account_id).await?)
    }

    pub async fn redeem_recovery_code(&self, code: &str) -> Result<Option<AccountId>, AppError> {
        let maybe_acct = recovery::redeem(&self.pool, code).await?;
        if let Some(ref account_id) = maybe_acct {
            self.record_security_event(account_id, SecurityEventKind::RecoveryCodeUsed, None)
                .await;
        }
        Ok(maybe_acct)
    }

    pub async fn request_recovery(
        &self,
        account_id: &AccountId,
    ) -> Result<(String, DateTime<Utc>), AppError> {
        let res = recovery::request(&self.pool, account_id).await?;
        self.record_security_event(account_id, SecurityEventKind::RecoveryRequested, None)
            .await;
        Ok(res)
    }

    pub async fn cancel_recovery(&self, account_id: &AccountId) -> Result<u64, AppError> {
        let count = recovery::cancel_all(&self.pool, account_id).await?;
        if count > 0 {
            self.record_security_event(account_id, SecurityEventKind::RecoveryCancelled, None)
                .await;
        }
        Ok(count)
    }

    pub async fn pending_recovery(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        Ok(recovery::pending(&self.pool, account_id).await?)
    }

    pub async fn redeem_recovery_request(&self, code: &str) -> Result<MaturedRecovery, AppError> {
        let res = recovery::redeem_request(&self.pool, code).await?;
        if let MaturedRecovery::Ready { ref account_id } = res {
            self.record_security_event(account_id, SecurityEventKind::RecoveryCompleted, None)
                .await;
        }
        Ok(res)
    }

    // --- Security Events ---

    pub async fn record_security_event(
        &self,
        account_id: &AccountId,
        kind: SecurityEventKind,
        detail: Option<&str>,
    ) {
        security_events::record(&self.pool, account_id, kind, detail).await;
    }

    pub async fn list_security_events(
        &self,
        account_id: &AccountId,
        limit: i64,
    ) -> Result<Vec<SecurityEvent>, AppError> {
        Ok(security_events::list(&self.pool, account_id, limit).await?)
    }

    // --- Merge ---

    pub async fn preview_merge(&self, from: &AccountId) -> Result<MergePreview, AppError> {
        MergeService::preview(&self.pool, from).await
    }

    pub async fn merge(
        &self,
        from: &AccountId,
        into: &AccountId,
    ) -> Result<MergeOutcome, AppError> {
        MergeService::merge(&self.pool, from, into).await
    }
}
