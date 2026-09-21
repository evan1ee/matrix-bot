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
             VALUES ('acct_' || lower(hex(randomblob(16))), ?, ?) RETURNING id",
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
             VALUES ('acct_' || lower(hex(randomblob(16))), ?, 'active') RETURNING id",
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
           AND NOT EXISTS (SELECT 1 FROM webauthn_credentials c WHERE c.account_id = accounts.id)"
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

    /// Builds a database in the shape this project had before accounts existed:
    /// `sender_mxid` columns, and owner strings from the two old identity
    /// spaces. This is what every deployed instance actually looks like on the
    /// boot that first runs the new migration.
    async fn legacy_database() -> crate::db::DbPool {
        let pool =
            crate::db::DbPool::Sqlite(sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap());

        crate::db_execute!(
            &pool,
            "CREATE TABLE todos (id INTEGER PRIMARY KEY AUTOINCREMENT, room_id TEXT NOT NULL, \
             sender_mxid TEXT NOT NULL, sender_name TEXT NOT NULL, text TEXT NOT NULL, \
             priority TEXT NOT NULL DEFAULT 'medium', due_at TEXT, \
             completed BOOLEAN NOT NULL DEFAULT 0, created_at TIMESTAMP)"
        )
        .unwrap();

        crate::db_execute!(
            &pool,
            "CREATE TABLE web_users (id INTEGER PRIMARY KEY AUTOINCREMENT, \
             username TEXT UNIQUE NOT NULL, password_hash TEXT NOT NULL, created_at TIMESTAMP)"
        )
        .unwrap();

        crate::db_execute!(
            &pool,
            "CREATE TABLE user_settings (owner_id TEXT PRIMARY KEY, \
             timezone TEXT NOT NULL DEFAULT 'UTC', currency TEXT NOT NULL DEFAULT 'USD', \
             updated_at TIMESTAMP)"
        )
        .unwrap();

        for (owner, text) in [
            ("@evan:mbm.one", "from matrix"),
            ("@evan:mbm.one", "also from matrix"),
            ("@web-demo:api", "from the web"),
        ] {
            crate::db_execute!(
                &pool,
                "INSERT INTO todos (room_id, sender_mxid, sender_name, text, priority, completed) \
                 VALUES ('!room:x', ?, 'evan', ?, 'medium', 0)",
                owner,
                text
            )
            .unwrap();
        }

        crate::db_execute!(
            &pool,
            "INSERT INTO user_settings (owner_id, timezone, currency) \
             VALUES ('@evan:mbm.one', 'Asia/Tokyo', 'JPY')"
        )
        .unwrap();

        crate::db_execute!(
            &pool,
            "INSERT INTO web_users (username, password_hash) VALUES ('demo', 'x')"
        )
        .unwrap();

        pool
    }

    /// Every legacy owner string must end up as an account id, with its rows
    /// following it. A row left holding an MXID after this is a row its owner
    /// can no longer see from anywhere.
    #[tokio::test]
    async fn test_backfill_converts_both_legacy_identity_spaces() {
        let pool = legacy_database().await;
        run_migrations(&pool).await.unwrap();

        // The column itself was renamed, so the old name must be gone.
        let owners: Vec<String> =
            crate::db_scalar_all!(&pool, _, "SELECT DISTINCT owner_id FROM todos").unwrap();
        assert_eq!(owners.len(), 2, "two legacy owners, two accounts");
        assert!(
            owners.iter().all(|o| o.starts_with("acct_")),
            "a row still holds a legacy owner: {owners:?}"
        );

        // The Matrix owner became an unclaimed account with its MXID linked.
        let matrix_account: String = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT account_id FROM matrix_links WHERE mxid = '@evan:mbm.one'"
        )
        .unwrap();
        let status: String = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT status FROM accounts WHERE id = ?",
            &matrix_account
        )
        .unwrap();
        assert_eq!(status, "unclaimed", "a Matrix owner has no credential yet");

        // Both of that MXID's todos moved together.
        let moved: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM todos WHERE owner_id = ?",
            &matrix_account
        )
        .unwrap();
        assert_eq!(moved, 2);

        // Settings followed the same owner, so the user keeps Asia/Tokyo.
        let tz: String = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT timezone FROM user_settings WHERE owner_id = ?",
            &matrix_account
        )
        .unwrap();
        assert_eq!(tz, "Asia/Tokyo");

        // The web owner became an active account wired to its `web_users` row,
        // so the existing password login keeps working and lands on its data.
        let web_account: String = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT account_id FROM web_users WHERE username = 'demo'"
        )
        .unwrap();
        assert_ne!(
            web_account, matrix_account,
            "unlinked identities stay separate"
        );

        let web_todos: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM todos WHERE owner_id = ?",
            &web_account
        )
        .unwrap();
        assert_eq!(web_todos, 1);
    }

    /// The migration runs on every boot. A second pass must not mint a second
    /// set of accounts and strand every row against the first set.
    #[tokio::test]
    async fn test_backfill_runs_once_however_often_migrations_run() {
        let pool = legacy_database().await;

        run_migrations(&pool).await.unwrap();
        let after_first: Vec<String> =
            crate::db_scalar_all!(&pool, _, "SELECT id FROM accounts ORDER BY id").unwrap();

        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let after_third: Vec<String> =
            crate::db_scalar_all!(&pool, _, "SELECT id FROM accounts ORDER BY id").unwrap();

        assert_eq!(
            after_first, after_third,
            "re-running must not create accounts"
        );

        let orphans: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM todos WHERE owner_id NOT IN (SELECT id FROM accounts)"
        )
        .unwrap();
        assert_eq!(orphans, 0, "every row must still point at a live account");
    }

    /// A registered user who never wrote a row still needs an account, or their
    /// next login would provision one and present as a brand-new person.
    #[tokio::test]
    async fn test_backfill_covers_users_with_no_data() {
        let pool = legacy_database().await;
        crate::db_execute!(
            &pool,
            "INSERT INTO web_users (username, password_hash) VALUES ('quiet', 'x')"
        )
        .unwrap();

        run_migrations(&pool).await.unwrap();

        let account: Option<String> = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT account_id FROM web_users WHERE username = 'quiet'"
        )
        .unwrap();
        assert!(
            account.is_some(),
            "a user with no rows still needs an account"
        );
    }

    /// Migrates a real database file and checks nothing was stranded.
    ///
    /// `#[ignore]`d because it needs a database to point at:
    ///
    /// ```text
    /// cp data/bot_data.db /tmp/copy.db
    /// MIGRATE_TEST_DB=/tmp/copy.db cargo test migrates_a_real_database -- --ignored
    /// ```
    ///
    /// Always run it against a **copy**. The value of this over the synthetic
    /// fixtures above is that a real database carries owner strings nobody
    /// wrote a fixture for — the live one turned out to hold a
    /// `@web-claude-smoketest:api` whose `web_users` row no longer exists.
    #[tokio::test]
    #[ignore]
    async fn test_migrates_a_real_database_without_stranding_rows() {
        let Ok(path) = std::env::var("MIGRATE_TEST_DB") else {
            panic!("set MIGRATE_TEST_DB to a *copy* of a real database");
        };

        let pool = crate::db::DbPool::Sqlite(
            sqlx::SqlitePool::connect(&format!("sqlite://{path}"))
                .await
                .unwrap(),
        );

        let before: i64 = crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM todos").unwrap();

        run_migrations(&pool).await.unwrap();

        let after: i64 = crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM todos").unwrap();
        assert_eq!(
            before, after,
            "the migration must not lose or duplicate rows"
        );

        // Nothing may still point at an owner that no account claims.
        for table in crate::accounts::OWNED_TABLES {
            let stranded: i64 = crate::db_scalar_one!(
                &pool,
                _,
                &format!(
                    "SELECT COUNT(*) FROM {table} WHERE owner_id NOT IN (SELECT id FROM accounts)"
                )
            )
            .unwrap();
            assert_eq!(stranded, 0, "{table} has rows no account owns");
        }

        // 'active' asserts a passkey exists. An active account without one is
        // the migration having lied about the state it created.
        let unreachable_active: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM accounts a WHERE a.status = 'active' \
               AND NOT EXISTS (SELECT 1 FROM webauthn_credentials c WHERE c.account_id = a.id)"
        )
        .unwrap();
        assert_eq!(
            unreachable_active, 0,
            "an account claims to be active but holds no passkey"
        );

        // Unclaimed accounts with no linked MXID are legacy orphans — data from
        // a local account that was deleted before this migration. Reported, not
        // failed: the migration inherited them, it did not create them.
        let orphans: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM accounts a WHERE a.status = 'unclaimed' \
               AND NOT EXISTS (SELECT 1 FROM matrix_links m WHERE m.account_id = a.id)"
        )
        .unwrap();
        if orphans > 0 {
            println!("note: {orphans} orphaned legacy account(s) carried over, data preserved");
        }
    }

    /// A fresh install has nothing to convert and must not invent an account.
    #[tokio::test]
    async fn test_backfill_is_a_no_op_on_an_empty_database() {
        let pool =
            crate::db::DbPool::Sqlite(sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();

        let accounts: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM accounts").unwrap();
        assert_eq!(accounts, 0);
    }
}
