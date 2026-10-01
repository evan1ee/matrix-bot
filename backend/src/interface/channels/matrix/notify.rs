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

use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::UserId;
use matrix_sdk::Client;
use tracing::{error, warn};

use crate::application::identity::IdentityService;
use crate::domain::accounts::{AccountId, SecurityEventKind};
use crate::domain::channels::Channel;

/// Announces an event to every linked Matrix identity.
///
/// Writes no audit row: `AccountService` or `IdentityService` records the event exactly once, in the
/// operation that caused it, and a second row here would double it.
pub async fn announce_account(
    client: Option<&Client>,
    identity: &IdentityService,
    account_id: &AccountId,
    kind: SecurityEventKind,
    detail: Option<&str>,
) {
    if !kind.warrants_a_dm() {
        return;
    }

    let Some(client) = client else {
        warn!(
            "No Matrix client available; {} for {} was logged but not announced",
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
        send_dm(client, &mxid, &body).await;
    }
}

/// Sends one line to a user's DM with the bot, opening one if none exists.
///
/// A user who has never messaged the bot has no room to receive this in, and a
/// security warning is exactly the message that must not be dropped for want of
/// a room — so one is created.
pub async fn send_dm(client: &Client, mxid: &str, body: &str) {
    let Ok(user_id) = UserId::parse(mxid) else {
        error!("Cannot notify {}: not a valid user ID", mxid);
        return;
    };

    let content = RoomMessageEventContent::text_plain(body);

    // An existing DM first: creating a second one every time would litter the
    // user's client with near-identical rooms.
    for room in client.joined_rooms() {
        if !room.is_direct().await.unwrap_or(false) {
            continue;
        }
        if !matches!(room.get_member(&user_id).await, Ok(Some(_))) {
            continue;
        }
        if let Err(err) = room.send(content.clone()).await {
            error!("Could not send a notification to {}: {:?}", mxid, err);
        }
        return;
    }

    match client.create_dm(&user_id).await {
        Ok(room) => {
            if let Err(err) = room.send(content).await {
                error!(
                    "Could not send a notification to the new DM with {}: {:?}",
                    mxid, err
                );
            }
        }
        Err(err) => error!(
            "Could not open a DM with {} to notify them: {:?}",
            mxid, err
        ),
    }
}

/// Sends an HTML formatted message to a user's DM with the bot.
pub async fn send_dm_html(client: &Client, mxid: &str, plain: &str, html: &str) {
    let Ok(user_id) = UserId::parse(mxid) else {
        error!("Cannot notify {}: not a valid user ID", mxid);
        return;
    };

    let content = RoomMessageEventContent::text_html(plain, html);

    for room in client.joined_rooms() {
        if !room.is_direct().await.unwrap_or(false) {
            continue;
        }
        if !matches!(room.get_member(&user_id).await, Ok(Some(_))) {
            continue;
        }
        if let Err(err) = room.send(content.clone()).await {
            error!("Could not send an HTML notification to {}: {:?}", mxid, err);
        }
        return;
    }

    match client.create_dm(&user_id).await {
        Ok(room) => {
            if let Err(err) = room.send(content).await {
                error!(
                    "Could not send an HTML notification to the new DM with {}: {:?}",
                    mxid, err
                );
            }
        }
        Err(err) => error!(
            "Could not open a DM with {} to notify them: {:?}",
            mxid, err
        ),
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
}
