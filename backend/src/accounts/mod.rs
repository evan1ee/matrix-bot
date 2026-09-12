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

pub use credentials::CredentialRepository;
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

/// Every table that carries an `owner_id`, in the order a merge rewrites them.
///
/// Kept here rather than inline at the call site because two separate
/// operations need the exact same list — the Phase 1 backfill and the Phase 3
/// account merge — and a table missing from one of them is a silent data leak
/// rather than a compile error.
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
];

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

    /// The merge in Phase 3 and the backfill in Phase 1 must agree on the list
    /// of tables that carry ownership, or one of them silently misses rows.
    #[test]
    fn test_owned_tables_covers_every_ownership_carrying_table() {
        assert_eq!(OWNED_TABLES.len(), 20);
        for table in [
            "todos",
            "reminders",
            "transactions",
            "user_settings",
            "bookmarks",
            "habits",
            "life_day_logs",
            "footprints",
            "ai_conversations",
        ] {
            assert!(
                OWNED_TABLES.contains(&table),
                "{table} must be re-pointed on merge"
            );
        }
    }
}
