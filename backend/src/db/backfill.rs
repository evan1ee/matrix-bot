//! One-shot data conversions that run alongside the schema migration.
//!
//! Separated from `schema.rs` because they answer a different question. That
//! file describes the shape the database should have; this one rewrites what is
//! already inside it, and every step here is guarded so re-running is a no-op.
//!
//! There is no migration table in this project, so nothing is versioned — each
//! conversion carries its own guard instead. **Order matters**:
//! `backfill_accounts` creates the rows that `demote_credentialless_accounts`
//! then inspects, so running the demotion first silently does nothing.

use crate::db::DbPool;
use tracing::info;

/// Runs every conversion, in the only order that works.
pub async fn run(pool: &DbPool) -> Result<(), sqlx::Error> {
    backfill_accounts(pool).await?;
    demote_credentialless_accounts(pool).await?;
    Ok(())
}

/// Gives every pre-existing row an `accounts` row to belong to.
///
/// Before this, `owner_id` held a raw MXID for anything created over Matrix and
/// `@web-<username>:api` for anything created through the API. Both are
/// rewritten to an account id here, once.
///
/// A Matrix owner becomes an **unclaimed** account with its MXID linked: it has
/// history and no credential, which is precisely the state a first-time sender
/// would be in today. Registering a passkey and linking that MXID then adopts
/// it, so this migration doubles as the first real exercise of the merge path.
///
/// Guarded on `accounts` being empty rather than on a version number, because
/// there is no migration table in this project and re-running the rewrite would
/// mint a second account for owners that had already been converted.
async fn backfill_accounts(pool: &DbPool) -> Result<(), sqlx::Error> {
    let existing: i64 = crate::db_scalar_one!(pool, _, "SELECT COUNT(*) FROM accounts")?;
    if existing > 0 {
        return Ok(());
    }

    // Every distinct legacy owner across every table that carries one.
    let mut legacy: Vec<String> = Vec::new();
    for table in crate::accounts::OWNED_TABLES {
        let owners = crate::db_scalar_all!(
            pool,
            String,
            &format!(
            "SELECT DISTINCT owner_id FROM {table} WHERE owner_id IS NOT NULL AND owner_id != ''"
        )
        )
        .unwrap_or_default();
        for owner in owners {
            if !legacy.contains(&owner) {
                legacy.push(owner);
            }
        }
    }

    if legacy.is_empty() {
        let users: i64 =
            crate::db_scalar_one!(pool, _, "SELECT COUNT(*) FROM web_users").unwrap_or(0);
        if users == 0 {
            return Ok(());
        }
    }

    info!("Backfilling {} legacy owner(s) onto accounts", legacy.len());

    for owner in &legacy {
        // `@web-<username>:api` was the API's namespace for a local account;
        // anything else starting with `@` was a real Matrix user.
        let local_username = owner
            .strip_prefix("@web-")
            .and_then(|rest| rest.strip_suffix(":api"));

        let (display_name, status, mxid) = match local_username {
            Some(username) => {
                // The `web_users` row is the credential. Without one there is
                // nothing to authenticate as, so this owner is data left behind
                // by a local account that no longer exists — already
                // unreachable before this migration ran, not made so by it.
                //
                // It is marked unclaimed rather than active, and kept rather
                // than deleted: a migration must not be the thing that destroys
                // someone's rows, and 'unclaimed' is exactly the state of
                // "owns data, holds no credential".
                let has_credential: bool = crate::db_scalar_one!(
                    pool,
                    i64,
                    "SELECT COUNT(*) FROM web_users WHERE username = ?",
                    username
                )
                .unwrap_or(0)
                    > 0;

                if !has_credential {
                    tracing::warn!(
                        "Legacy owner {} has no web_users row; its data is kept under an \
                         unclaimed account that nobody can currently sign in to",
                        owner
                    );
                }

                (
                    username.to_string(),
                    if has_credential {
                        crate::accounts::STATUS_ACTIVE
                    } else {
                        crate::accounts::STATUS_UNCLAIMED
                    },
                    None,
                )
            }
            None => (
                crate::accounts::display_name_for_mxid(owner),
                crate::accounts::STATUS_UNCLAIMED,
                Some(owner.clone()),
            ),
        };

        let account_id = crate::db_scalar_one!(
            pool,
            String,
            "INSERT INTO accounts (id, display_name, status) \
             VALUES (?, ?, ?) RETURNING id",
            crate::utils::random::new_account_id(),
            &display_name,
            status
        )?;

        if let Some(mxid) = mxid {
            crate::db_execute!(
                pool,
                "INSERT INTO matrix_links (mxid, account_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
                &mxid,
                &account_id
            )?;
        } else {
            crate::db_execute!(
                pool,
                "UPDATE web_users SET account_id = ? WHERE username = ?",
                &account_id,
                &display_name
            )?;
        }

        for table in crate::accounts::OWNED_TABLES {
            let _ = crate::db_execute!(
                pool,
                &format!("UPDATE {table} SET owner_id = ? WHERE owner_id = ?"),
                &account_id,
                owner
            );
        }
    }

    // A registered user who never created a row still needs an account, or
    // their first login would mint one and look like a brand-new person.
    let orphan_users = crate::db_scalar_all!(
        pool,
        String,
        "SELECT username FROM web_users WHERE account_id IS NULL"
    )
    .unwrap_or_default();

    for username in orphan_users {
        let account_id = crate::db_scalar_one!(
            pool,
            String,
            "INSERT INTO accounts (id, display_name, status) \
             VALUES (?, ?, 'active') RETURNING id",
            crate::utils::random::new_account_id(),
            &username
        )?;

        crate::db_execute!(
            pool,
            "UPDATE web_users SET account_id = ? WHERE username = ?",
            &account_id,
            &username
        )?;
    }

    Ok(())
}

/// Demotes accounts whose only credential was a password.
///
/// Passwords are gone, so such an account can no longer sign in. Calling it
/// 'active' would be a claim the schema can no longer honour: 'active' means
/// "holds a passkey", and it holds none.
///
/// Demoted rather than deleted — the rows it owns are real, and 'unclaimed' is
/// exactly the state of "owns data, holds no credential". Linking a Matrix
/// account to it later restores access to everything in it.
async fn demote_credentialless_accounts(pool: &DbPool) -> Result<(), sqlx::Error> {
    let demoted = crate::db_execute!(
        pool,
        "UPDATE accounts SET status = 'unclaimed' \
         WHERE status = 'active' \
           AND NOT EXISTS (SELECT CAST(1 AS BIGINT) FROM webauthn_credentials c WHERE c.account_id = accounts.id)"
    )?;

    if demoted > 0 {
        info!(
            "{demoted} account(s) had only a password and now hold no credential; \
             marked unclaimed. Their data is intact — register a passkey and link a \
             Matrix account to reach it."
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::schema::run_migrations;

    /// A fresh install has nothing to convert and must not invent an account.
    #[tokio::test]
    async fn test_backfill_is_a_no_op_on_an_empty_database() {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();

        let accounts: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM accounts").unwrap();
        assert_eq!(accounts, 0);
    }
}
