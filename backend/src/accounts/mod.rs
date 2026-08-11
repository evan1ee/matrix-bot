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

pub mod repository;

pub use repository::AccountRepository;

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
    "finance_categories",
    "user_settings",
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

    /// The merge in Phase 3 and the backfill in Phase 1 must agree on the list
    /// of tables that carry ownership, or one of them silently misses rows.
    #[test]
    fn test_owned_tables_covers_every_ownership_carrying_table() {
        assert_eq!(OWNED_TABLES.len(), 8);
        for table in ["todos", "reminders", "transactions", "user_settings"] {
            assert!(OWNED_TABLES.contains(&table), "{table} must be re-pointed on merge");
        }
    }
}
