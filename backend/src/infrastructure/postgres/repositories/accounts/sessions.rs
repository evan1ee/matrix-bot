//! Sessions repository: cookie sessions, verification timestamps, and revocation.

use chrono::{DateTime, Utc};

use crate::domain::accounts::{hash_token, AccountId, Session};

struct SessionRow {
    id_hash: String,
    account_id: uuid::Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    last_verified_at: DateTime<Utc>,
    restricted_until: Option<DateTime<Utc>>,
    user_agent: Option<String>,
    ip: Option<String>,
    revoked_at: Option<DateTime<Utc>>,
}

impl From<SessionRow> for Session {
    fn from(row: SessionRow) -> Self {
        Session {
            id_hash: row.id_hash,
            account_id: AccountId::from_uuid(row.account_id),
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            last_verified_at: row.last_verified_at,
            restricted_until: row.restricted_until,
            user_agent: row.user_agent,
            ip: row.ip,
            revoked_at: row.revoked_at,
        }
    }
}

pub async fn create(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    user_agent: Option<&str>,
    ip: Option<&str>,
    restricted_until: Option<DateTime<Utc>>,
) -> Result<String, sqlx::Error> {
    let token = crate::utils::random::random_hex(32);
    let now = Utc::now();
    let hashed = hash_token(&token);

    sqlx::query!(
        "INSERT INTO sessions \
         (id_hash, account_id, created_at, last_seen_at, last_verified_at, \
          restricted_until, user_agent, ip) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        hashed,
        account_id.as_uuid(),
        now,
        now,
        now,
        restricted_until,
        user_agent,
        ip,
    )
    .execute(conn)
    .await?;

    Ok(token)
}

pub async fn resolve(pool: &sqlx::PgPool, token: &str) -> Result<Option<Session>, sqlx::Error> {
    let id_hash = hash_token(token);

    let row = sqlx::query_as!(
        SessionRow,
        "SELECT id_hash, account_id, created_at, last_seen_at, last_verified_at, \
                restricted_until, user_agent, ip, revoked_at \
         FROM sessions WHERE id_hash = $1",
        id_hash,
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let session: Session = row.into();
    let now = Utc::now();
    if !session.is_live(now) {
        return Ok(None);
    }

    sqlx::query!(
        "UPDATE sessions SET last_seen_at = $1 WHERE id_hash = $2",
        now,
        id_hash,
    )
    .execute(pool)
    .await?;

    Ok(Some(Session {
        last_seen_at: now,
        ..session
    }))
}

pub async fn mark_verified(
    conn: impl sqlx::PgExecutor<'_>,
    id_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE sessions SET last_verified_at = $1 WHERE id_hash = $2",
        Utc::now(),
        id_hash,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn revoke(conn: impl sqlx::PgExecutor<'_>, id_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE sessions SET revoked_at = $1 WHERE id_hash = $2 AND revoked_at IS NULL",
        Utc::now(),
        id_hash,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn revoke_all_for_account(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    except: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let affected = sqlx::query!(
        "UPDATE sessions SET revoked_at = $1 \
         WHERE account_id = $2 AND revoked_at IS NULL AND id_hash IS DISTINCT FROM $3",
        Utc::now(),
        account_id.as_uuid(),
        except,
    )
    .execute(conn)
    .await?
    .rows_affected();

    Ok(affected)
}

pub async fn list_for_account(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Vec<Session>, sqlx::Error> {
    let rows = sqlx::query_as!(
        SessionRow,
        "SELECT id_hash, account_id, created_at, last_seen_at, last_verified_at, \
                restricted_until, user_agent, ip, revoked_at \
         FROM sessions WHERE account_id = $1 AND revoked_at IS NULL \
         ORDER BY last_seen_at DESC",
        account_id.as_uuid(),
    )
    .fetch_all(conn)
    .await?;

    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(Session::from)
        .filter(|s| s.is_live(now))
        .collect())
}

pub async fn update_last_verified_at(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    verified_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE sessions SET last_verified_at = $1 WHERE account_id = $2",
        verified_at,
        account_id.as_uuid(),
    )
    .execute(conn)
    .await?;
    Ok(())
}
