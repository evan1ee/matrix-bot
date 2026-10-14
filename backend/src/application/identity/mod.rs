//! Identity use cases: who a platform user is, across every channel.
//!
//! An adapter hands over the `PlatformIdentity` it authenticated from the
//! platform (never a model- or client-supplied owner) and gets back the
//! account it speaks as, under that channel's provisioning policy
//! (spec §7.2, invariants 1-2): Matrix auto-provisions an unknown sender,
//! every newer channel must be linked from an existing account first.

mod link;
pub use link::{LinkOutcome, LinkPreview, LinkService, LinkStart};

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::{AccountId, SecurityEventKind};
use crate::domain::channels::{
    provisioning_policy, Channel, LinkedIdentity, PlatformIdentity, PlatformUserId,
    ProvisioningPolicy,
};
use crate::infrastructure::postgres::repositories::accounts::{core, credentials, security_events};
use crate::infrastructure::postgres::repositories::identities;

/// The outcome of resolving a sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Account(AccountId),
    /// Nobody has linked this identity and its channel does not auto-provision.
    Unlinked,
}

#[derive(Clone)]
pub struct IdentityService {
    pool: PgPool,
}

impl IdentityService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Applies `provisioning_policy(identity.channel())`; refreshes a changed
    /// display name of an identity that is already bound.
    pub async fn resolve(&self, identity: &PlatformIdentity) -> Result<Resolution, AppError> {
        match provisioning_policy(identity.channel()) {
            ProvisioningPolicy::ProvisionUnknown => {
                let account = identities::resolve_or_provision(&self.pool, identity).await?;
                self.refresh_name(identity).await;
                Ok(Resolution::Account(account))
            }
            ProvisioningPolicy::RequireLink => {
                match identities::find_account(&self.pool, identity).await? {
                    Some(account) => {
                        self.refresh_name(identity).await;
                        Ok(Resolution::Account(account))
                    }
                    None => Ok(Resolution::Unlinked),
                }
            }
        }
    }

    /// A stale display name is cosmetic: it must never fail the message that
    /// carried it.
    async fn refresh_name(&self, identity: &PlatformIdentity) {
        if let Err(err) = identities::touch_display_name(&self.pool, identity).await {
            tracing::warn!(
                channel = %identity.channel(),
                error = %err,
                "could not refresh an identity's display name"
            );
        }
    }

    pub async fn find_account(
        &self,
        identity: &PlatformIdentity,
    ) -> Result<Option<AccountId>, AppError> {
        Ok(identities::find_account(&self.pool, identity).await?)
    }

    pub async fn user_ids(
        &self,
        account: &AccountId,
        channel: Channel,
    ) -> Result<Vec<String>, AppError> {
        Ok(identities::user_ids(&self.pool, account, channel).await?)
    }

    pub async fn list(&self, account: &AccountId) -> Result<Vec<LinkedIdentity>, AppError> {
        Ok(identities::list(&self.pool, account).await?)
    }

    /// Removes an identity from `account` and records `identity_unlinked`
    /// (detail `<platform>:<user_id>`) in the same transaction; `Ok(false)`
    /// when the account did not hold it (nothing is recorded). Refuses to
    /// remove the last identity of an account with no passkey: that would
    /// leave no way in. The account row is locked so two concurrent unlinks
    /// cannot each see the other's identity still present.
    pub async fn unlink(
        &self,
        account: &AccountId,
        channel: Channel,
        user_id: &str,
    ) -> Result<bool, AppError> {
        // A malformed id cannot be bound, so there is nothing to remove.
        let Ok(user_id) = PlatformUserId::parse(channel, user_id) else {
            tracing::warn!(%channel, "ignoring unlink of a malformed user id");
            return Ok(false);
        };

        let mut tx = self.pool.begin().await?;
        if !core::lock(&mut *tx, account).await? {
            return Ok(false);
        }
        if !identities::unbind(&mut *tx, account, channel, user_id.as_str()).await? {
            return Ok(false);
        }
        let remaining = identities::list(&mut *tx, account).await?.len();
        let passkeys = credentials::count_for_account(&mut *tx, account).await?;
        if remaining == 0 && passkeys == 0 {
            // Dropping the transaction rolls the unbind back.
            return Err(AppError::Conflict(
                "That is your only way to sign in. Add a passkey or link another account first."
                    .to_string(),
            ));
        }
        let detail = format!("{channel}:{}", user_id.as_str());
        security_events::record(
            &mut *tx,
            account,
            SecurityEventKind::IdentityUnlinked,
            Some(&detail),
        )
        .await;
        tx.commit().await?;
        Ok(true)
    }

    #[cfg(test)]
    pub async fn count_accounts_for_test(&self) -> i64 {
        core::count(&self.pool).await.unwrap()
    }

    /// Test convenience: the account a Matrix user speaks as, provisioned on
    /// first sight (what the Matrix adapter does for every sender).
    #[cfg(test)]
    pub async fn provision_matrix_for_test(&self, mxid: &str) -> AccountId {
        match self.resolve(&PlatformIdentity::matrix(mxid).unwrap()).await {
            Ok(Resolution::Account(account)) => account,
            other => panic!("Matrix users are always provisioned: {other:?}"),
        }
    }

    #[cfg(test)]
    pub async fn find_matrix_for_test(&self, mxid: &str) -> Option<AccountId> {
        self.find_account(&PlatformIdentity::matrix(mxid).unwrap())
            .await
            .unwrap()
    }

    #[cfg(test)]
    pub async fn bind_for_test(&self, account: &AccountId, identity: &PlatformIdentity) {
        identities::bind(&self.pool, account, identity)
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::PlatformUserId;
    use crate::infrastructure::postgres::repositories::accounts::{
        core, credentials, security_events,
    };
    use crate::infrastructure::postgres::repositories::identities;

    async fn service_with_pool() -> (IdentityService, PgPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        (IdentityService::new(pool.clone()), pool)
    }

    async fn service() -> IdentityService {
        service_with_pool().await.0
    }

    fn ident(channel: Channel, raw: &str) -> PlatformIdentity {
        PlatformIdentity::new(channel, PlatformUserId::parse(channel, raw).unwrap(), None)
    }
    fn mx(raw: &str) -> PlatformIdentity {
        ident(Channel::Matrix, raw)
    }
    fn tg(raw: &str) -> PlatformIdentity {
        ident(Channel::Telegram, raw)
    }
    fn dc(raw: &str) -> PlatformIdentity {
        ident(Channel::Discord, raw)
    }
    fn em(raw: &str) -> PlatformIdentity {
        ident(Channel::Email, raw)
    }

    #[tokio::test]
    async fn an_unknown_matrix_user_is_provisioned() {
        let s = service().await;
        let r = s.resolve(&mx("@new:mbm.one")).await.unwrap();
        assert!(matches!(r, Resolution::Account(_)));
    }

    #[tokio::test]
    async fn an_unknown_telegram_user_is_not_provisioned() {
        let s = service().await;
        let before = s.count_accounts_for_test().await;
        assert_eq!(
            s.resolve(&tg("987654321")).await.unwrap(),
            Resolution::Unlinked
        );
        assert_eq!(
            s.count_accounts_for_test().await,
            before,
            "no account may be created"
        );
    }

    #[tokio::test]
    async fn a_bound_identity_on_any_channel_resolves_to_its_account() {
        let (s, pool) = service_with_pool().await;
        let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
        for id in [
            dc("123456789012345678"),
            tg("987654321"),
            em("evan@example.com"),
        ] {
            identities::bind(&pool, &a, &id).await.unwrap();
            assert_eq!(s.resolve(&id).await.unwrap(), Resolution::Account(a));
        }
    }

    #[tokio::test]
    async fn resolving_refreshes_a_changed_display_name() {
        let (s, pool) = service_with_pool().await;
        let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
        identities::bind(&pool, &a, &tg("5")).await.unwrap();
        let named = PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, "5").unwrap(),
            Some("Evan".into()),
        );
        s.resolve(&named).await.unwrap();
        let listed = s.list(&a).await.unwrap();
        assert_eq!(listed[0].display_name.as_deref(), Some("Evan"));
    }

    #[tokio::test]
    async fn unlinking_the_last_way_in_to_a_passkeyless_account_is_refused() {
        let (s, pool) = service_with_pool().await;
        let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        assert!(matches!(
            s.unlink(&a, Channel::Telegram, "1").await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(s.user_ids(&a, Channel::Telegram).await.unwrap(), vec!["1"]);
        assert!(security_events::list(&pool, &a, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn the_last_identity_may_go_when_a_passkey_remains() {
        let (s, pool) = service_with_pool().await;
        let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        credentials::add(&pool, &a, "cred-1", "{}", "key")
            .await
            .unwrap();
        assert!(s.unlink(&a, Channel::Telegram, "1").await.unwrap());
    }

    #[tokio::test]
    async fn unlinking_records_a_security_event() {
        let (s, pool) = service_with_pool().await;
        let a = core::create(&pool, "a", STATUS_ACTIVE).await.unwrap();
        identities::bind(&pool, &a, &mx("@evan:mbm.one"))
            .await
            .unwrap();
        identities::bind(&pool, &a, &tg("1")).await.unwrap();

        let stranger = core::create(&pool, "s", STATUS_ACTIVE).await.unwrap();
        assert!(!s
            .unlink(&stranger, Channel::Matrix, "@evan:mbm.one")
            .await
            .unwrap());
        assert!(security_events::list(&pool, &stranger, 10)
            .await
            .unwrap()
            .is_empty());

        assert!(s
            .unlink(&a, Channel::Matrix, "@evan:mbm.one")
            .await
            .unwrap());
        assert!(!s
            .unlink(&a, Channel::Matrix, "@evan:mbm.one")
            .await
            .unwrap());

        let events = security_events::list(&pool, &a, 10).await.unwrap();
        assert_eq!(
            events.len(),
            1,
            "one row, only because something was removed"
        );
        assert_eq!(events[0].kind, "identity_unlinked");
        assert_eq!(events[0].detail.as_deref(), Some("matrix:@evan:mbm.one"));
    }
}
