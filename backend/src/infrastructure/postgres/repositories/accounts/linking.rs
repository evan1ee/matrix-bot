//! Matrix account linking repository.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

use super::core;
use crate::domain::accounts::{normalise_link_code, AccountId, LinkChallenge, LinkOutcome};

const LINK_CODE_TTL_MINUTES: i64 = 15;

struct LinkChallengeRow {
    code: String,
    account_id: String,
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
        account_id.as_str(),
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

    let existing = core::find_by_mxid(pool, mxid).await?;

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
            let previous_account = sqlx::query_scalar!(
                "SELECT account_id FROM security_events \
                 WHERE kind = 'matrix_unlinked' AND detail = $1 \
                 ORDER BY id DESC LIMIT 1",
                mxid,
            )
            .fetch_optional(pool)
            .await?;

            sqlx::query!(
                "INSERT INTO matrix_links (mxid, account_id) VALUES ($1, $2)",
                mxid,
                challenge.account_id.as_str(),
            )
            .execute(pool)
            .await?;

            if let Some(prev) = previous_account {
                let prev_id = AccountId::new(prev);
                if prev_id != challenge.account_id {
                    let preview = crate::accounts::MergeRepository::preview(pool, prev_id.as_str())
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
    let affected = sqlx::query!(
        "DELETE FROM matrix_links WHERE mxid = $1 AND account_id = $2",
        mxid,
        account_id.as_str(),
    )
    .execute(conn)
    .await?
    .rows_affected();

    Ok(affected > 0)
}

pub async fn link(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    mxid: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO matrix_links (mxid, account_id) VALUES ($1, $2)",
        mxid,
        account_id.as_str(),
    )
    .execute(conn)
    .await?;
    Ok(())
}
