//! Schema for the authentication and account layer.
//!
//! Every table here is read by `accounts/`. They live apart from the
//! application tables in `schema.rs` because they answer a different question
//! — who someone is, rather than what they own — and because keeping them
//! together makes the set easy to audit as a whole.

use sqlx::SqlitePool;

/// Tables backing sign-in: accounts, the identities that reach them, the
/// credentials that open them, and the paperwork for linking and recovery.
///
/// Split out of `schema.rs` purely for size. This is the same migration
/// mechanism, called from the same place, in the same order — there is still
/// exactly one, and `db/backfill.rs` still runs after all of it.
pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
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

    // One-time codes for claiming a Matrix account.
    //
    // The web UI issues one and the user DMs it to the bot from the MXID they
    // are claiming, so proof of ownership *is* the message sender and cannot be
    // forged. `consumed_at` makes the code single-use: a code seen in a public
    // room is already burned by the time anyone could race it.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS matrix_link_challenges (
            code        TEXT PRIMARY KEY,
            account_id  TEXT NOT NULL,
            created_at  TIMESTAMP NOT NULL,
            expires_at  TIMESTAMP NOT NULL,
            consumed_at TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Single-use recovery codes, bcrypt-hashed for the same reason a password
    // would be: a leaked backup must not hand someone a way in.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS recovery_codes (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL,
            code_hash   TEXT NOT NULL,
            created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            used_at     TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recovery_codes_account ON recovery_codes(account_id);",
    )
    .execute(pool)
    .await?;

    // Recovery started from a linked Matrix account.
    //
    // `valid_after` is what makes this survivable when the Matrix account is
    // the thing that was compromised: the request is announced immediately and
    // does nothing for a day, during which any live session can cancel it.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS recovery_requests (
            id            TEXT PRIMARY KEY,
            account_id    TEXT NOT NULL,
            code_hash     TEXT NOT NULL,
            requested_at  TIMESTAMP NOT NULL,
            valid_after   TIMESTAMP NOT NULL,
            expires_at    TIMESTAMP NOT NULL,
            cancelled_at  TIMESTAMP,
            consumed_at   TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recovery_requests_account \
         ON recovery_requests(account_id);",
    )
    .execute(pool)
    .await?;

    // The account's own audit log.
    //
    // Written for every sensitive action, and readable in the web UI. This is
    // what gives an account with no linked Matrix identity somewhere to see
    // security notices at all — a DM needs somewhere to send to.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS security_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id  TEXT NOT NULL,
            kind        TEXT NOT NULL,
            detail      TEXT,
            created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_security_events_account \
         ON security_events(account_id, created_at DESC);",
    )
    .execute(pool)
    .await?;

    Ok(())
}
