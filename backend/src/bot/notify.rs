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

use crate::accounts::{AccountRepository, SecurityEventKind, SecurityEventRepository};
use crate::db::Database;

/// Records an event and announces it to every linked Matrix identity.
pub async fn notify_account(
    client: Option<&Client>,
    db: &Database,
    account_id: &str,
    kind: SecurityEventKind,
    detail: Option<&str>,
) {
    SecurityEventRepository::record(db.sqlite_pool(), account_id, kind, detail).await;

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

    let mxids = match AccountRepository::linked_mxids(db.sqlite_pool(), account_id).await {
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
    use crate::accounts::{AccountRepository, SecurityEventRepository, STATUS_ACTIVE};
    use crate::db::schema::run_migrations;
    use sqlx::SqlitePool;

    async fn db() -> Database {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&crate::db::DbPool::Sqlite(pool.clone()))
            .await
            .unwrap();
        Database {
            pool: crate::db::DbPool::Sqlite(pool),
        }
    }

    /// With no Matrix client the event must still be recorded. An account with
    /// no linked identity would otherwise have no record of anything at all.
    #[tokio::test]
    async fn test_the_event_is_logged_even_with_nowhere_to_send_it() {
        let db = db().await;
        let account = AccountRepository::create(db.sqlite_pool(), "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        notify_account(
            None,
            &db,
            &account,
            SecurityEventKind::RecoveryRequested,
            Some("from @evan:mbm.one"),
        )
        .await;

        let events = SecurityEventRepository::list(db.sqlite_pool(), &account, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "recovery_requested");
        assert_eq!(events[0].detail.as_deref(), Some("from @evan:mbm.one"));
    }

    /// Notifying an account that has since been deleted must not panic — the
    /// row is gone, the log line is orphaned, and the caller carries on.
    #[tokio::test]
    async fn test_notifying_a_missing_account_is_harmless() {
        let db = db().await;
        notify_account(
            None,
            &db,
            "acct_gone",
            SecurityEventKind::PasskeyRemoved,
            None,
        )
        .await;
    }
}
