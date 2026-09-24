//! The `!link` command, handled before the sender is resolved.
//!
//! Ordering is the whole reason this is not an ordinary [`BotModule`]. Every
//! other command runs after the sender's MXID has been resolved to an account —
//! provisioning one if it is unknown. Doing that before a link would create an
//! empty account for the very MXID being claimed, and the link would then find
//! it already attached, turning every first-time link into a merge of nothing.
//!
//! So this runs first, on the raw MXID, and never provisions.

use crate::accounts::{LinkOutcome, LinkRepository, MergeRepository};
use crate::db::Database;
use crate::utils::html::html_escape as escape;

/// Where a completed link should record a merge awaiting confirmation.
pub type PendingMerges = tokio::sync::RwLock<std::collections::HashMap<String, String>>;

/// Handles `!link <code>` and returns the HTML reply.
pub async fn handle_link_command(
    db: &Database,
    args: &str,
    sender_mxid: &str,
    is_direct: bool,
    pending_merges: Option<&PendingMerges>,
) -> String {
    let code = args.trim();
    if code.is_empty() {
        return "Usage: <code>!link &lt;code&gt;</code><br/>\
                Get a code from the web app under Settings → Security, then send it \
                here in a direct message."
            .to_string();
    }

    match LinkRepository::claim(&db.pool, code, sender_mxid, is_direct).await {
        Ok(LinkOutcome::NotADirectMessage) => {
            "That code was visible to everyone in this room, so I have cancelled it.<br/>\
             Generate a new one and send it to me in a <b>direct message</b>."
                .to_string()
        }
        Ok(LinkOutcome::UnknownCode) => {
            "That code is not valid — it may have expired, or already been used.<br/>\
             Generate a fresh one in the web app."
                .to_string()
        }
        Ok(LinkOutcome::AlreadyLinked) => {
            "This Matrix account is already linked to a different web account.<br/>\
             Unlink it there first."
                .to_string()
        }
        Ok(LinkOutcome::Linked { account_id }) => {
            notify_linked(db, &account_id, sender_mxid).await;
            "Linked. This Matrix account and your web account are now the same \
             account — anything you add here shows up there, and the other way round."
                .to_string()
        }
        Ok(LinkOutcome::NeedsMergeConfirmation {
            account_id,
            from_account,
        }) => {
            let preview = MergeRepository::preview(&db.pool, &from_account)
                .await
                .unwrap_or_default();

            if let Some(pending) = pending_merges {
                pending
                    .write()
                    .await
                    .insert(account_id.clone(), from_account.clone());
            }

            // The link itself is left undone until the merge is confirmed:
            // attaching the MXID now would leave two accounts both reachable
            // from it, which is exactly the ambiguity the merge resolves.
            format!(
                "This Matrix account already has <b>{}</b> item{} saved against it.<br/><br/>\
                 Open the web app under Settings → Security to review and confirm \
                 moving them into your account. Nothing has been changed yet.",
                preview.total(),
                if preview.total() == 1 { "" } else { "s" }
            )
        }
        Err(err) => {
            tracing::error!("Could not process !link from {}: {:?}", sender_mxid, err);
            "Something went wrong linking that account. Try again shortly.".to_string()
        }
    }
}

async fn notify_linked(db: &Database, account_id: &str, mxid: &str) {
    crate::accounts::SecurityEventRepository::record(
        &db.pool,
        account_id,
        crate::accounts::SecurityEventKind::MatrixLinked,
        Some(&escape(mxid)),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};

    async fn db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        Database { pool: pool }
    }

    #[tokio::test]
    async fn test_no_code_explains_where_to_get_one() {
        let db = db().await;
        let reply = handle_link_command(&db, "", "@evan:mbm.one", true, None).await;
        assert!(reply.contains("Settings"));
    }

    /// The ordering guarantee: a first-time link from an MXID the bot has never
    /// seen must simply link, with no account provisioned for it along the way
    /// and no merge prompt for data that does not exist.
    #[tokio::test]
    async fn test_a_first_time_link_does_not_provision_or_prompt() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&db.pool, &account).await.unwrap();

        let reply = handle_link_command(&db, &code, "@evan:mbm.one", true, None).await;
        assert!(reply.contains("Linked"), "{reply}");

        let accounts: i64 =
            crate::db_scalar_one!(&db.pool, _, "SELECT COUNT(*) FROM accounts").unwrap();
        assert_eq!(
            accounts, 1,
            "no throwaway account may be created for the MXID"
        );

        assert_eq!(
            AccountRepository::find_by_mxid(&db.pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(account)
        );
    }

    #[tokio::test]
    async fn test_a_public_room_cancels_the_code_and_says_so() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&db.pool, &account).await.unwrap();

        let reply = handle_link_command(&db, &code, "@evan:mbm.one", false, None).await;
        assert!(reply.contains("direct message"));
        assert!(reply.contains("cancelled"));

        // And the code really is dead.
        let retry = handle_link_command(&db, &code, "@evan:mbm.one", true, None).await;
        assert!(retry.contains("not valid"));
    }

    /// Existing history prompts rather than merging silently, and nothing is
    /// changed until the user has seen the number and agreed to it.
    #[tokio::test]
    async fn test_existing_history_asks_before_moving_anything() {
        let db = db().await;
        let old = AccountRepository::resolve_or_provision(&db.pool, "@evan:mbm.one")
            .await
            .unwrap();
        for text in ["one", "two", "three"] {
            crate::db_execute!(
                &db.pool,
                "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
                 VALUES ('!r:x', ?, 'evan', ?, 'medium', FALSE)",
                &old,
                text
            )
            .unwrap();
        }

        let new = AccountRepository::create(&db.pool, "web", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&db.pool, &new).await.unwrap();

        let pending: PendingMerges = Default::default();
        let reply = handle_link_command(&db, &code, "@evan:mbm.one", true, Some(&pending)).await;

        assert!(reply.contains('3'), "the count must be shown: {reply}");
        assert!(reply.contains("Nothing has been changed"));

        assert_eq!(pending.read().await.get(&new), Some(&old));

        // The MXID still points at the old account: the link is deliberately
        // left undone until the merge is confirmed.
        assert_eq!(
            AccountRepository::find_by_mxid(&db.pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(old)
        );
    }

    #[tokio::test]
    async fn test_an_mxid_belonging_to_someone_else_is_refused() {
        let db = db().await;
        let theirs = AccountRepository::create(&db.pool, "theirs", STATUS_ACTIVE)
            .await
            .unwrap();
        crate::db_execute!(
            &db.pool,
            "INSERT INTO matrix_links (mxid, account_id) VALUES ('@evan:mbm.one', ?)",
            &theirs
        )
        .unwrap();

        let mine = AccountRepository::create(&db.pool, "mine", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&db.pool, &mine).await.unwrap();

        let reply = handle_link_command(&db, &code, "@evan:mbm.one", true, None).await;
        assert!(reply.contains("already linked"));
    }

    #[tokio::test]
    async fn test_a_successful_link_is_written_to_the_audit_log() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&db.pool, &account).await.unwrap();

        handle_link_command(&db, &code, "@evan:mbm.one", true, None).await;

        let events = crate::accounts::SecurityEventRepository::list(&db.pool, &account, 10)
            .await
            .unwrap();
        assert_eq!(events[0].kind, "matrix_linked");
    }
}
