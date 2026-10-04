//! Resolving a presented credential to the account it acts as, and ending a
//! session.

use crate::application::AppError;
use crate::domain::accounts::{Credential, Principal};
use crate::infrastructure::postgres::repositories::accounts::{core, sessions};
use crate::infrastructure::postgres::repositories::mcp as mcp_repo;

use super::AccountService;

impl AccountService {
    /// Resolves a credential to the account it acts as.
    ///
    /// `Ok(None)` = expired, revoked, unknown, or naming a deleted account (the
    /// caller answers 401; the reason is not exposed).
    pub async fn resolve_credential(
        &self,
        credential: &Credential<'_>,
    ) -> Result<Option<Principal>, AppError> {
        match credential {
            Credential::McpToken(token) => self.resolve_mcp_token(token).await,
            Credential::Cookie(token) | Credential::BearerSession(token) => {
                self.resolve_session_token(token).await
            }
        }
    }

    async fn resolve_mcp_token(&self, token: &str) -> Result<Option<Principal>, AppError> {
        let Some(mcp_token) = mcp_repo::resolve(&self.pool, token).await? else {
            tracing::debug!("MCP token is invalid or has expired");
            return Ok(None);
        };

        // Best-effort last-used stamp, never awaited by the request; bounded by
        // the pool's acquire timeout.
        let pool_for_touch = self.pool.clone();
        let token_id = mcp_token.id.clone();
        tokio::spawn(async move {
            let _ = mcp_repo::touch_last_used(&pool_for_touch, &token_id).await;
        });

        let Some(account) = core::find(&self.pool, &mcp_token.account_id).await? else {
            tracing::debug!("MCP token names an account that no longer exists");
            return Ok(None);
        };

        Ok(Some(Principal {
            account_id: mcp_token.account_id,
            display_name: account.display_name,
            session_id_hash: mcp_token.token_hash,
            // Never eligible for step-up. A personal access token is a machine
            // credential with nobody at the far end to confirm anything, so it
            // must not open a `VerifiedUser` route — and it did, for the first
            // five minutes after it was minted, because it reported its
            // creation time as the moment a passkey was last presented.
            last_verified_at: chrono::DateTime::<chrono::Utc>::MIN_UTC,
            restricted_until: None,
        }))
    }

    async fn resolve_session_token(&self, token: &str) -> Result<Option<Principal>, AppError> {
        let Some(session) = sessions::resolve(&self.pool, token).await? else {
            tracing::debug!("session token is invalid or has expired");
            return Ok(None);
        };

        // The display name is read per request rather than copied into the
        // session, so renaming an account takes effect immediately instead of
        // on next sign-in.
        let Some(account) = core::find(&self.pool, &session.account_id).await? else {
            // A session whose account is gone is not a session.
            tracing::debug!("session names an account that no longer exists");
            return Ok(None);
        };

        Ok(Some(Principal {
            account_id: session.account_id,
            display_name: account.display_name,
            session_id_hash: session.id_hash,
            last_verified_at: session.last_verified_at,
            restricted_until: session.restricted_until,
        }))
    }

    /// Logout: revokes one session and records nothing (parity with the
    /// legacy session facade this replaces; `revoke_session`
    /// audits, logout never did).
    pub async fn end_session(&self, id_hash: &str) -> Result<(), AppError> {
        Ok(sessions::revoke(&self.pool, id_hash).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{hash_token, STATUS_ACTIVE};
    use chrono::{DateTime, Utc};

    async fn service() -> AccountService {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        AccountService::new(pool)
    }

    #[tokio::test]
    async fn an_mcp_token_never_satisfies_step_up() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let (raw, _) = mcp_repo::create(service.pool(), &account, "a machine", None)
            .await
            .unwrap();

        let principal = service
            .resolve_credential(&Credential::McpToken(&raw))
            .await
            .unwrap()
            .expect("a fresh MCP token must authenticate");

        assert_eq!(principal.account_id, account);
        assert_eq!(principal.last_verified_at, DateTime::<Utc>::MIN_UTC);
        assert_eq!(principal.restricted_until, None);
    }

    #[tokio::test]
    async fn a_session_naming_a_deleted_account_resolves_to_nothing() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let token = service
            .create_session(&account, None, None, None)
            .await
            .unwrap();
        service.delete(&account).await.unwrap();

        let resolved = service
            .resolve_credential(&Credential::Cookie(&token))
            .await
            .unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn a_cookie_and_a_bearer_session_resolve_to_the_same_principal() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let token = service
            .create_session(&account, None, None, None)
            .await
            .unwrap();

        let cookie = service
            .resolve_credential(&Credential::Cookie(&token))
            .await
            .unwrap()
            .unwrap();
        let bearer = service
            .resolve_credential(&Credential::BearerSession(&token))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(cookie.session_id_hash, hash_token(&token));
        assert_eq!(cookie.account_id, account);
        assert_eq!(cookie.display_name, "evan");
        assert_eq!(cookie, bearer);
    }

    #[tokio::test]
    async fn logout_records_no_security_event() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let token = service
            .create_session(&account, None, None, None)
            .await
            .unwrap();
        let before = service.list_security_events(&account, 50).await.unwrap();

        service.end_session(&hash_token(&token)).await.unwrap();

        let after = service.list_security_events(&account, 50).await.unwrap();
        assert_eq!(before.len(), after.len(), "logout must audit nothing");
        assert!(
            service
                .resolve_credential(&Credential::Cookie(&token))
                .await
                .unwrap()
                .is_none(),
            "the session must be revoked"
        );
    }

    /// The other half of the parity rule: the audited variant still audits.
    #[tokio::test]
    async fn revoking_through_the_audited_method_records_one_event() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let token = service
            .create_session(&account, None, None, None)
            .await
            .unwrap();
        let before = service
            .list_security_events(&account, 50)
            .await
            .unwrap()
            .len();

        let revoked = service
            .revoke_session(&account, &hash_token(&token))
            .await
            .unwrap();
        assert!(revoked);

        let after = service
            .list_security_events(&account, 50)
            .await
            .unwrap()
            .len();
        assert_eq!(after, before + 1);
    }

    /// Given a session of account B, When account A "revokes" it by hash,
    /// Then B's session survives, nothing is reported revoked and neither
    /// account gets an audit row.
    #[tokio::test]
    async fn revoking_another_accounts_session_does_nothing() {
        let service = service().await;
        let a = service.create("alice", STATUS_ACTIVE).await.unwrap();
        let b = service.create("bob", STATUS_ACTIVE).await.unwrap();
        let b_token = service.create_session(&b, None, None, None).await.unwrap();
        let a_before = service.list_security_events(&a, 50).await.unwrap().len();
        let b_before = service.list_security_events(&b, 50).await.unwrap().len();

        let revoked = service
            .revoke_session(&a, &hash_token(&b_token))
            .await
            .unwrap();

        assert!(!revoked, "a foreign session must not count as revoked");
        assert!(
            service
                .resolve_credential(&Credential::Cookie(&b_token))
                .await
                .unwrap()
                .is_some(),
            "the other account's session must survive"
        );
        assert_eq!(
            service.list_security_events(&a, 50).await.unwrap().len(),
            a_before
        );
        assert_eq!(
            service.list_security_events(&b, 50).await.unwrap().len(),
            b_before
        );
    }
}
