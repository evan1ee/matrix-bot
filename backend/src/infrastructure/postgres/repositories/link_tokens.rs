//! Link tokens (`link_tokens`): the single-use, ten-minute proof that someone
//! on a platform asked, in a direct conversation, to connect that identity to
//! an account. Only the SHA-256 of a token is ever stored; the token itself
//! exists in the chat message and the browser URL and nowhere else.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};

use crate::domain::channels::{Channel, PlatformIdentity, PlatformUserId, LINK_TOKEN_TTL};

/// A stored token, minus its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTokenRow {
    pub identity: PlatformIdentity,
    /// Storage-form address of the conversation that asked, for the confirmation.
    pub conversation: String,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

struct Row {
    platform: String,
    platform_user_id: String,
    display_name: Option<String>,
    conversation: String,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

impl TryFrom<Row> for LinkTokenRow {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> Result<Self, Self::Error> {
        let decode =
            |err: crate::domain::channels::ChannelsError| sqlx::Error::Decode(Box::new(err));
        let channel = row.platform.parse::<Channel>().map_err(decode)?;
        let user_id = PlatformUserId::parse(channel, &row.platform_user_id).map_err(decode)?;
        Ok(Self {
            identity: PlatformIdentity::new(channel, user_id, row.display_name),
            conversation: row.conversation,
            expires_at: row.expires_at,
            consumed_at: row.consumed_at,
        })
    }
}

/// Stores a token expiring `LINK_TOKEN_TTL` after `now`.
pub async fn insert(
    conn: &mut PgConnection,
    token_hash: &str,
    identity: &PlatformIdentity,
    conversation: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO link_tokens
            (token_hash, platform, platform_user_id, display_name, conversation, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        token_hash,
        identity.channel().as_str(),
        identity.user_id().as_str(),
        identity.display_name(),
        conversation,
        now,
        now + LINK_TOKEN_TTL,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Burns every unconsumed token of the identity (one live token per identity);
/// returns how many.
pub async fn consume_live_for_identity(
    conn: &mut PgConnection,
    identity: &PlatformIdentity,
    now: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let affected = sqlx::query!(
        "UPDATE link_tokens SET consumed_at = $3
         WHERE platform = $1 AND platform_user_id = $2 AND consumed_at IS NULL",
        identity.channel().as_str(),
        identity.user_id().as_str(),
        now,
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected)
}

pub async fn find(
    conn: impl PgExecutor<'_>,
    token_hash: &str,
) -> Result<Option<LinkTokenRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT platform, platform_user_id, display_name, conversation, expires_at, consumed_at
         FROM link_tokens WHERE token_hash = $1",
        token_hash,
    )
    .fetch_optional(conn)
    .await?;
    row.map(LinkTokenRow::try_from).transpose()
}

/// Like [`find`], but takes the row lock for the rest of the transaction so two
/// claims of one token serialise and only the first can succeed.
pub async fn lock(
    conn: &mut PgConnection,
    token_hash: &str,
) -> Result<Option<LinkTokenRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT platform, platform_user_id, display_name, conversation, expires_at, consumed_at
         FROM link_tokens WHERE token_hash = $1 FOR UPDATE",
        token_hash,
    )
    .fetch_optional(conn)
    .await?;
    row.map(LinkTokenRow::try_from).transpose()
}

