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
//! resolves directly. `owner_id` is a native `UUID` column with a foreign key to
//! `accounts(id)` (migration `0002_account_uuid`), compared for equality, so
//! every ownership predicate is unchanged. On the wire and in the audit hash
//! it is still the `acct_<32 hex>` text form.
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

use crate::domain::accounts::STATUS_UNCLAIMED;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    /// `active`, or [`STATUS_UNCLAIMED`].
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
