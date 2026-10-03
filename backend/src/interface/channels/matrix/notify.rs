//! Telling someone their account changed.
//!
//! Every sensitive action records a [`SecurityEvent`] and, unless it is routine,
//! DMs each linked Matrix identity. The two cover each other: an account with no
//! Matrix link still has an in-app log, and a user who never opens the web UI
//! still hears about it.
//!
//! The DM is the load-bearing half for one case in particular. Matrix recovery
//! is announced the moment it is requested and then does nothing for 24 hours,
//! and that message is the only warning the real owner gets before someone else
//! takes their account. It has to arrive.
//!
//! Nothing here can fail the action it describes. By the time a notification is
//! sent, the thing has already happened; refusing it afterwards would be worse
//! than a message that did not arrive.

use std::sync::Weak;

use tracing::{error, warn};

use crate::application::identity::IdentityService;
use crate::domain::accounts::{AccountId, SecurityEventKind};
use crate::domain::channels::{Channel, PlatformIdentity};
use crate::interface::channels::message::{Content, OutgoingMessage, Target, Visibility};
use crate::interface::channels::registry::ChannelRegistry;

/// Announces an event to every linked Matrix identity, as one direct message
/// each, through the channel registry.
///
/// Writes no audit row: `AccountService` or `IdentityService` records the event exactly once, in the
/// operation that caused it, and a second row here would double it.
///
/// `outbound` is `None` where no registry exists (unit tests, the web recovery
/// redemption) and a dead `Weak` during shutdown; either way the event is
/// logged but not announced, the same as a missing Matrix client always was.
pub async fn announce_account(
    outbound: Option<&Weak<ChannelRegistry>>,
    identity: &IdentityService,
    account_id: &AccountId,
    kind: SecurityEventKind,
    detail: Option<&str>,
) {
    if !kind.warrants_a_dm() {
        return;
    }

    let Some(registry) = outbound.and_then(Weak::upgrade) else {
        warn!(
            "No channel registry available; {} for {} was logged but not announced",
            kind.as_str(),
            account_id
        );
        return;
    };

    let mxids = match identity.user_ids(account_id, Channel::Matrix).await {
        Ok(mxids) => mxids,
        Err(err) => {
            error!(
                "Could not read linked identities for {}: {:?}",
                account_id, err
            );
            return;
        }
    };

    let body = kind.message(detail);
    for mxid in mxids {
        let recipient = match PlatformIdentity::matrix(&mxid) {
            Ok(recipient) => recipient,
            Err(err) => {
                error!("Cannot notify {}: {}", mxid, err);
                continue;
            }
        };
        let message = OutgoingMessage {
            target: Target::Direct(recipient),
            content: Content::text(body.clone()),
            reply_to: None,
            visibility: Visibility::Private,
            idempotency_key: None,
            join_if_needed: true,
        };
        if let Err(err) = registry.send(message).await {
            error!("Could not send a notification to {}: {}", mxid, err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppServices;
    use crate::domain::accounts::STATUS_ACTIVE;

    async fn services() -> AppServices {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        AppServices::new(&pool)
    }

    /// The audit row is `AccountService`'s to write, once. Announcing with
    /// nowhere to send it must therefore leave the log exactly as it found it,
    /// or every announcement would double the row the service already wrote.
    #[tokio::test]
    async fn test_announcing_writes_no_audit_row_of_its_own() {
        let services = services().await;
        let acct_id = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        announce_account(
            None,
            &services.identity,
            &acct_id,
            SecurityEventKind::RecoveryRequested,
            Some("from @evan:mbm.one"),
        )
        .await;

        let events = services
            .accounts
            .list_security_events(&acct_id, 10)
            .await
            .unwrap();
        assert!(events.is_empty(), "announcing must not record: {events:?}");
    }

    /// Announcing an account that has since been deleted must not panic — the
    /// row is gone, the log line is orphaned, and the caller carries on.
    #[tokio::test]
    async fn test_notifying_a_missing_account_is_harmless() {
        let services = services().await;
        let acct_id = AccountId::from_seed("acct_gone");
        announce_account(
            None,
            &services.identity,
            &acct_id,
            SecurityEventKind::PasskeyRemoved,
            None,
        )
        .await;
    }
    /// The announcement goes out as one direct message per linked MXID,
    /// through the registry, with the text `SecurityEventKind` words.
    #[tokio::test]
    async fn announcing_sends_a_direct_message_to_each_linked_mxid_through_the_registry() {
        use crate::interface::channels::fake::FakeAdapter;
        use crate::interface::channels::message::Target;
        use crate::interface::channels::registry::ChannelRegistry;
        use std::sync::Arc;

        let services = services().await;
        let acct_id = services
            .identity
            .provision_matrix_for_test("@evan:mbm.one")
            .await;
        let adapter = Arc::new(FakeAdapter::connected(Channel::Matrix));
        let registry = Arc::new(ChannelRegistry::new(vec![adapter.clone()]).unwrap());
        let weak = Arc::downgrade(&registry);

        announce_account(
            Some(&weak),
            &services.identity,
            &acct_id,
            SecurityEventKind::RecoveryRequested,
            Some("from @evan:mbm.one"),
        )
        .await;

        let sent = adapter.sent();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            &sent[0].target,
            Target::Direct(identity) if identity.user_id().as_str() == "@evan:mbm.one"
        ));
        assert_eq!(
            sent[0].content.plain,
            SecurityEventKind::RecoveryRequested.message(Some("from @evan:mbm.one"))
        );
        assert!(
            sent[0].content.html.is_none(),
            "a plain-text notice stays plain"
        );
    }

    /// A registry that is gone (shutdown) or absent (unit tests) skips quietly.
    #[tokio::test]
    async fn announcing_without_a_live_registry_is_harmless() {
        use crate::interface::channels::registry::ChannelRegistry;
        use std::sync::{Arc, Weak};

        let services = services().await;
        let acct_id = services
            .identity
            .provision_matrix_for_test("@evan:mbm.one")
            .await;
        let gone: Weak<ChannelRegistry> =
            Arc::downgrade(&Arc::new(ChannelRegistry::new(Vec::new()).unwrap()));
        announce_account(
            Some(&gone),
            &services.identity,
            &acct_id,
            SecurityEventKind::PasskeyRemoved,
            None,
        )
        .await;
    }
}
