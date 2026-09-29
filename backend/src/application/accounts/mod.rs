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

    /// Consumes a backup code. Redeeming one is the whole of that recovery, so
    /// it records both the use and the completion, once each; callers only
    /// open the restricted session.
    pub async fn redeem_recovery_code(&self, code: &str) -> Result<Option<AccountId>, AppError> {
        let maybe_acct = recovery::redeem(&self.pool, code).await?;
        if let Some(ref account_id) = maybe_acct {
            self.record_security_event(account_id, SecurityEventKind::RecoveryCodeUsed, None)
                .await;
            self.record_security_event(account_id, SecurityEventKind::RecoveryCompleted, None)
                .await;
        }
        Ok(maybe_acct)
    }

    /// Starts a Matrix-initiated recovery and records the single
    /// `recovery_requested` row; `detail` says who asked.
    pub async fn request_recovery(
        &self,
        account_id: &AccountId,
        detail: Option<&str>,
    ) -> Result<(String, DateTime<Utc>), AppError> {
        let res = recovery::request(&self.pool, account_id).await?;
        self.record_security_event(account_id, SecurityEventKind::RecoveryRequested, detail)
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

    /// Redeems a matured `!recover` code. Records `recovery_completed` itself;
    /// no caller may record it again.
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

    /// Merges `from` into `into` and records `accounts_merged` on `into`, once,
    /// after the merge has committed. A merge that fails records nothing.
    pub async fn merge(
        &self,
        from: &AccountId,
        into: &AccountId,
    ) -> Result<MergeOutcome, AppError> {
        let outcome = MergeService::merge(&self.pool, from, into).await?;
        self.record_security_event(
            into,
            SecurityEventKind::AccountsMerged,
            Some(&format!("{} rows", outcome.moved.total())),
        )
        .await;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::infrastructure::postgres::repositories::todos as todo_repo;

    async fn service() -> AccountService {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        AccountService::new(pool, EventBroadcaster::default())
    }

    /// `(kind, detail)` of every audit row of the account, oldest first.
    async fn audit(service: &AccountService, account: &AccountId) -> Vec<(String, Option<String>)> {
        let mut rows: Vec<_> = service
            .list_security_events(account, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.detail))
            .collect();
        rows.reverse();
        rows
    }

    async fn add_todo(service: &AccountService, owner: &AccountId) {
        todo_repo::create(
            service.pool(),
            owner,
            &NewTodo {
                room_id: "!r:x".to_string(),
                sender_name: "someone".to_string(),
                text: "item".to_string(),
                priority: TodoPriority::Medium,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            },
        )
        .await
        .unwrap();
    }

    /// R29: a completed merge leaves exactly one `accounts_merged` row on the
    /// surviving account, counting the moved rows. The HTTP handler used to
    /// write it; the service owns it now so no other caller can forget it.
    #[tokio::test]
    async fn merge_records_accounts_merged_once() {
        let service = service().await;
        let into = service.create("into", STATUS_ACTIVE).await.unwrap();
        let from = service.resolve_or_provision("@old:mbm.one").await.unwrap();
        add_todo(&service, &from).await;
        add_todo(&service, &from).await;

        let outcome = service.merge(&from, &into).await.unwrap();
        assert_eq!(outcome.moved.total(), 2);

        assert_eq!(
            audit(&service, &into).await,
            vec![(
                "accounts_merged".to_string(),
                Some(format!("{} rows", outcome.moved.total()))
            )]
        );
    }

    #[tokio::test]
    async fn claim_link_records_matrix_linked_once_on_success_only() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();

        // Refused outside a DM: the code is burned and nothing is recorded.
        let code = service.issue_link_challenge(&account).await.unwrap();
        let outcome = service.claim_link(&code, "@evan:mbm.one", false).await;
        assert_eq!(outcome.unwrap(), LinkOutcome::NotADirectMessage);
        assert_eq!(
            service
                .claim_link(&code, "@evan:mbm.one", true)
                .await
                .unwrap(),
            LinkOutcome::UnknownCode,
            "the code was burned by the refusal"
        );
        assert!(audit(&service, &account).await.is_empty());

        let code = service.issue_link_challenge(&account).await.unwrap();
        let outcome = service.claim_link(&code, "@evan:mbm.one", true).await;
        assert_eq!(
            outcome.unwrap(),
            LinkOutcome::Linked {
                account_id: account
            }
        );
        assert_eq!(
            audit(&service, &account).await,
            vec![(
                "matrix_linked".to_string(),
                Some("@evan:mbm.one".to_string())
            )]
        );
    }

    #[tokio::test]
    async fn claiming_an_mxid_on_a_claimed_account_records_nothing() {
        let service = service().await;
        let theirs = service.create("theirs", STATUS_ACTIVE).await.unwrap();
        let held = service.issue_link_challenge(&theirs).await.unwrap();
        service
            .claim_link(&held, "@evan:mbm.one", true)
            .await
            .unwrap();

        let mine = service.create("mine", STATUS_ACTIVE).await.unwrap();
        let code = service.issue_link_challenge(&mine).await.unwrap();
        let outcome = service.claim_link(&code, "@evan:mbm.one", true).await;
        assert_eq!(outcome.unwrap(), LinkOutcome::AlreadyLinked);
        assert!(audit(&service, &mine).await.is_empty());
    }

    #[tokio::test]
    async fn unlink_records_matrix_unlinked_only_when_it_removed_something() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let code = service.issue_link_challenge(&account).await.unwrap();
        service
            .claim_link(&code, "@evan:mbm.one", true)
            .await
            .unwrap();

        let stranger = service.create("stranger", STATUS_ACTIVE).await.unwrap();
        assert!(!service.unlink(&stranger, "@evan:mbm.one").await.unwrap());
        assert!(audit(&service, &stranger).await.is_empty());

        assert!(service.unlink(&account, "@evan:mbm.one").await.unwrap());
        assert!(!service.unlink(&account, "@evan:mbm.one").await.unwrap());
        assert_eq!(
            audit(&service, &account).await,
            vec![
                (
                    "matrix_linked".to_string(),
                    Some("@evan:mbm.one".to_string())
                ),
                (
                    "matrix_unlinked".to_string(),
                    Some("@evan:mbm.one".to_string())
                ),
            ]
        );
    }
}
