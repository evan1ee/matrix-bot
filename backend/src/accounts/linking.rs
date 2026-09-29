//! Claiming a Matrix account, and letting one go.
//!
//! The web UI issues a one-time code and the user sends it to the bot **from
//! the Matrix account they are claiming**. Proof of ownership is therefore the
//! message sender, which cannot be forged — no password is typed anywhere, and
//! the bot never has to DM a stranger to start the flow.
//!
//! The bot refuses `!link` outside a direct message. In a public room the code
//! is visible to everyone present, and a bystander could send it from *their*
//! MXID and attach their account instead. Seeing the code in a room is enough
//! to burn it.

use crate::db::DbPool;
use chrono::{DateTime, Utc};

/// How long a link code stays usable.
///
/// Long enough to switch to a Matrix client and paste it, short enough that a
/// code left on screen is not a standing invitation.
pub const LINK_CODE_TTL_MINUTES: i64 = 5;

/// Why a `!link` attempt did not result in a link.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    /// Linked, and nothing had to be merged.
    Linked { account_id: String },
    /// The MXID's existing unclaimed account has data. The user is asked to
    /// confirm the merge in the web UI, where the counts can be shown.
    NeedsMergeConfirmation {
        account_id: String,
        from_account: String,
    },
    /// No such code, or it expired, or it has already been used.
    UnknownCode,
    /// This MXID already reaches a different account. One Matrix account maps
    /// to one local account; unlink first.
    AlreadyLinked,
    /// The command arrived somewhere other than a direct message.
    NotADirectMessage,
}

/// A code the user has been given but not yet sent.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LinkChallenge {
    pub code: String,
    pub account_id: String,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

pub struct LinkRepository;

