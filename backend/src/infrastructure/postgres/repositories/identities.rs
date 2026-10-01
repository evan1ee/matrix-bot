//! Platform identities: which account a person is on each channel
//! (`platform_identities`). One `(platform, platform_user_id)` belongs to at
//! most one account; the schema's unique key is the guarantee, the functions
//! here report a lost race as `Ok(false)` rather than an error.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::{display_name_for_mxid, AccountId, STATUS_UNCLAIMED};
use crate::domain::channels::{Channel, LinkedIdentity, PlatformIdentity};
use crate::infrastructure::postgres::repositories::accounts::core;

struct IdentityRow {
    platform: String,
    platform_user_id: String,
    display_name: Option<String>,
    created_at: DateTime<Utc>,
}

impl TryFrom<IdentityRow> for LinkedIdentity {
    type Error = sqlx::Error;

    fn try_from(row: IdentityRow) -> Result<Self, Self::Error> {
        let channel = row
            .platform
            .parse::<Channel>()
            .map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
        Ok(LinkedIdentity::new(
            channel,
            row.platform_user_id,
            row.display_name,
            row.created_at,
        ))
    }
}

/// The account an identity is bound to, if any.
pub async fn find_account(
    conn: impl sqlx::PgExecutor<'_>,
    identity: &PlatformIdentity,
) -> Result<Option<AccountId>, sqlx::Error> {
    let row = sqlx::query_scalar!(
        r#"SELECT account_id AS "account_id: uuid::Uuid" FROM platform_identities
           WHERE platform = $1 AND platform_user_id = $2"#,
        identity.channel().as_str(),
        identity.user_id().as_str(),
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(AccountId::new))
}

/// Every user id the account holds on `channel`, oldest binding first.
pub async fn user_ids(
    conn: impl sqlx::PgExecutor<'_>,
    account: &AccountId,
    channel: Channel,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT platform_user_id FROM platform_identities
         WHERE account_id = $1 AND platform = $2 ORDER BY created_at, id",
        account.as_uuid(),
        channel.as_str(),
    )
    .fetch_all(conn)
    .await
}

/// Every identity bound to the account, oldest first.
pub async fn list(
    conn: impl sqlx::PgExecutor<'_>,
    account: &AccountId,
) -> Result<Vec<LinkedIdentity>, sqlx::Error> {
    let rows = sqlx::query_as!(
        IdentityRow,
        "SELECT platform, platform_user_id, display_name, created_at FROM platform_identities
         WHERE account_id = $1 ORDER BY created_at, id",
        account.as_uuid(),
    )
    .fetch_all(conn)
    .await?;
    rows.into_iter().map(LinkedIdentity::try_from).collect()
}

