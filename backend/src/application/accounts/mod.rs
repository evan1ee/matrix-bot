//! Account application service: identity, authentication, linking, recovery, and security events.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub(crate) mod merge;
mod passkeys;
pub(crate) use passkeys::register_account_in;
#[cfg(test)]
pub(crate) use passkeys::test_passkey;
mod principal;
use merge::MergeService;

use crate::application::AppError;
use crate::domain::accounts::{
    Account, AccountId, AccountsError, MaturedRecovery, MergeOutcome, MergePreview, SecurityEvent,
    SecurityEventKind, Session,
};
use crate::infrastructure::postgres::repositories::accounts::{
    core, recovery, security_events, sessions,
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
}

impl AccountService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // --- Core Account Operations ---

    pub async fn find(&self, account_id: &AccountId) -> Result<Option<Account>, AppError> {
        Ok(core::find(&self.pool, account_id).await?)
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

    pub async fn count(&self) -> Result<i64, AppError> {
        Ok(core::count(&self.pool).await?)
    }

    pub async fn list_active_ids(&self) -> Result<Vec<AccountId>, AppError> {
        Ok(core::list_active_ids(&self.pool).await?)
    }

    pub async fn delete(&self, account_id: &AccountId) -> Result<(), AppError> {
        Ok(core::delete(&self.pool, account_id).await?)
    }

    pub async fn update_role(
        &self,
        account_id: &AccountId,
        role: crate::domain::accounts::AccountRole,
    ) -> Result<bool, AppError> {
        Ok(core::update_role(&self.pool, account_id, role).await?)
    }

    pub async fn has_passkey(&self, account_id: &AccountId) -> Result<bool, AppError> {
        Ok(self.count_passkeys(account_id).await? > 0)
    }

    /// Grants `admin` role to an active account that has at least one enrolled passkey.
    ///
    /// Used exclusively by the controlled CLI bootstrap (`matrix-bot admin grant <account_id>`).
    /// Public registration and passkey enrollment never call this method.
    pub async fn grant_admin(&self, account_id: &AccountId) -> Result<Account, AppError> {
        let Some(account) = core::find(&self.pool, account_id).await? else {
            return Err(AccountsError::AccountNotFound.into());
        };
        if account.is_unclaimed() {
            return Err(AppError::Validation(
                "Cannot grant administrator privileges to an unclaimed placeholder account"
                    .to_string(),
            ));
        }
        if !self.has_passkey(account_id).await? {
            return Err(AppError::Validation(
                "Account must have at least one enrolled passkey before administrator privileges can be granted"
                    .to_string(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        core::update_role(
            &mut *tx,
            account_id,
            crate::domain::accounts::AccountRole::Admin,
        )
        .await?;
        let entry = crate::domain::config::ConfigAuditEntry::new(
            None,
            crate::domain::config::ConfigAuditAction::AdminGranted,
            crate::domain::config::ConfigAuditTarget::Account(*account_id),
            crate::domain::config::ConfigAuditOutcome::Success,
            Some(serde_json::json!({
                "previous_role": account.role.as_str(),
                "new_role": "admin",
            })),
        );
        crate::infrastructure::postgres::repositories::config_audit::insert_audit_entry(
            &mut *tx,
            entry.actor_id.as_ref(),
            entry.action.as_str(),
            &entry.target_type,
            &entry.target_key,
            entry.outcome.as_str(),
            entry.detail.as_ref(),
        )
        .await?;
        let updated = core::find(&mut *tx, account_id)
            .await?
            .ok_or(AccountsError::AccountNotFound)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Revokes `admin` role from an account, demoting it to `user` and revoking all active sessions.
    ///
    /// Used by the controlled CLI recovery procedure (`matrix-bot admin revoke <account_id>`).
    pub async fn revoke_admin(&self, account_id: &AccountId) -> Result<Account, AppError> {
        let Some(account) = core::find(&self.pool, account_id).await? else {
            return Err(AccountsError::AccountNotFound.into());
        };

        let mut tx = self.pool.begin().await?;
        core::update_role(
            &mut *tx,
            account_id,
            crate::domain::accounts::AccountRole::User,
        )
        .await?;
        sessions::revoke_all_for_account(&mut *tx, account_id, None).await?;
        let entry = crate::domain::config::ConfigAuditEntry::new(
            None,
            crate::domain::config::ConfigAuditAction::AdminRevoked,
            crate::domain::config::ConfigAuditTarget::Account(*account_id),
            crate::domain::config::ConfigAuditOutcome::Success,
            Some(serde_json::json!({
                "previous_role": account.role.as_str(),
                "new_role": "user",
            })),
        );
        crate::infrastructure::postgres::repositories::config_audit::insert_audit_entry(
            &mut *tx,
            entry.actor_id.as_ref(),
            entry.action.as_str(),
            &entry.target_type,
            &entry.target_key,
            entry.outcome.as_str(),
            entry.detail.as_ref(),
        )
        .await?;
        let updated = core::find(&mut *tx, account_id)
            .await?
            .ok_or(AccountsError::AccountNotFound)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Controlled CLI entry point for `matrix-bot admin <grant|revoke> <account_id>`.
    pub async fn run_admin_cli(
        &self,
        action: &str,
        raw_account_id: &str,
    ) -> Result<String, AppError> {
        let account_id: AccountId = raw_account_id.trim().parse().map_err(|_| {
            AppError::Validation(format!(
                "Invalid account ID '{}': expected format acct_<uuid>",
                raw_account_id.trim()
            ))
        })?;
        match action {
            "grant" => {
                let updated = self.grant_admin(&account_id).await?;
                Ok(format!(
                    "Granted administrator privileges to {} ({})",
                    updated.display_name, updated.id
                ))
            }
            "revoke" => {
                let updated = self.revoke_admin(&account_id).await?;
                Ok(format!(
                    "Revoked administrator privileges from {} ({}) and terminated active sessions",
                    updated.display_name, updated.id
                ))
            }
            other => Err(AppError::Validation(format!(
                "Unknown admin subcommand '{}': expected 'grant' or 'revoke'",
                other
            ))),
        }
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

    pub async fn create_unverified_session(
        &self,
        account_id: &AccountId,
        user_agent: Option<&str>,
        ip: Option<&str>,
        restricted_until: Option<DateTime<Utc>>,
    ) -> Result<String, AppError> {
        let token =
            sessions::create_unverified(&self.pool, account_id, user_agent, ip, restricted_until)
                .await?;
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

    /// Signs out one of `account_id`'s own sessions. Returns whether a live
    /// session of that account was revoked; a hash belonging to another
    /// account (or already revoked) changes nothing and records nothing.
    pub async fn revoke_session(
        &self,
        account_id: &AccountId,
        id_hash: &str,
    ) -> Result<bool, AppError> {
        let revoked = sessions::revoke_for_account(&self.pool, account_id, id_hash).await?;
        if revoked {
            self.record_security_event(account_id, SecurityEventKind::SessionsRevoked, None)
                .await;
        }
        Ok(revoked)
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
        AccountService::new(pool)
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
        let from = crate::application::identity::IdentityService::new(service.pool().clone())
            .provision_matrix_for_test("@old:mbm.one")
            .await;
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
}
