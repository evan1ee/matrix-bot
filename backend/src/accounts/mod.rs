//! Local accounts — the single thing that owns a row in this database.
//!
//! Before this module existed, `owner_id` held one of two shapes: a raw MXID
//! for anything created over Matrix, or `@web-<username>:api` for anything
//! created through the REST API. Those are two identity spaces with no bridge,
//! so the same human using both saw two disjoint datasets and had no way to
//! join them.
//!
//! Now every row is owned by an `accounts` row, and both access paths resolve
//! onto it: a Matrix message resolves through [`matrix_links`], a web session
//! resolves directly. `owner_id` stays a `TEXT` column compared for equality,
//! so every repository and every ownership predicate is unchanged — only the
//! value it holds is different.
//!
//! # Two statuses, one difference
//!
//! An account is `unclaimed` until it has a credential of its own. The bot
//! provisions one the first time an unknown MXID sends a command, which is what
//! keeps the bot usable with no web signup at all: the user's data accumulates
//! against a real account from their first message. Registering a passkey and
//! linking that MXID later adopts the account and everything in it.
//!
//! An unclaimed account can be reached *only* through the MXID linked to it.
//! It has no credential, so there is nothing to authenticate as, and nothing to
//! steal by guessing its id.

pub mod credentials;
pub mod linking;
pub mod merge;
pub mod recovery;
pub mod repository;
pub mod security_events;
pub mod session;

pub use credentials::{CredentialRepository, EnrollmentRepository};
pub use linking::{LinkOutcome, LinkRepository};
pub use merge::{MergePreview, MergeRepository};
pub use recovery::{MaturedRecovery, RecoveryRepository, RECOVERY_RESTRICTION_HOURS};
pub use repository::AccountRepository;
pub use security_events::{SecurityEventKind, SecurityEventRepository};
pub use session::{Session, SessionRepository};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An account that has a credential of its own.
pub const STATUS_ACTIVE: &str = "active";

/// An account auto-provisioned for an MXID, reachable only through that MXID
/// until someone claims it by registering a credential.
pub const STATUS_UNCLAIMED: &str = "unclaimed";

/// Every table that carries an `owner_id` and whose rows follow a merge.
///
/// Two separate operations need this exact list — the Phase 1 backfill and the
/// Phase 3 account merge — and a table missing from it is a silent data leak
/// rather than a compile error. It drifted three tables behind the schema
/// before `test_owned_tables_and_exceptions_match_the_schema` existed; that
/// test now derives the truth from `PRAGMA table_info` and fails on any new
/// `owner_id` table that nobody classified.
pub const OWNED_TABLES: &[&str] = &[
    "todos",
    "reminders",
    "transactions",
    "recurring_bills",
    "portfolio_holdings",
    "portfolio_cash_balances",
    "portfolio_daily_snapshots",
    "portfolio_price_alerts",
    "finance_categories",
    "finance_shares",
    "user_settings",
    "bookmarks",
    "habits",
    "habit_logs",
    "life_day_logs",
    "life_profiles",
    "footprints",
    "ai_conversations",
    "ai_user_configs",
    "webhooks",
    "webhook_outbox",
    "bot_message_targets",
];

/// Tables carrying `owner_id` that a merge moves through a dedicated routine
/// rather than the generic loop.
///
/// `portfolio_audit_log` is immutable at the database engine level and its
/// `entry_hash` folds `owner_id` into the digest, so it can be neither
/// re-pointed by a plain `UPDATE` nor left valid by one. It is moved by
/// `modules::portfolio::audit::absorb_ledger`, which drops the trigger for the
/// length of the transaction and rebuilds the chain. Listed separately so the
/// schema-derived test still accounts for it, and so nobody adds it to
/// `OWNED_TABLES` and discovers at runtime that every merge now aborts.
pub const OWNER_ID_MERGED_SEPARATELY: &[&str] = &["portfolio_audit_log"];

/// Tables keyed by `account_id` whose rows follow the absorbed account.
pub const ACCOUNT_ID_FOLLOWS: &[&str] = &[
    "matrix_links",
    "webauthn_credentials",
    "security_events",
    "account_rooms",
];

/// Tables keyed by `account_id` whose rows are destroyed rather than moved.
///
/// Every one is a credential or a ceremony in flight. A credential belongs to
/// the identity that minted it: carrying one across a merge would hand whoever
/// held the absorbed account a live way in to the survivor. `sessions` is
/// revoked rather than deleted so the row survives as an audit trail; the rest
/// are deleted outright.
pub const ACCOUNT_ID_DISCARDED: &[&str] = &[
    "sessions",
    "recovery_codes",
    "recovery_requests",
    "mcp_tokens",
    "passkey_enrollment_tokens",
    "matrix_link_challenges",
];

/// Tables carrying `account_id` that a merge deliberately leaves alone.
///
/// `web_users` is the dead password table. Nothing authenticates against it —
/// only `db::backfill` reads it, once, to decide whether a legacy owner had a
/// credential. Deleting from it would destroy the only record that a legacy
/// local account ever existed, and gain nothing.
pub const ACCOUNT_ID_UNTOUCHED: &[&str] = &["web_users"];

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    /// [`STATUS_ACTIVE`] or [`STATUS_UNCLAIMED`].
    pub status: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl Account {
    pub fn is_unclaimed(&self) -> bool {
        self.status == STATUS_UNCLAIMED
    }
}

/// A Matrix user ID bound to an account.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MatrixLink {
    pub mxid: String,
    pub account_id: String,
    pub linked_at: Option<DateTime<Utc>>,
}

