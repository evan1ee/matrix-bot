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

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

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
    NeedsMergeConfirmation { account_id: String, from_account: String },
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
    ///
    /// Formatted in two short groups because a human retypes this into a chat
    /// window: `A3F9-K2QX`. Drawn from an alphabet with no `0/O` or `1/I/L`, so
    /// a misread character cannot silently become a different valid code.
    pub async fn issue(pool: &SqlitePool, account_id: &str) -> Result<String, sqlx::Error> {
        const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

        let bytes: Vec<u8> = sqlx::query_scalar::<_, Vec<u8>>("SELECT randomblob(8)")
            .fetch_one(pool)
            .await?;

        let raw: String = bytes
            .iter()
            .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
            .collect();
        let code = format!("{}-{}", &raw[..4], &raw[4..8]);

        let now = Utc::now();
        sqlx::query(
            "INSERT INTO matrix_link_challenges (code, account_id, created_at, expires_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&code)
        .bind(account_id)
        .bind(now)
        .bind(now + Duration::minutes(LINK_CODE_TTL_MINUTES))
        .execute(pool)
        .await?;

        Ok(code)
    }

    /// Consumes a code, if it is live.
    ///
    /// Consumption is unconditional on success: single-use means a code that
    /// has been *seen* is spent, whatever happens next.
    pub async fn consume(
        pool: &SqlitePool,
        code: &str,
    ) -> Result<Option<LinkChallenge>, sqlx::Error> {
        let normalised = normalise_code(code);

        let challenge = sqlx::query_as::<_, LinkChallenge>(
            "SELECT code, account_id, expires_at, consumed_at \
             FROM matrix_link_challenges WHERE code = ?",
        )
        .bind(&normalised)
        .fetch_optional(pool)
        .await?;

        let Some(challenge) = challenge else {
            return Ok(None);
        };

        if challenge.consumed_at.is_some() || challenge.expires_at < Utc::now() {
            return Ok(None);
        }

        sqlx::query("UPDATE matrix_link_challenges SET consumed_at = ? WHERE code = ?")
            .bind(Utc::now())
            .bind(&normalised)
            .execute(pool)
            .await?;

        Ok(Some(challenge))
    }

    /// Handles `!link <code>` arriving from `mxid`.
    ///
    /// `is_direct` is passed in rather than inferred here so the Matrix-specific
    /// room lookup stays in the bot layer and this stays testable without a
    /// homeserver.
    pub async fn claim(
        pool: &SqlitePool,
        code: &str,
        mxid: &str,
        is_direct: bool,
    ) -> Result<LinkOutcome, sqlx::Error> {
        if !is_direct {
            // Burn the code anyway: it has been shown to a room.
            let _ = Self::consume(pool, code).await;
            return Ok(LinkOutcome::NotADirectMessage);
        }

        let Some(challenge) = Self::consume(pool, code).await? else {
            return Ok(LinkOutcome::UnknownCode);
        };

        let existing = super::AccountRepository::find_by_mxid(pool, mxid).await?;

        match existing {
            // Already this account's — nothing to do.
            Some(ref current) if *current == challenge.account_id => {
                Ok(LinkOutcome::Linked { account_id: challenge.account_id })
            }
            Some(current) => {
                let account = super::AccountRepository::find(pool, &current).await?;

                // An MXID whose account someone has claimed belongs to that
                // person. Only an *unclaimed* account — one auto-provisioned
                // for this MXID and holding no credential — can be absorbed.
                match account {
                    Some(account) if account.is_unclaimed() => {
                        Ok(LinkOutcome::NeedsMergeConfirmation {
                            account_id: challenge.account_id,
                            from_account: current,
                        })
                    }
                    _ => Ok(LinkOutcome::AlreadyLinked),
                }
            }
            None => {
                sqlx::query("INSERT INTO matrix_links (mxid, account_id) VALUES (?, ?)")
                    .bind(mxid)
                    .bind(&challenge.account_id)
                    .execute(pool)
                    .await?;
                Ok(LinkOutcome::Linked { account_id: challenge.account_id })
            }
        }
    }

    /// Removes a link, leaving every row it created exactly where it is.
    ///
    /// Data belongs to the account; unlinking revokes an access path and
    /// nothing else. A later command from that MXID provisions a fresh empty
    /// account, so the identity goes cold rather than inheriting anything.
    pub async fn unlink(
        pool: &SqlitePool,
        account_id: &str,
        mxid: &str,
    ) -> Result<bool, sqlx::Error> {
        let removed = sqlx::query("DELETE FROM matrix_links WHERE mxid = ? AND account_id = ?")
            .bind(mxid)
            .bind(account_id)
            .execute(pool)
            .await?
            .rows_affected();

        Ok(removed > 0)
    }
}