pub async fn consume(
    conn: &mut PgConnection,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE link_tokens SET consumed_at = $2 WHERE token_hash = $1",
        token_hash,
        now,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Purges expired or consumed link tokens older than `cutoff`.
pub async fn purge_expired(
    conn: impl PgExecutor<'_>,
    cutoff: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let mut builder =
        sqlx::QueryBuilder::<sqlx::Postgres>::new("DELETE FROM link_tokens WHERE expires_at < ");
    builder.push_bind(cutoff);
    builder.push(" OR (consumed_at IS NOT NULL AND consumed_at < ");
    builder.push_bind(cutoff);
    builder.push(")");
    let res = builder.build().execute(conn).await?;
    Ok(res.rows_affected())
}

/// Test-only: how many tokens exist.
#[cfg(test)]
pub async fn count(ex: impl PgExecutor<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM link_tokens"#)
        .fetch_one(ex)
        .await
}

/// Test-only: every stored token hash.
#[cfg(test)]
pub async fn all_hashes(ex: impl PgExecutor<'_>) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!("SELECT token_hash FROM link_tokens ORDER BY created_at, token_hash")
        .fetch_all(ex)
        .await
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::channels::{Channel, PlatformUserId, LINK_TOKEN_TTL};
    use crate::utils::time::now_at_storage_precision;

    async fn db() -> sqlx::PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    fn tg(raw: &str) -> PlatformIdentity {
        PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, raw).unwrap(),
            Some("Evan".into()),
        )
    }

    #[tokio::test]
    async fn a_token_round_trips_with_its_identity_and_expiry() {
        let pool = db().await;
        let now = now_at_storage_precision();
        let mut conn = pool.acquire().await.unwrap();
        insert(&mut conn, "h1", &tg("42"), "telegram:42", now)
            .await
            .unwrap();

        let row = find(&pool, "h1").await.unwrap().expect("stored");
        assert_eq!(row.identity, tg("42"));
        assert_eq!(row.conversation, "telegram:42");
        assert_eq!(row.expires_at, now + LINK_TOKEN_TTL);
        assert_eq!(row.consumed_at, None);
        assert!(find(&pool, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn consuming_live_tokens_touches_only_that_identity() {
        let pool = db().await;
        let now = now_at_storage_precision();
        let mut conn = pool.acquire().await.unwrap();
        insert(&mut conn, "a", &tg("1"), "telegram:1", now)
            .await
            .unwrap();
        insert(&mut conn, "b", &tg("2"), "telegram:2", now)
            .await
            .unwrap();

        assert_eq!(
            consume_live_for_identity(&mut conn, &tg("1"), now)
                .await
                .unwrap(),
            1
        );
        assert!(find(&pool, "a")
            .await
            .unwrap()
            .unwrap()
            .consumed_at
            .is_some());
        assert!(find(&pool, "b")
            .await
            .unwrap()
            .unwrap()
            .consumed_at
            .is_none());
        assert_eq!(
            consume_live_for_identity(&mut conn, &tg("1"), now)
                .await
                .unwrap(),
            0,
            "an already-consumed token is not consumed twice"
        );
    }

    #[tokio::test]
    async fn lock_and_consume_mark_the_row_used() {
        let pool = db().await;
        let now = now_at_storage_precision();
        let mut tx = pool.begin().await.unwrap();
        insert(
            &mut tx,
            "h",
            &tg("7"),
            "telegram:7",
            now - Duration::minutes(1),
        )
        .await
        .unwrap();
        let locked = lock(&mut tx, "h").await.unwrap().expect("locked");
        assert!(locked.consumed_at.is_none());
        consume(&mut tx, "h", now).await.unwrap();
        assert_eq!(
            lock(&mut tx, "h").await.unwrap().unwrap().consumed_at,
            Some(now)
        );
        assert!(lock(&mut tx, "missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_expired_deletes_tokens_older_than_cutoff() {
        let pool = db().await;
        let now = now_at_storage_precision();
        let mut conn = pool.acquire().await.unwrap();

        let old_created = now - Duration::days(2);
        insert(&mut conn, "old_tok", &tg("10"), "telegram:10", old_created)
            .await
            .unwrap();

        insert(&mut conn, "new_tok", &tg("11"), "telegram:11", now)
            .await
            .unwrap();

        let cutoff = now - Duration::days(1);
        let deleted = purge_expired(&pool, cutoff).await.unwrap();
        assert_eq!(deleted, 1);

        assert!(find(&pool, "old_tok").await.unwrap().is_none());
        assert!(find(&pool, "new_tok").await.unwrap().is_some());
    }
}