impl LinkRepository {
    /// Issues a code for `account_id`.
    pub async fn issue(pool: &DbPool, account_id: &str) -> Result<String, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::linking::issue(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// Consumes a code, if it is live.
    pub async fn consume(pool: &DbPool, code: &str) -> Result<Option<LinkChallenge>, sqlx::Error> {
        let res =
            crate::infrastructure::postgres::repositories::accounts::linking::consume(pool, code)
                .await?;
        Ok(res.map(|c| LinkChallenge {
            code: c.code,
            account_id: c.account_id.to_string(),
            expires_at: c.expires_at,
            consumed_at: c.consumed_at,
        }))
    }

    /// Handles `!link <code>` arriving from `mxid`.
    pub async fn claim(
        pool: &DbPool,
        code: &str,
        mxid: &str,
        is_direct: bool,
    ) -> Result<LinkOutcome, sqlx::Error> {
        let outcome = crate::infrastructure::postgres::repositories::accounts::linking::claim(
            pool, code, mxid, is_direct,
        )
        .await?;
        Ok(match outcome {
            crate::domain::accounts::LinkOutcome::Linked { account_id } => LinkOutcome::Linked {
                account_id: account_id.to_string(),
            },
            crate::domain::accounts::LinkOutcome::NeedsMergeConfirmation {
                account_id,
                from_account,
            } => LinkOutcome::NeedsMergeConfirmation {
                account_id: account_id.to_string(),
                from_account: from_account.to_string(),
            },
            crate::domain::accounts::LinkOutcome::UnknownCode => LinkOutcome::UnknownCode,
            crate::domain::accounts::LinkOutcome::AlreadyLinked => LinkOutcome::AlreadyLinked,
            crate::domain::accounts::LinkOutcome::NotADirectMessage => {
                LinkOutcome::NotADirectMessage
            }
        })
    }

    /// Removes a link, leaving every row it created exactly where it is.
    pub async fn unlink(pool: &DbPool, account_id: &str, mxid: &str) -> Result<bool, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::linking::unlink(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
            mxid,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use chrono::Duration;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_codes_are_readable_and_unique() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        let a = LinkRepository::issue(&pool, &account).await.unwrap();
        let b = LinkRepository::issue(&pool, &account).await.unwrap();

        assert_ne!(a, b);
        assert_eq!(a.len(), 9, "four, dash, four");
        assert_eq!(a.chars().nth(4), Some('-'));
        assert!(
            !a.contains(['0', 'O', '1', 'I', 'L']),
            "the alphabet must exclude characters that are misread: {a}"
        );
    }

    #[tokio::test]
    async fn test_a_code_is_accepted_however_it_is_typed() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        let sloppy = code.to_lowercase().replace('-', " ");
        let outcome = LinkRepository::claim(&pool, &sloppy, "@evan:mbm.one", true)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            LinkOutcome::Linked {
                account_id: account
            }
        );
    }

    /// Proof of ownership is the sender, so the happy path is simply: the code
    /// arrives from the MXID being claimed.
    #[tokio::test]
    async fn test_a_fresh_mxid_links_straight_away() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        let outcome = LinkRepository::claim(&pool, &code, "@evan:mbm.one", true)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            LinkOutcome::Linked {
                account_id: account.clone()
            }
        );

        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(account)
        );
    }

    /// A code sent in a public room is visible to everyone in it, so a
    /// bystander could send it from their own MXID and take the account. The
    /// bot refuses, and burns the code so the race cannot be run at all.
    #[tokio::test]
    async fn test_a_code_shown_in_a_public_room_is_refused_and_burned() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        let outcome = LinkRepository::claim(&pool, &code, "@evan:mbm.one", false)
            .await
            .unwrap();
        assert_eq!(outcome, LinkOutcome::NotADirectMessage);

        // The bystander's attempt finds nothing left to use.
        let stolen = LinkRepository::claim(&pool, &code, "@bystander:mbm.one", true)
            .await
            .unwrap();
        assert_eq!(stolen, LinkOutcome::UnknownCode);

        assert!(AccountRepository::find_by_mxid(&pool, "@bystander:mbm.one")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_a_code_is_single_use() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        assert!(matches!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true)
                .await
                .unwrap(),
            LinkOutcome::Linked { .. }
        ));
        assert_eq!(
            LinkRepository::claim(&pool, &code, "@someone:else.org", true)
                .await
                .unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    #[tokio::test]
    async fn test_an_expired_code_is_refused() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        sqlx::query!(
            "UPDATE matrix_link_challenges SET expires_at = $1 WHERE code = $2",
            Utc::now() - Duration::minutes(1),
            &code
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true)
                .await
                .unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    #[tokio::test]
    async fn test_an_unknown_code_links_nothing() {
        let pool = db().await;
        assert_eq!(
            LinkRepository::claim(&pool, "ZZZZ-ZZZZ", "@evan:mbm.one", true)
                .await
                .unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    /// An MXID already reaching a *claimed* account belongs to that person.
    /// Linking it elsewhere would hand someone else's data over.
    #[tokio::test]
    async fn test_an_mxid_on_a_claimed_account_cannot_be_taken() {
        let pool = db().await;
        let theirs = AccountRepository::create(&pool, "theirs", STATUS_ACTIVE)
            .await
            .unwrap();
        let theirs_id = crate::domain::accounts::AccountId::from(theirs.as_str());
        sqlx::query!(
            "INSERT INTO matrix_links (mxid, account_id) VALUES ('@evan:mbm.one', $1)",
            theirs_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let mine = AccountRepository::create(&pool, "mine", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &mine).await.unwrap();

        assert_eq!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true)
                .await
                .unwrap(),
            LinkOutcome::AlreadyLinked
        );

        // The link is untouched.
        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(theirs)
        );
    }

    /// The common case for anyone who used the bot before the web UI existed:
    /// the MXID already has an unclaimed account full of history, so linking
    /// asks for confirmation rather than silently merging or refusing.
    #[tokio::test]
    async fn test_an_unclaimed_account_with_history_asks_to_merge() {
        let pool = db().await;
        let old = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let new = AccountRepository::create(&pool, "web", STATUS_ACTIVE)
            .await
            .unwrap();
        let code = LinkRepository::issue(&pool, &new).await.unwrap();

        assert_eq!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true)
                .await
                .unwrap(),
            LinkOutcome::NeedsMergeConfirmation {
                account_id: new,
                from_account: old
            }
        );
    }

    /// Unlinking revokes an access path. The rows stay put.
    #[tokio::test]
    async fn test_unlinking_keeps_the_data() {
        let pool = db().await;
        let account = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();

        let acct_id = crate::domain::accounts::AccountId::from(account.as_str());
        sqlx::query!(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('!r:x', $1, 'evan', 'keep me', 'medium', FALSE)",
            acct_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(LinkRepository::unlink(&pool, &account, "@evan:mbm.one")
            .await
            .unwrap());

        // The MXID reaches nothing now.
        assert!(AccountRepository::find_by_mxid(&pool, "@evan:mbm.one")
            .await
            .unwrap()
            .is_none());

        // The row is exactly where it was.
        let owner: uuid::Uuid =
            sqlx::query_scalar!("SELECT owner_id FROM todos WHERE text = 'keep me'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            crate::domain::accounts::AccountId::from_uuid(owner).to_string(),
            account
        );
    }

    /// A later message from an unlinked MXID starts a fresh, empty account
    /// rather than finding its way back into the one it left.
    #[tokio::test]
    async fn test_an_unlinked_mxid_comes_back_as_a_stranger() {
        let pool = db().await;
        let original = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        LinkRepository::unlink(&pool, &original, "@evan:mbm.one")
            .await
            .unwrap();

        let fresh = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        assert_ne!(
            fresh, original,
            "the MXID must not inherit the account it was unlinked from"
        );
    }

    /// Unlinking is scoped to the account doing it: knowing an MXID must not
    /// let one account sever another's link.
    #[tokio::test]
    async fn test_one_account_cannot_unlink_anothers_mxid() {
        let pool = db().await;
        let theirs = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let mine = AccountRepository::create(&pool, "mine", STATUS_ACTIVE)
            .await
            .unwrap();

        assert!(!LinkRepository::unlink(&pool, &mine, "@evan:mbm.one")
            .await
            .unwrap());
        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(theirs)
        );
    }
}