/// Accepts the code however a human typed it — lower case, spaces, missing
/// dash. Only the characters carry meaning.
fn normalise_code(code: &str) -> String {
    let cleaned: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();

    if cleaned.len() == 8 {
        format!("{}-{}", &cleaned[..4], &cleaned[4..])
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use crate::db::schema::run_migrations;

    async fn db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_codes_are_readable_and_unique() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();

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
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        let sloppy = code.to_lowercase().replace('-', " ");
        let outcome = LinkRepository::claim(&pool, &sloppy, "@evan:mbm.one", true).await.unwrap();

        assert_eq!(outcome, LinkOutcome::Linked { account_id: account });
    }

    /// Proof of ownership is the sender, so the happy path is simply: the code
    /// arrives from the MXID being claimed.
    #[tokio::test]
    async fn test_a_fresh_mxid_links_straight_away() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        let outcome = LinkRepository::claim(&pool, &code, "@evan:mbm.one", true).await.unwrap();
        assert_eq!(outcome, LinkOutcome::Linked { account_id: account.clone() });

        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one").await.unwrap(),
            Some(account)
        );
    }

    /// A code sent in a public room is visible to everyone in it, so a
    /// bystander could send it from their own MXID and take the account. The
    /// bot refuses, and burns the code so the race cannot be run at all.
    #[tokio::test]
    async fn test_a_code_shown_in_a_public_room_is_refused_and_burned() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        let outcome = LinkRepository::claim(&pool, &code, "@evan:mbm.one", false).await.unwrap();
        assert_eq!(outcome, LinkOutcome::NotADirectMessage);

        // The bystander's attempt finds nothing left to use.
        let stolen = LinkRepository::claim(&pool, &code, "@bystander:mbm.one", true).await.unwrap();
        assert_eq!(stolen, LinkOutcome::UnknownCode);

        assert!(AccountRepository::find_by_mxid(&pool, "@bystander:mbm.one").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_a_code_is_single_use() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        assert!(matches!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::Linked { .. }
        ));
        assert_eq!(
            LinkRepository::claim(&pool, &code, "@someone:else.org", true).await.unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    #[tokio::test]
    async fn test_an_expired_code_is_refused() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &account).await.unwrap();

        sqlx::query("UPDATE matrix_link_challenges SET expires_at = ? WHERE code = ?")
            .bind(Utc::now() - Duration::minutes(1))
            .bind(&code)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    #[tokio::test]
    async fn test_an_unknown_code_links_nothing() {
        let pool = db().await;
        assert_eq!(
            LinkRepository::claim(&pool, "ZZZZ-ZZZZ", "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    /// An MXID already reaching a *claimed* account belongs to that person.
    /// Linking it elsewhere would hand someone else's data over.
    #[tokio::test]
    async fn test_an_mxid_on_a_claimed_account_cannot_be_taken() {
        let pool = db().await;
        let theirs = AccountRepository::create(&pool, "theirs", STATUS_ACTIVE).await.unwrap();
        sqlx::query("INSERT INTO matrix_links (mxid, account_id) VALUES ('@evan:mbm.one', ?)")
            .bind(&theirs)
            .execute(&pool)
            .await
            .unwrap();

        let mine = AccountRepository::create(&pool, "mine", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &mine).await.unwrap();

        assert_eq!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::AlreadyLinked
        );

        // The link is untouched.
        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one").await.unwrap(),
            Some(theirs)
        );
    }

    /// The common case for anyone who used the bot before the web UI existed:
    /// the MXID already has an unclaimed account full of history, so linking
    /// asks for confirmation rather than silently merging or refusing.
    #[tokio::test]
    async fn test_an_unclaimed_account_with_history_asks_to_merge() {
        let pool = db().await;
        let old = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let new = AccountRepository::create(&pool, "web", STATUS_ACTIVE).await.unwrap();
        let code = LinkRepository::issue(&pool, &new).await.unwrap();

        assert_eq!(
            LinkRepository::claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::NeedsMergeConfirmation { account_id: new, from_account: old }
        );
    }

    /// Unlinking revokes an access path. The rows stay put.
    #[tokio::test]
    async fn test_unlinking_keeps_the_data() {
        let pool = db().await;
        let account = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();

        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('!r:x', ?, 'evan', 'keep me', 'medium', 0)",
        )
        .bind(&account)
        .execute(&pool)
        .await
        .unwrap();

        assert!(LinkRepository::unlink(&pool, &account, "@evan:mbm.one").await.unwrap());

        // The MXID reaches nothing now.
        assert!(AccountRepository::find_by_mxid(&pool, "@evan:mbm.one").await.unwrap().is_none());

        // The row is exactly where it was.
        let owner: String = sqlx::query_scalar("SELECT owner_id FROM todos WHERE text = 'keep me'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(owner, account);
    }

    /// A later message from an unlinked MXID starts a fresh, empty account
    /// rather than finding its way back into the one it left.
    #[tokio::test]
    async fn test_an_unlinked_mxid_comes_back_as_a_stranger() {
        let pool = db().await;
        let original = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        LinkRepository::unlink(&pool, &original, "@evan:mbm.one").await.unwrap();

        let fresh = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        assert_ne!(fresh, original, "the MXID must not inherit the account it was unlinked from");
    }

    /// Unlinking is scoped to the account doing it: knowing an MXID must not
    /// let one account sever another's link.
    #[tokio::test]
    async fn test_one_account_cannot_unlink_anothers_mxid() {
        let pool = db().await;
        let theirs = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let mine = AccountRepository::create(&pool, "mine", STATUS_ACTIVE).await.unwrap();

        assert!(!LinkRepository::unlink(&pool, &mine, "@evan:mbm.one").await.unwrap());
        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one").await.unwrap(),
            Some(theirs)
        );
    }
}