/// Binds an identity to an account. `Ok(false)` when the identity is already
/// bound (to any account): a bind never steals.
pub async fn bind(
    conn: impl sqlx::PgExecutor<'_>,
    account: &AccountId,
    identity: &PlatformIdentity,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query!(
        "INSERT INTO platform_identities (account_id, platform, platform_user_id, display_name)
         VALUES ($1, $2, $3, $4) ON CONFLICT (platform, platform_user_id) DO NOTHING",
        account.as_uuid(),
        identity.channel().as_str(),
        identity.user_id().as_str(),
        identity.display_name(),
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Moves an already-bound identity to another account. `Ok(false)` when the
/// identity is not bound at all. The caller decides whether the move is
/// allowed (only an unclaimed account's identity may be taken over).
pub async fn rebind(
    conn: impl sqlx::PgExecutor<'_>,
    to: &AccountId,
    identity: &PlatformIdentity,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query!(
        "UPDATE platform_identities SET account_id = $1, updated_at = now()
         WHERE platform = $2 AND platform_user_id = $3",
        to.as_uuid(),
        identity.channel().as_str(),
        identity.user_id().as_str(),
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Removes an identity from `account`; `Ok(false)` if the account did not hold it.
pub async fn unbind(
    conn: impl sqlx::PgExecutor<'_>,
    account: &AccountId,
    channel: Channel,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query!(
        "DELETE FROM platform_identities
         WHERE account_id = $1 AND platform = $2 AND platform_user_id = $3",
        account.as_uuid(),
        channel.as_str(),
        user_id,
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Records the name the platform currently shows for the identity. A no-op
/// without a name or when nothing changed (so a chatty sender costs no write).
pub async fn touch_display_name(
    conn: impl sqlx::PgExecutor<'_>,
    identity: &PlatformIdentity,
) -> Result<(), sqlx::Error> {
    let Some(name) = identity.display_name() else {
        return Ok(());
    };
    sqlx::query!(
        "UPDATE platform_identities SET display_name = $3, updated_at = now()
         WHERE platform = $1 AND platform_user_id = $2 AND display_name IS DISTINCT FROM $3",
        identity.channel().as_str(),
        identity.user_id().as_str(),
        name,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The account an identity speaks as; an unknown identity gets a fresh
/// unclaimed account, created and bound in one transaction so a lost race
/// leaves no orphan account behind. Concurrent first messages converge on the
/// winner's account.
pub async fn resolve_or_provision(
    pool: &PgPool,
    identity: &PlatformIdentity,
) -> Result<AccountId, sqlx::Error> {
    if let Some(existing) = find_account(pool, identity).await? {
        return Ok(existing);
    }

    let mut tx = pool.begin().await?;
    let account_id = core::create(&mut *tx, &provisioned_name(identity), STATUS_UNCLAIMED).await?;
    if bind(&mut *tx, &account_id, identity).await? {
        tx.commit().await?;
        return Ok(account_id);
    }
    // Lost the race: the winner's row is committed by the time our conflicting
    // insert returned, so dropping our account and reading theirs is safe.
    tx.rollback().await?;
    find_account(pool, identity)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// What an auto-provisioned account is called: the platform's own display
/// name, else the Matrix localpart, else the raw user id.
fn provisioned_name(identity: &PlatformIdentity) -> String {
    match (identity.display_name(), identity.channel()) {
        (Some(name), _) if !name.trim().is_empty() => name.to_owned(),
        (_, Channel::Matrix) => display_name_for_mxid(identity.user_id().as_str()),
        _ => identity.user_id().as_str().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::PlatformUserId;

    async fn pool() -> PgPool {
        let p = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&p).await;
        p
    }

    fn identity(channel: Channel, raw: &str) -> PlatformIdentity {
        PlatformIdentity::new(channel, PlatformUserId::parse(channel, raw).unwrap(), None)
    }
    fn mx(raw: &str) -> PlatformIdentity {
        identity(Channel::Matrix, raw)
    }
    fn dc(raw: &str) -> PlatformIdentity {
        identity(Channel::Discord, raw)
    }
    fn tg(raw: &str) -> PlatformIdentity {
        identity(Channel::Telegram, raw)
    }
    fn em(raw: &str) -> PlatformIdentity {
        identity(Channel::Email, raw)
    }

    #[tokio::test]
    async fn a_platform_user_id_belongs_to_at_most_one_account() {
        let p = pool().await;
        let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
        let b = core::create(&p, "b", STATUS_ACTIVE).await.unwrap();
        let id = tg("987654321");
        assert!(bind(&p, &a, &id).await.unwrap());
        assert!(
            !bind(&p, &b, &id).await.unwrap(),
            "second bind must not steal the identity"
        );
        assert_eq!(find_account(&p, &id).await.unwrap(), Some(a));
    }

    #[tokio::test]
    async fn every_channel_resolves_to_the_same_account_once_bound() {
        let p = pool().await;
        let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
        for id in [
            mx("@evan:mbm.one"),
            dc("123456789012345678"),
            tg("987654321"),
            em("evan@example.com"),
        ] {
            bind(&p, &a, &id).await.unwrap();
            assert_eq!(find_account(&p, &id).await.unwrap(), Some(a));
        }
        assert_eq!(list(&p, &a).await.unwrap().len(), 4);
        assert_eq!(
            user_ids(&p, &a, Channel::Matrix).await.unwrap(),
            vec!["@evan:mbm.one"]
        );
    }

    #[tokio::test]
    async fn identities_are_isolated_per_account() {
        let p = pool().await;
        let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
        let b = core::create(&p, "b", STATUS_ACTIVE).await.unwrap();
        bind(&p, &a, &tg("1")).await.unwrap();
        assert!(list(&p, &b).await.unwrap().is_empty());
        assert!(
            !unbind(&p, &b, Channel::Telegram, "1").await.unwrap(),
            "b cannot unbind a's identity"
        );
    }

    #[tokio::test]
    async fn resolve_or_provision_is_idempotent_and_creates_an_unclaimed_account() {
        let p = pool().await;
        let id = mx("@new:mbm.one");
        let first = resolve_or_provision(&p, &id).await.unwrap();
        let second = resolve_or_provision(&p, &id).await.unwrap();
        assert_eq!(first, second);
        assert!(core::find(&p, &first)
            .await
            .unwrap()
            .unwrap()
            .is_unclaimed());
    }

    /// Concurrent first messages from a brand-new sender converge on one
    /// account and strand no orphan (moved here from `accounts/core.rs`).
    #[tokio::test]
    async fn concurrent_first_messages_converge_on_one_account() {
        let p = pool().await;
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let p = p.clone();
                tokio::spawn(
                    async move { resolve_or_provision(&p, &mx("@racer:x")).await.unwrap() },
                )
            })
            .collect();
        let mut ids = Vec::new();
        for handle in handles {
            ids.push(handle.await.unwrap());
        }
        assert!(
            ids.windows(2).all(|w| w[0] == w[1]),
            "resolved to different accounts: {ids:?}"
        );
        assert_eq!(core::count(&p).await.unwrap(), 1, "a loser left an orphan");
    }

    #[tokio::test]
    async fn provisioned_matrix_account_is_named_after_the_localpart() {
        let p = pool().await;
        let id = resolve_or_provision(&p, &mx("@evan:mbm.one"))
            .await
            .unwrap();
        assert_eq!(
            core::find(&p, &id).await.unwrap().unwrap().display_name,
            "evan"
        );
    }

    #[tokio::test]
    async fn rebind_moves_a_bound_identity_and_touch_updates_the_name() {
        let p = pool().await;
        let a = core::create(&p, "a", STATUS_ACTIVE).await.unwrap();
        let b = core::create(&p, "b", STATUS_ACTIVE).await.unwrap();
        let id = tg("42");
        assert!(!rebind(&p, &b, &id).await.unwrap(), "nothing to move yet");
        bind(&p, &a, &id).await.unwrap();
        assert!(rebind(&p, &b, &id).await.unwrap());
        assert_eq!(find_account(&p, &id).await.unwrap(), Some(b));

        let named =
            PlatformIdentity::new(Channel::Telegram, id.user_id().clone(), Some("Evan".into()));
        touch_display_name(&p, &named).await.unwrap();
        let listed = list(&p, &b).await.unwrap();
        assert_eq!(listed[0].display_name.as_deref(), Some("Evan"));
    }
}
