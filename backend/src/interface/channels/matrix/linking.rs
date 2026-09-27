//! The `!link` command, handled before the sender is resolved.
//!
//! Ordering is the whole reason this is not an ordinary [`BotModule`]. Every
//! other command runs after the sender's MXID has been resolved to an account —
//! provisioning one if it is unknown. Doing that before a link would create an
//! empty account for the very MXID being claimed, and the link would then find
//! it already attached, turning every first-time link into a merge of nothing.
//!
//! So this runs first, on the raw MXID, and never provisions.

use crate::application::accounts::AccountService;
use crate::db::Database;
use crate::domain::accounts::{AccountId, LinkOutcome};
use crate::utils::html::html_escape as escape;

/// Where a completed link should record a merge awaiting confirmation.
pub type PendingMerges = tokio::sync::RwLock<std::collections::HashMap<String, String>>;

/// Handles `!link <code>` and returns the HTML reply.
pub async fn handle_link_command(
    accounts: &AccountService,
    _db: &Database,
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

    match accounts.claim_link(code, sender_mxid, is_direct).await {
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
            notify_linked(accounts, &account_id, sender_mxid).await;
            "Linked. This Matrix account and your web account are now the same \
             account — anything you add here shows up there, and the other way round."
                .to_string()
        }
        Ok(LinkOutcome::NeedsMergeConfirmation {
            account_id,
            from_account,
        }) => {
            let preview = accounts
                .preview_merge(&from_account)
                .await
                .unwrap_or_default();

            if let Some(pending) = pending_merges {
                pending
                    .write()
                    .await
                    .insert(account_id.to_string(), from_account.to_string());
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

async fn notify_linked(accounts: &AccountService, account_id: &AccountId, mxid: &str) {
    accounts
        .record_security_event(
            account_id,
            crate::domain::accounts::SecurityEventKind::MatrixLinked,
            Some(&escape(mxid)),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::events::EventBroadcaster;
    use crate::infrastructure::postgres::repositories::accounts as repo;
    use crate::modules::todo::models::TodoPriority;
    use crate::modules::todo::repository::TodoRepository;

    async fn db() -> (Database, AccountService) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let accounts = AccountService::new(pool.clone(), EventBroadcaster::default());
        (Database { pool }, accounts)
    }

    #[tokio::test]
    async fn test_no_code_explains_where_to_get_one() {
        let (db, accounts) = db().await;
        let reply = handle_link_command(&accounts, &db, "", "@evan:mbm.one", true, None).await;
        assert!(reply.contains("Settings"));
    }

    #[tokio::test]
    async fn test_a_first_time_link_does_not_provision_or_prompt() {
        let (db, accounts) = db().await;
        let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
        let code = accounts.issue_link_challenge(&account).await.unwrap();

        let reply = handle_link_command(&accounts, &db, &code, "@evan:mbm.one", true, None).await;
        assert!(reply.contains("Linked"), "{reply}");

        let count = accounts.count().await.unwrap();
        assert_eq!(count, 1, "no throwaway account may be created for the MXID");

        assert_eq!(
            accounts.find_by_mxid("@evan:mbm.one").await.unwrap(),
            Some(account)
        );
    }

    #[tokio::test]
    async fn test_a_public_room_cancels_the_code_and_says_so() {
        let (db, accounts) = db().await;
        let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
        let code = accounts.issue_link_challenge(&account).await.unwrap();

        let reply = handle_link_command(&accounts, &db, &code, "@evan:mbm.one", false, None).await;
        assert!(reply.contains("direct message"));
        assert!(reply.contains("cancelled"));

        // And the code really is dead.
        let retry = handle_link_command(&accounts, &db, &code, "@evan:mbm.one", true, None).await;
        assert!(retry.contains("not valid"));
    }

    #[tokio::test]
    async fn test_existing_history_asks_before_moving_anything() {
        let (db, accounts) = db().await;
        let old = accounts
            .resolve_or_provision("@evan:mbm.one")
            .await
            .unwrap();
        for text in ["one", "two", "three"] {
            TodoRepository::add(
                &db.pool,
                "!r:x",
                old.as_str(),
                "evan",
                text,
                TodoPriority::Medium,
                None,
                None,
            )
            .await
            .unwrap();
        }

        let new = accounts.create("web", STATUS_ACTIVE).await.unwrap();
        let code = accounts.issue_link_challenge(&new).await.unwrap();

        let pending: PendingMerges = Default::default();
        let reply =
            handle_link_command(&accounts, &db, &code, "@evan:mbm.one", true, Some(&pending)).await;

        assert!(reply.contains('3'), "the count must be shown: {reply}");
        assert!(reply.contains("Nothing has been changed"));

        assert_eq!(
            pending.read().await.get(new.as_str()),
            Some(&old.to_string())
        );

        // The MXID still points at the old account: the link is deliberately
        // left undone until the merge is confirmed.
        assert_eq!(
            accounts.find_by_mxid("@evan:mbm.one").await.unwrap(),
            Some(old)
        );
    }

    #[tokio::test]
    async fn test_an_mxid_belonging_to_someone_else_is_refused() {
        let (db, accounts) = db().await;
        let theirs = accounts.create("theirs", STATUS_ACTIVE).await.unwrap();
        repo::linking::link(&db.pool, &theirs, "@evan:mbm.one")
            .await
            .unwrap();

        let mine = accounts.create("mine", STATUS_ACTIVE).await.unwrap();
        let code = accounts.issue_link_challenge(&mine).await.unwrap();

        let reply = handle_link_command(&accounts, &db, &code, "@evan:mbm.one", true, None).await;
        assert!(reply.contains("already linked"));
    }

    #[tokio::test]
    async fn test_a_successful_link_is_written_to_the_audit_log() {
        let (db, accounts) = db().await;
        let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
        let code = accounts.issue_link_challenge(&account).await.unwrap();

        handle_link_command(&accounts, &db, &code, "@evan:mbm.one", true, None).await;

        let events = accounts.list_security_events(&account, 10).await.unwrap();
        assert_eq!(events[0].kind, "matrix_linked");
    }
}
