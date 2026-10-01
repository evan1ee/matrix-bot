//! Matrix account linking repository.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

use super::core;
use crate::domain::accounts::{
    normalise_link_code, AccountId, LinkChallenge, LinkOutcome, LINK_CODE_TTL_MINUTES,
};
use crate::domain::channels::{Channel, PlatformIdentity};
use crate::infrastructure::postgres::repositories::identities;

/// The Matrix identity for `mxid`, or a decode error for a malformed id (a
/// real homeserver sender is always well-formed; this only guards the type).
fn matrix_identity(mxid: &str) -> Result<PlatformIdentity, sqlx::Error> {
    PlatformIdentity::matrix(mxid).map_err(|err| {
        tracing::warn!(error = %err, "refusing a malformed Matrix user id");
        sqlx::Error::Decode(Box::new(err))
    })
}

struct LinkChallengeRow {
    code: String,
    account_id: uuid::Uuid,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

impl From<LinkChallengeRow> for LinkChallenge {
    fn from(row: LinkChallengeRow) -> Self {
        LinkChallenge {
            code: row.code,
            account_id: AccountId::new(row.account_id),
            expires_at: row.expires_at,
            consumed_at: row.consumed_at,
        }
    }
}

pub async fn issue(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<String, sqlx::Error> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

    let bytes: Vec<u8> = crate::utils::random::random_bytes(8);

    let raw: String = bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    let code = format!("{}-{}", &raw[..4], &raw[4..8]);

    let now = Utc::now();
    let expires = now + Duration::minutes(LINK_CODE_TTL_MINUTES);

    sqlx::query!(
        "INSERT INTO matrix_link_challenges (code, account_id, created_at, expires_at) \
         VALUES ($1, $2, $3, $4)",
        code,
        account_id.as_uuid(),
        now,
        expires,
    )
    .execute(conn)
    .await?;

    Ok(code)
}

pub async fn consume(pool: &PgPool, code: &str) -> Result<Option<LinkChallenge>, sqlx::Error> {
    let normalised = normalise_link_code(code);
    let mut tx = pool.begin().await?;

    let challenge = sqlx::query_as!(
        LinkChallengeRow,
        "SELECT code, account_id, expires_at, consumed_at \
         FROM matrix_link_challenges WHERE code = $1",
        normalised,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(challenge) = challenge else {
        return Ok(None);
    };

    if challenge.consumed_at.is_some() || challenge.expires_at < Utc::now() {
        return Ok(None);
    }

    sqlx::query!(
        "UPDATE matrix_link_challenges SET consumed_at = $1 WHERE code = $2",
        Utc::now(),
        normalised,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Some(challenge.into()))
}

pub async fn claim(
    pool: &PgPool,
    code: &str,
    mxid: &str,
    is_direct: bool,
) -> Result<LinkOutcome, sqlx::Error> {
    if !is_direct {
        let _ = consume(pool, code).await;
        return Ok(LinkOutcome::NotADirectMessage);
    }

    let Some(challenge) = consume(pool, code).await? else {
        return Ok(LinkOutcome::UnknownCode);
    };

    let identity = matrix_identity(mxid)?;
    let existing = identities::find_account(pool, &identity).await?;

    match existing {
        Some(ref current) if *current == challenge.account_id => Ok(LinkOutcome::Linked {
            account_id: challenge.account_id,
        }),
        Some(current) => {
            let account = core::find(pool, &current).await?;

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
            let previous_account: Option<uuid::Uuid> = sqlx::query_scalar!(
                r#"SELECT account_id AS "account_id: uuid::Uuid" FROM security_events
                 WHERE (kind = 'matrix_unlinked' AND detail = $1)
                    OR (kind = 'identity_unlinked' AND detail = $2)
                 ORDER BY id DESC LIMIT 1"#,
                mxid,
                format!("matrix:{mxid}"),
            )
            .fetch_optional(pool)
            .await?;

            if !identities::bind(pool, &challenge.account_id, &identity).await? {
                // Someone bound this MXID between the lookup and the insert.
                return Ok(LinkOutcome::AlreadyLinked);
            }

            if let Some(prev) = previous_account {
                let prev_id = AccountId::new(prev);
                if prev_id != challenge.account_id {
                    let preview = crate::infrastructure::postgres::merge::preview(pool, &prev_id)
                        .await
                        .unwrap_or_default();
                    if preview.total() > 0 {
                        return Ok(LinkOutcome::NeedsMergeConfirmation {
                            account_id: challenge.account_id,
                            from_account: prev_id,
                        });
                    }
                }
            }

            Ok(LinkOutcome::Linked {
                account_id: challenge.account_id,
            })
        }
    }
}

pub async fn unlink(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    mxid: &str,
) -> Result<bool, sqlx::Error> {
    // A malformed id cannot be bound, so there is nothing to remove.
    let Ok(identity) = PlatformIdentity::matrix(mxid) else {
        tracing::warn!("ignoring unlink of a malformed Matrix user id");
        return Ok(false);
    };
    identities::unbind(
        conn,
        account_id,
        Channel::Matrix,
        identity.user_id().as_str(),
    )
    .await
}

pub async fn link(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    mxid: &str,
) -> Result<(), sqlx::Error> {
    let identity = matrix_identity(mxid)?;
    if identities::bind(conn, account_id, &identity).await? {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(
            "Matrix user id is already linked".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;

    async fn find_by_mxid(pool: &PgPool, mxid: &str) -> Result<Option<AccountId>, sqlx::Error> {
        identities::find_account(pool, &PlatformIdentity::matrix(mxid).unwrap()).await
    }

    async fn resolve_or_provision(pool: &PgPool, mxid: &str) -> Result<AccountId, sqlx::Error> {
        identities::resolve_or_provision(pool, &PlatformIdentity::matrix(mxid).unwrap()).await
    }

    async fn db() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_codes_are_readable_and_unique() {
        let pool = db().await;
        let account = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();

        let a = issue(&pool, &account).await.unwrap();
        let b = issue(&pool, &account).await.unwrap();

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
        let account = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &account).await.unwrap();

        let sloppy = code.to_lowercase().replace('-', " ");
        let outcome = claim(&pool, &sloppy, "@evan:mbm.one", true).await.unwrap();

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
        let account = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &account).await.unwrap();

        let outcome = claim(&pool, &code, "@evan:mbm.one", true).await.unwrap();
        assert_eq!(
            outcome,
            LinkOutcome::Linked {
                account_id: account
            }
        );

        assert_eq!(
            find_by_mxid(&pool, "@evan:mbm.one").await.unwrap(),
            Some(account)
        );
    }

    /// A code sent in a public room is visible to everyone in it, so a
    /// bystander could send it from their own MXID and take the account. The
    /// bot refuses, and burns the code so the race cannot be run at all.
    #[tokio::test]
    async fn test_a_code_shown_in_a_public_room_is_refused_and_burned() {
        let pool = db().await;
        let account = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &account).await.unwrap();

        let outcome = claim(&pool, &code, "@evan:mbm.one", false).await.unwrap();
        assert_eq!(outcome, LinkOutcome::NotADirectMessage);

        // The bystander's attempt finds nothing left to use.
        let stolen = claim(&pool, &code, "@bystander:mbm.one", true)
            .await
            .unwrap();
        assert_eq!(stolen, LinkOutcome::UnknownCode);

        assert!(find_by_mxid(&pool, "@bystander:mbm.one")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_a_code_is_single_use() {
        let pool = db().await;
        let account = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &account).await.unwrap();

        assert!(matches!(
            claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::Linked { .. }
        ));
        assert_eq!(
            claim(&pool, &code, "@someone:else.org", true)
                .await
                .unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    #[tokio::test]
    async fn test_an_expired_code_is_refused() {
        let pool = db().await;
        let account = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &account).await.unwrap();

        sqlx::query!(
            "UPDATE matrix_link_challenges SET expires_at = $1 WHERE code = $2",
            Utc::now() - Duration::minutes(1),
            &code
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::UnknownCode
        );
    }

    #[tokio::test]
    async fn test_an_unknown_code_links_nothing() {
        let pool = db().await;
        assert_eq!(
            claim(&pool, "ZZZZ-ZZZZ", "@evan:mbm.one", true)
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
        let theirs = core::create(&pool, "theirs", STATUS_ACTIVE).await.unwrap();
        sqlx::query!(
            "INSERT INTO platform_identities (account_id, platform, platform_user_id) \
             VALUES ($1, 'matrix', '@evan:mbm.one')",
            theirs.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let mine = core::create(&pool, "mine", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &mine).await.unwrap();

        assert_eq!(
            claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::AlreadyLinked
        );

        // The link is untouched.
        assert_eq!(
            find_by_mxid(&pool, "@evan:mbm.one").await.unwrap(),
            Some(theirs)
        );
    }

    /// The common case for anyone who used the bot before the web UI existed:
    /// the MXID already has an unclaimed account full of history, so linking
    /// asks for confirmation rather than silently merging or refusing.
    #[tokio::test]
    async fn test_an_unclaimed_account_with_history_asks_to_merge() {
        let pool = db().await;
        let old = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let new = core::create(&pool, "web", STATUS_ACTIVE).await.unwrap();
        let code = issue(&pool, &new).await.unwrap();

        assert_eq!(
            claim(&pool, &code, "@evan:mbm.one", true).await.unwrap(),
            LinkOutcome::NeedsMergeConfirmation {
                account_id: new,
                from_account: old
            }
        );
    }

    /// A3: the "previously unlinked from an account with data" prompt must honour
    /// both the historic `matrix_unlinked` rows (detail `<mxid>`) and the new
    /// `identity_unlinked` rows (detail `matrix:<mxid>`).
    #[tokio::test]
    async fn test_a_previous_unlink_asks_to_merge_under_either_audit_kind() {
        use crate::domain::accounts::SecurityEventKind;
        use crate::infrastructure::postgres::repositories::accounts::security_events;

        for (kind, detail) in [
            (SecurityEventKind::MatrixUnlinked, "@evan:mbm.one"),
            (SecurityEventKind::IdentityUnlinked, "matrix:@evan:mbm.one"),
        ] {
            let pool = db().await;
            let mxid = format!("@evan{}:mbm.one", kind.as_str());
            let detail = detail.replace("@evan:mbm.one", &mxid);
            let old = core::create(&pool, "old", STATUS_ACTIVE).await.unwrap();
            sqlx::query!(
                "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
                 VALUES ('!r:x', $1, 'evan', 'history', 'medium', FALSE)",
                old.as_uuid()
            )
            .execute(&pool)
            .await
            .unwrap();
            security_events::record(&pool, &old, kind, Some(&detail)).await;

            let new = core::create(&pool, "web", STATUS_ACTIVE).await.unwrap();
            let code = issue(&pool, &new).await.unwrap();
            assert_eq!(
                claim(&pool, &code, &mxid, true).await.unwrap(),
                LinkOutcome::NeedsMergeConfirmation {
                    account_id: new,
                    from_account: old
                },
                "{} rows must still trigger the merge prompt",
                kind.as_str()
            );
        }
    }

    /// Unlinking revokes an access path. The rows stay put.
    #[tokio::test]
    async fn test_unlinking_keeps_the_data() {
        let pool = db().await;
        let account = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();

        sqlx::query!(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('!r:x', $1, 'evan', 'keep me', 'medium', FALSE)",
            account.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(unlink(&pool, &account, "@evan:mbm.one").await.unwrap());

        // The MXID reaches nothing now.
        assert!(find_by_mxid(&pool, "@evan:mbm.one")
            .await
            .unwrap()
            .is_none());

        // The row is exactly where it was.
        let owner: uuid::Uuid =
            sqlx::query_scalar!("SELECT owner_id FROM todos WHERE text = 'keep me'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(AccountId::from_uuid(owner), account);
    }

    /// A later message from an unlinked MXID starts a fresh, empty account
    /// rather than finding its way back into the one it left.
    #[tokio::test]
    async fn test_an_unlinked_mxid_comes_back_as_a_stranger() {
        let pool = db().await;
        let original = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        unlink(&pool, &original, "@evan:mbm.one").await.unwrap();

        let fresh = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
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
        let theirs = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let mine = core::create(&pool, "mine", STATUS_ACTIVE).await.unwrap();

        assert!(!unlink(&pool, &mine, "@evan:mbm.one").await.unwrap());
        assert_eq!(
            find_by_mxid(&pool, "@evan:mbm.one").await.unwrap(),
            Some(theirs)
        );
    }
}