/// The account id as the UUID WebAuthn calls a "user handle".
///
/// No extra column and no UUID dependency: an account id is `acct_` followed
/// by exactly the 32 hex characters of 16 random bytes, which is the same
/// shape a UUID's simple form has. The two representations are the same
/// number, so this is a total, reversible mapping rather than a lookup that
/// could go stale.
///
/// The handle is stored inside the credential on the authenticator, so this
/// bijection is what lets a discoverable login identify an account with no
/// username typed and no server-side guess.
pub fn account_id_to_handle(account_id: &str) -> Option<uuid::Uuid> {
    uuid::Uuid::parse_str(account_id.strip_prefix("acct_")?).ok()
}

/// The inverse of [`account_id_to_handle`].
pub fn handle_to_account_id(handle: &uuid::Uuid) -> String {
    format!("acct_{}", handle.simple())
}

/// The display name to give an account provisioned for `mxid`.
///
/// Purely cosmetic — the localpart is what a human recognises, and nothing
/// authorises against it.
pub fn display_name_for_mxid(mxid: &str) -> String {
    mxid.trim_start_matches('@')
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(mxid)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use sqlx::{Row, SqlitePool};
    use std::collections::BTreeSet;

    #[test]
    fn test_display_name_is_the_localpart() {
        assert_eq!(display_name_for_mxid("@evan:mbm.one"), "evan");
        assert_eq!(display_name_for_mxid("@a:b.c"), "a");
    }

    /// A malformed MXID must still produce something renderable rather than an
    /// empty string, because this lands in a UI label.
    #[test]
    fn test_display_name_survives_a_malformed_mxid() {
        assert_eq!(display_name_for_mxid("nonsense"), "nonsense");
        assert_eq!(display_name_for_mxid("@"), "@");
        assert_eq!(display_name_for_mxid("@:server"), "@:server");
    }

    /// A passkey stores the handle, so this mapping has to survive a round trip
    /// exactly — a lossy one would let a login resolve to the wrong account, or
    /// to none at all.
    #[test]
    fn test_account_handle_round_trips() {
        let id = "acct_26ab01c0f4c19b7d854f46dca154ee89";
        let handle = account_id_to_handle(id).expect("a real account id must map to a handle");
        assert_eq!(handle_to_account_id(&handle), id);
    }

    /// Anything that is not a real account id must fail closed rather than
    /// producing some other account's handle.
    #[test]
    fn test_malformed_account_ids_have_no_handle() {
        for bad in [
            "26ab01c0f4c19b7d854f46dca154ee89", // no prefix
            "acct_",                            // no body
            "acct_notactuallyhex",
            "@evan:mbm.one", // a legacy owner string
            "",
        ] {
            assert!(
                account_id_to_handle(bad).is_none(),
                "{bad} must not map to a handle"
            );
        }
    }

    /// Every table in the schema that has `column`.
    async fn tables_with_column(pool: &SqlitePool, column: &str) -> BTreeSet<String> {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_all(pool)
        .await
        .unwrap();

        let mut found = BTreeSet::new();
        for table in tables {
            let has_column = sqlx::query(&format!("PRAGMA table_info({table})"))
                .fetch_all(pool)
                .await
                .unwrap()
                .iter()
                .any(|row| row.get::<String, _>("name") == column);
            if has_column {
                found.insert(table);
            }
        }
        found
    }

    fn declared(lists: &[&[&str]]) -> BTreeSet<String> {
        lists
            .iter()
            .flat_map(|list| list.iter())
            .map(|name| name.to_string())
            .collect()
    }

    async fn migrated_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    /// The list a merge walks must be derived from the schema, not remembered.
    ///
    /// It fell three tables behind before this existed, so a merge silently
    /// stranded a user's finance share, their audit ledger and their reaction
    /// targets on an account it then deleted. A new `owner_id` table must now be
    /// classified as following a merge or explicitly not, or this fails.
    #[tokio::test]
    async fn test_owned_tables_and_exceptions_match_the_schema() {
        let pool = migrated_pool().await;
        let in_schema = tables_with_column(&pool, "owner_id").await;
        let classified = declared(&[OWNED_TABLES, OWNER_ID_MERGED_SEPARATELY]);

        let unclassified: Vec<_> = in_schema.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "these tables carry owner_id but no list classifies them: {unclassified:?}"
        );

        let phantom: Vec<_> = classified.difference(&in_schema).collect();
        assert!(
            phantom.is_empty(),
            "these tables are classified but are not in the schema: {phantom:?}"
        );

        for table in OWNER_ID_MERGED_SEPARATELY {
            assert!(
                !OWNED_TABLES.contains(table),
                "{table} cannot both follow a merge and be excluded from one"
            );
        }
    }

    /// Same rule for the other ownership column. An unclassified table here leaves
    /// a row pointing at an account id that the merge is about to delete.
    #[tokio::test]
    async fn test_account_id_tables_are_all_classified() {
        let pool = migrated_pool().await;
        let in_schema = tables_with_column(&pool, "account_id").await;
        let classified = declared(&[
            ACCOUNT_ID_FOLLOWS,
            ACCOUNT_ID_DISCARDED,
            ACCOUNT_ID_UNTOUCHED,
        ]);

        let unclassified: Vec<_> = in_schema.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "these tables carry account_id but no list classifies them: {unclassified:?}"
        );

        let phantom: Vec<_> = classified.difference(&in_schema).collect();
        assert!(
            phantom.is_empty(),
            "these tables are classified but are not in the schema: {phantom:?}"
        );
    }
}
