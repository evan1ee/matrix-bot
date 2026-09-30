//! The `!link` command, handled before the sender is resolved.
//!
//! Ordering is the whole reason this is not an ordinary [`BotModule`]. Every
//! other command runs after the sender's MXID has been resolved to an account —
//! provisioning one if it is unknown. Doing that before a link would create an
//! empty account for the very MXID being claimed, and the link would then find
//! it already attached, turning every first-time link into a merge of nothing.
//!
//! So this runs first, on the raw MXID, and never provisions.
//!
//! The `matrix_linked` audit row is written by `AccountService::claim_link`;
//! this handler only words the reply.

use crate::application::accounts::AccountService;
use crate::db::Database;
use crate::domain::accounts::LinkOutcome;

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
        Ok(LinkOutcome::Linked { .. }) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::infrastructure::postgres::repositories::accounts as repo;
    use crate::infrastructure::postgres::repositories::todos as todo_repo;

    async fn db() -> (Database, AccountService) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let accounts = AccountService::new(pool.clone());
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
            todo_repo::create(
                &db.pool,
                &old,
                &NewTodo {
                    room_id: "!r:x".to_string(),
                    sender_name: "evan".to_string(),
                    text: text.to_string(),
                    priority: TodoPriority::Medium,
                    due_at: None,
                    remind_before_minutes: None,
                    remind_room_id: None,
                },
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
            pending.read().await.get(&new.to_string()),
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

    /// The audit row belongs to `AccountService::claim_link`; the `!link`
    /// handler must not write it a second time (it once did, doubling the row).
    #[tokio::test]
    async fn linking_records_matrix_linked_once() {
        let (db, accounts) = db().await;
        let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
        let code = accounts.issue_link_challenge(&account).await.unwrap();

        handle_link_command(&accounts, &db, &code, "@evan:mbm.one", true, None).await;

        let linked: Vec<_> = accounts
            .list_security_events(&account, 10)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "matrix_linked")
            .collect();
        assert_eq!(linked.len(), 1, "exactly one matrix_linked row");
        assert_eq!(linked[0].detail.as_deref(), Some("@evan:mbm.one"));
    }
}
