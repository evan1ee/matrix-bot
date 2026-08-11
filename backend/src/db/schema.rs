use sqlx::SqlitePool;
use tracing::info;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    info!("Running database table schema initialization...");

    // The ownership column used to be called `sender_mxid`, because it held a
    // Matrix user ID. It now holds an `accounts.id`, so the old name actively
    // lies about what is in it — and a column whose name contradicts its
    // contents is exactly how the two-identity-space bug survived as long as it
    // did. Renamed first, before anything below reads or indexes the column.
    //
    // Best-effort per the convention in this file: on a fresh database the
    // table does not exist yet, and on an already-migrated one the column is
    // already called `owner_id`. Both raise an error that means "nothing to do".
    // SQLite rewrites index definitions to follow a renamed column, so the
    // `CREATE INDEX IF NOT EXISTS` calls further down stay no-ops.
    for table in ["todos", "reminders", "transactions", "recurring_bills"] {
        let _ = sqlx::query(&format!(
            "ALTER TABLE {table} RENAME COLUMN sender_mxid TO owner_id;"
        ))
        .execute(pool)
        .await;
    }

    // Local accounts. Every `owner_id` in every other table points here.
    //
    // `status` is 'unclaimed' for an account the bot provisioned on a first
    // message from an unknown MXID: it owns real data but has no credential, so
    // it is reachable only through the MXID linked to it until someone claims
    // it by registering a passkey and linking that MXID.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS accounts (
            id            TEXT PRIMARY KEY,
            display_name  TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'active',
            created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Which Matrix identities may act as which account.
    //
    // `mxid` is the primary key, and that is the whole of the "one Matrix
    // account cannot be linked to two local accounts" rule — enforced by the
    // schema rather than by a check-then-insert that a concurrent link could
    // race. Unlinking deletes the row and leaves the data behind: the rows
    // belong to the account, and unlinking only revokes an access path.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS matrix_links (
            mxid        TEXT PRIMARY KEY,
            account_id  TEXT NOT NULL,
            linked_at   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_matrix_links_account ON matrix_links(account_id);",
    )
    .execute(pool)
    .await?;

    // Registered passkeys. `passkey_json` is the serialised `webauthn_rs`
    // credential, which carries the public key and the signature counter — the
    // library owns that shape, so it is stored whole rather than picked apart
    // into columns this code would then have to keep in step with the crate.
    //
    // There is no secret here: a passkey's private half never leaves the
    // authenticator, which is the entire reason this replaces `password_hash`.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS webauthn_credentials (
            id            TEXT PRIMARY KEY,
            account_id    TEXT NOT NULL,
            passkey_json  TEXT NOT NULL,
            label         TEXT NOT NULL,
            created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            last_used_at  TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_webauthn_credentials_account \
         ON webauthn_credentials(account_id);",
    )
    .execute(pool)
    .await?;

    // Live sessions.
    //
    // `id_hash` is the SHA-256 of the cookie value, never the value itself: a
    // read of this table must not yield credentials that can be replayed. That
    // mirrors how `api/auth/types.rs` already keys its token cache.
    //
    // `last_verified_at` is the step-up clock — a sensitive action requires a
    // passkey assertion newer than a few minutes, and this is what that is
    // measured against. `restricted_until` marks a session created by account
    // recovery, which may register a passkey and nothing else.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id_hash          TEXT PRIMARY KEY,
            account_id       TEXT NOT NULL,
            created_at       TIMESTAMP NOT NULL,
            last_seen_at     TIMESTAMP NOT NULL,
            last_verified_at TIMESTAMP NOT NULL,
            restricted_until TIMESTAMP,
            user_agent       TEXT,
            ip               TEXT,
            revoked_at       TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_account ON sessions(account_id);")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            text TEXT NOT NULL,
            priority TEXT NOT NULL DEFAULT 'medium',
            due_at TEXT,
            completed BOOLEAN NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            message TEXT NOT NULL,
            trigger_at TIMESTAMP NOT NULL,
            recurrence TEXT NOT NULL DEFAULT 'none',
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS web_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Per-user display preferences, keyed by the same `owner_id` as every other
    // ownership check. A user with no row uses the defaults, so rows appear only
    // when a preference is actually changed.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_settings (
            owner_id TEXT PRIMARY KEY,
            timezone TEXT NOT NULL DEFAULT 'UTC',
            currency TEXT NOT NULL DEFAULT 'USD',
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Investment holdings. One row per (owner, symbol) — `quantity_minor` and
    // `avg_cost_minor` are integers so weighted-average cost math never touches
    // a float; `currency_exponent` is stored per row for the same reason the
    // ledger stores it: a later change to the currency table cannot rewrite
    // what an existing row means.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_holdings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            quantity_minor INTEGER NOT NULL,
            avg_cost_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            current_price_minor INTEGER,
            price_updated_at TIMESTAMP,
            note TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_holdings_owner_symbol \
         ON portfolio_holdings(owner_id, symbol);",
    )
    .execute(pool)
    .await?;

    // Cash available to buy with, one row per (owner, currency). No CHECK
    // constraint on `balance_minor` — nothing else in this schema uses one;
    // non-negativity is enforced in `PortfolioRepository`, the same
    // app-level-only convention `quantity_minor`/`avg_cost_minor` already use.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_cash_balances (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            balance_minor INTEGER NOT NULL DEFAULT 0,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_cash_owner_currency \
         ON portfolio_cash_balances(owner_id, currency);",
    )
    .execute(pool)
    .await?;

    // Per-user spending categories. Names are unique per owner, not globally.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS finance_categories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_finance_categories_owner_name \
         ON finance_categories(owner_id, name);",
    )
    .execute(pool)
    .await?;

    // The ledger. `amount_minor` is always positive and `direction` carries the
    // sign, so a row cannot contradict itself; `currency_exponent` is stored per
    // row so a later change to the currency table cannot rewrite history.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            direction TEXT NOT NULL,
            amount_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            category_id INTEGER,
            note TEXT,
            occurred_at TIMESTAMP NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_transactions_owner_occurred \
         ON transactions(owner_id, occurred_at DESC);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS recurring_bills (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            name TEXT NOT NULL,
            category TEXT NOT NULL,
            amount_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            frequency TEXT NOT NULL DEFAULT 'monthly',
            due_day INTEGER,
            next_due_at TIMESTAMP NOT NULL,
            total_installments INTEGER,
            paid_installments INTEGER NOT NULL DEFAULT 0,
            auto_pay BOOLEAN NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'active',
            notes TEXT,
            remind_before_minutes INTEGER,
            remind_room_id TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recurring_bills_owner ON recurring_bills(owner_id, status);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recurring_bills_due ON recurring_bills(owner_id, next_due_at);",
    )
    .execute(pool)
    .await?;

    // Composite indexes for performance optimization
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(owner_id, room_id);"
    )
    .execute(pool)
    .await?;

    // Migration helper for existing sqlite DBs missing retry_count column
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;")
        .execute(pool)
        .await;

    // How long before `due_at` to fire a reminder. Kept on the todo rather than
    // derived from the linked reminder so the web UI can read the current
    // setting back without reverse-engineering it from a timestamp difference.
    let _ = sqlx::query("ALTER TABLE todos ADD COLUMN remind_before_minutes INTEGER;")
        .execute(pool)
        .await;

    // Links a reminder back to the row whose due date scheduled it, so the
    // reminder can be rescheduled or cancelled when that row changes.
    //
    // This started as a bare `todo_id`. Bills want the same lead-time reminder,
    // and a second nullable foreign key per module does not scale, so the link
    // is now `(source_kind, source_id)`: one set of repository functions serves
    // every module that owns a due date.
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN todo_id INTEGER;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN source_kind TEXT;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN source_id INTEGER;")
        .execute(pool)
        .await;

    // Carry existing todo links over. Guarded on `source_id IS NULL` so a
    // re-run cannot overwrite a link that has since moved.
    let _ = sqlx::query(
        "UPDATE reminders SET source_kind = 'todo', source_id = todo_id \
         WHERE todo_id IS NOT NULL AND source_id IS NULL;",
    )
    .execute(pool)
    .await;

    let _ = sqlx::query("DROP INDEX IF EXISTS idx_reminders_todo;")
        .execute(pool)
        .await;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_source ON reminders(source_kind, source_id);",
    )
    .execute(pool)
    .await?;

    // Migration helpers for recurring_bills columns on existing SQLite DBs
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN currency_exponent INTEGER NOT NULL DEFAULT 2;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN frequency TEXT NOT NULL DEFAULT 'monthly';").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN due_day INTEGER;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN next_due_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN total_installments INTEGER;").execute(pool).await;
    // Lead-time reminder, mirroring the todo columns above: minutes before
    // `next_due_at`, and where to deliver it when the bill has no room of its
    // own (the `web` sentinel).
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN remind_before_minutes INTEGER;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN remind_room_id TEXT;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN paid_installments INTEGER NOT NULL DEFAULT 0;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN auto_pay BOOLEAN NOT NULL DEFAULT 0;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN status TEXT NOT NULL DEFAULT 'active';").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN notes TEXT;").execute(pool).await;

    // `due_at` used to hold the rendered display string (`2026-12-31 23:59 UTC`),
    // which cannot be compared or subtracted from. Rewrite legacy rows into the
    // timestamp format sqlx reads back into `DateTime<Utc>`. The `strftime`
    // guard leaves anything unparseable alone rather than nulling the column.
    let _ = sqlx::query(
        r#"
        UPDATE todos
        SET due_at = strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', ''))
        WHERE due_at LIKE '% UTC'
          AND strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', '')) IS NOT NULL;
        "#,
    )
    .execute(pool)
    .await;

    // Clean up legacy matrix.org room_id placeholders
    let _ = sqlx::query("UPDATE reminders SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;
    let _ = sqlx::query("UPDATE todos SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;

    // Ties an existing password login to its account. Phase 2 deletes
    // `web_users` entirely and moves the credential onto `accounts`; this
    // column is what keeps password login working in the meantime.
    let _ = sqlx::query("ALTER TABLE web_users ADD COLUMN account_id TEXT;")
        .execute(pool)
        .await;

    backfill_accounts(pool).await?;

    info!("Database migrations applied successfully.");
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
async fn backfill_accounts(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(pool)
        .await?;
    if existing > 0 {
        return Ok(());
    }

    // Every distinct legacy owner across every table that carries one.
    let mut legacy: Vec<String> = Vec::new();
    for table in crate::accounts::OWNED_TABLES {
        let owners = sqlx::query_scalar::<_, String>(&format!(
            "SELECT DISTINCT owner_id FROM {table} WHERE owner_id IS NOT NULL AND owner_id != ''"
        ))
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for owner in owners {
            if !legacy.contains(&owner) {
                legacy.push(owner);
            }
        }
    }

    if legacy.is_empty() {
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM web_users")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
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
                let has_credential: bool = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM web_users WHERE username = ?",
                )
                .bind(username)
                .fetch_one(pool)
                .await
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

        let account_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO accounts (id, display_name, status) \
             VALUES ('acct_' || lower(hex(randomblob(16))), ?, ?) RETURNING id",
        )
        .bind(&display_name)
        .bind(status)
        .fetch_one(pool)
        .await?;

        if let Some(mxid) = mxid {
            sqlx::query("INSERT OR IGNORE INTO matrix_links (mxid, account_id) VALUES (?, ?)")
                .bind(&mxid)
                .bind(&account_id)
                .execute(pool)
                .await?;
        } else {
            sqlx::query("UPDATE web_users SET account_id = ? WHERE username = ?")
                .bind(&account_id)
                .bind(&display_name)
                .execute(pool)
                .await?;
        }

        for table in crate::accounts::OWNED_TABLES {
            let _ = sqlx::query(&format!("UPDATE {table} SET owner_id = ? WHERE owner_id = ?"))
                .bind(&account_id)
                .bind(owner)
                .execute(pool)
                .await;
        }
    }

    // A registered user who never created a row still needs an account, or
    // their first login would mint one and look like a brand-new person.
    let orphan_users = sqlx::query_scalar::<_, String>(
        "SELECT username FROM web_users WHERE account_id IS NULL",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for username in orphan_users {
        let account_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO accounts (id, display_name, status) \
             VALUES ('acct_' || lower(hex(randomblob(16))), ?, 'active') RETURNING id",
        )
        .bind(&username)
        .fetch_one(pool)
        .await?;

        sqlx::query("UPDATE web_users SET account_id = ? WHERE username = ?")
            .bind(&account_id)
            .bind(&username)
            .execute(pool)
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn table_names(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_creates_every_table() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let tables = table_names(&pool).await;
        for expected in [
            "finance_categories",
            "portfolio_cash_balances",
            "portfolio_holdings",
            "recurring_bills",
            "reminders",
            "todos",
            "transactions",
            "user_settings",
            "web_users",
        ] {
            assert!(tables.contains(&expected.to_string()), "missing table {expected}");
        }
    }

    /// This is the only migration mechanism there is, and it runs on every boot.
    /// It must be safe to apply repeatedly to an already-migrated database.
    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        run_migrations(&pool).await.unwrap();
        let first = table_names(&pool).await;

        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let third = table_names(&pool).await;

        assert_eq!(first, third, "re-running migrations must not change the schema");
    }

    /// Re-running migrations must not disturb rows that already exist.
    #[tokio::test]
    async fn test_migrations_preserve_existing_data() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', '@a:x', 'a', 'keep me', 'medium', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        run_migrations(&pool).await.unwrap();

        let text: String = sqlx::query_scalar("SELECT text FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(text, "keep me");
    }

    /// Builds a database in the shape this project had before accounts existed:
    /// `sender_mxid` columns, and owner strings from the two old identity
    /// spaces. This is what every deployed instance actually looks like on the
    /// boot that first runs the new migration.
    async fn legacy_database() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE todos (id INTEGER PRIMARY KEY AUTOINCREMENT, room_id TEXT NOT NULL, \
             sender_mxid TEXT NOT NULL, sender_name TEXT NOT NULL, text TEXT NOT NULL, \
             priority TEXT NOT NULL DEFAULT 'medium', due_at TEXT, \
             completed BOOLEAN NOT NULL DEFAULT 0, created_at TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE web_users (id INTEGER PRIMARY KEY AUTOINCREMENT, \
             username TEXT UNIQUE NOT NULL, password_hash TEXT NOT NULL, created_at TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE user_settings (owner_id TEXT PRIMARY KEY, \
             timezone TEXT NOT NULL DEFAULT 'UTC', currency TEXT NOT NULL DEFAULT 'USD', \
             updated_at TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();

        for (owner, text) in [
            ("@evan:mbm.one", "from matrix"),
            ("@evan:mbm.one", "also from matrix"),
            ("@web-demo:api", "from the web"),
        ] {
            sqlx::query(
                "INSERT INTO todos (room_id, sender_mxid, sender_name, text, priority, completed) \
                 VALUES ('!room:x', ?, 'evan', ?, 'medium', 0)",
            )
            .bind(owner)
            .bind(text)
            .execute(&pool)
            .await
            .unwrap();
        }

        sqlx::query(
            "INSERT INTO user_settings (owner_id, timezone, currency) \
             VALUES ('@evan:mbm.one', 'Asia/Tokyo', 'JPY')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("INSERT INTO web_users (username, password_hash) VALUES ('demo', 'x')")
            .execute(&pool)
            .await
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
        let owners: Vec<String> = sqlx::query_scalar("SELECT DISTINCT owner_id FROM todos")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(owners.len(), 2, "two legacy owners, two accounts");
        assert!(
            owners.iter().all(|o| o.starts_with("acct_")),
            "a row still holds a legacy owner: {owners:?}"
        );

        // The Matrix owner became an unclaimed account with its MXID linked.
        let matrix_account: String =
            sqlx::query_scalar("SELECT account_id FROM matrix_links WHERE mxid = '@evan:mbm.one'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let status: String = sqlx::query_scalar("SELECT status FROM accounts WHERE id = ?")
            .bind(&matrix_account)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "unclaimed", "a Matrix owner has no credential yet");

        // Both of that MXID's todos moved together.
        let moved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM todos WHERE owner_id = ?")
            .bind(&matrix_account)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(moved, 2);

        // Settings followed the same owner, so the user keeps Asia/Tokyo.
        let tz: String = sqlx::query_scalar("SELECT timezone FROM user_settings WHERE owner_id = ?")
            .bind(&matrix_account)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tz, "Asia/Tokyo");

        // The web owner became an active account wired to its `web_users` row,
        // so the existing password login keeps working and lands on its data.
        let web_account: String =
            sqlx::query_scalar("SELECT account_id FROM web_users WHERE username = 'demo'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_ne!(web_account, matrix_account, "unlinked identities stay separate");

        let web_todos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM todos WHERE owner_id = ?")
            .bind(&web_account)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(web_todos, 1);
    }

    /// The migration runs on every boot. A second pass must not mint a second
    /// set of accounts and strand every row against the first set.
    #[tokio::test]
    async fn test_backfill_runs_once_however_often_migrations_run() {
        let pool = legacy_database().await;

        run_migrations(&pool).await.unwrap();
        let after_first: Vec<String> = sqlx::query_scalar("SELECT id FROM accounts ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();

        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let after_third: Vec<String> = sqlx::query_scalar("SELECT id FROM accounts ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert_eq!(after_first, after_third, "re-running must not create accounts");

        let orphans: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM todos WHERE owner_id NOT IN (SELECT id FROM accounts)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(orphans, 0, "every row must still point at a live account");
    }

    /// A registered user who never wrote a row still needs an account, or their
    /// next login would provision one and present as a brand-new person.
    #[tokio::test]
    async fn test_backfill_covers_users_with_no_data() {
        let pool = legacy_database().await;
        sqlx::query("INSERT INTO web_users (username, password_hash) VALUES ('quiet', 'x')")
            .execute(&pool)
            .await
            .unwrap();

        run_migrations(&pool).await.unwrap();

        let account: Option<String> =
            sqlx::query_scalar("SELECT account_id FROM web_users WHERE username = 'quiet'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(account.is_some(), "a user with no rows still needs an account");
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

        let pool = SqlitePool::connect(&format!("sqlite://{path}")).await.unwrap();

        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();

        run_migrations(&pool).await.unwrap();

        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, after, "the migration must not lose or duplicate rows");

        // Nothing may still point at an owner that no account claims.
        for table in crate::accounts::OWNED_TABLES {
            let stranded: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE owner_id NOT IN (SELECT id FROM accounts)"
            ))
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(stranded, 0, "{table} has rows no account owns");
        }

        // Every *active* account must be reachable: 'active' asserts a
        // credential exists, so an active account with no way in is the
        // migration having lied about the state it created.
        let unreachable_active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM accounts a WHERE a.status = 'active' \
               AND NOT EXISTS (SELECT 1 FROM matrix_links m WHERE m.account_id = a.id) \
               AND NOT EXISTS (SELECT 1 FROM web_users w WHERE w.account_id = a.id)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            unreachable_active, 0,
            "an account claims to be active but has no credential behind it"
        );

        // Unclaimed accounts with no linked MXID are legacy orphans — data from
        // a local account that was deleted before this migration. Reported, not
        // failed: the migration inherited them, it did not create them.
        let orphans: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM accounts a WHERE a.status = 'unclaimed' \
               AND NOT EXISTS (SELECT 1 FROM matrix_links m WHERE m.account_id = a.id)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if orphans > 0 {
            println!("note: {orphans} orphaned legacy account(s) carried over, data preserved");
        }
    }

    /// A fresh install has nothing to convert and must not invent an account.
    #[tokio::test]
    async fn test_backfill_is_a_no_op_on_an_empty_database() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(accounts, 0);
    }

    /// The unique index is what stops a user ending up with two categories of
    /// the same name; the repository relies on it rather than a read-then-write.
    #[tokio::test]
    async fn test_category_name_uniqueness_is_enforced_by_the_schema() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')")
            .execute(&pool)
            .await
            .unwrap();

        let duplicate =
            sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')")
                .execute(&pool)
                .await;
        assert!(duplicate.is_err(), "a duplicate name must be rejected by the index");

        // A different owner may reuse the name.
        sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@b:x', 'food')")
            .execute(&pool)
            .await
            .unwrap();
    }
}
