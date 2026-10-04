//! Pure accounts domain types, invariants, and business rules.
//!
//! Owns the `AccountId` identity type, account status, link rules,
//! security event types, session invariants, and the `WEB_ONLY` Matrix command ceiling.
//! Depends only on std, chrono, uuid, serde, and thiserror.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod merge;
pub use merge::{MergeOutcome, MergePreview};

mod principal;
pub use principal::{cookie_value, Credential, Principal};
mod recovery;
pub use recovery::{
    restriction_expiry, MaturedRecovery, RecoveryOutcome, RECOVERY_CODE_COUNT,
    RECOVERY_DELAY_HOURS, RECOVERY_RESTRICTION_HOURS, RECOVERY_WINDOW_HOURS,
};
mod session;
pub use session::{
    hash_token, DeleteOutcome, Session, StoredCredential, ABSOLUTE_TIMEOUT_DAYS, COOKIE_NAME,
    IDLE_TIMEOUT_DAYS, STEP_UP_WINDOW_MINUTES,
};

/// The single owner identifier used across all application tables.
/// Wraps a native `uuid::Uuid`, formatting and serialising as `acct_<32 hex chars>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(uuid::Uuid);

impl AccountId {
    /// Creates a new `AccountId`.
    pub fn new(id: impl Into<AccountId>) -> Self {
        id.into()
    }

    /// Creates a new `AccountId` from a UUID.
    pub fn from_uuid(id: uuid::Uuid) -> Self {
        Self(id)
    }

    /// Generates a new random `AccountId` using UUID v4.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// Returns a reference to the underlying UUID.
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }

    /// Returns the underlying UUID.
    pub fn uuid(&self) -> uuid::Uuid {
        self.0
    }

    /// Consumes the wrapper and returns the UUID.
    pub fn into_inner(self) -> uuid::Uuid {
        self.0
    }

    /// Converts this account ID to a UUID WebAuthn user handle.
    pub fn to_handle(&self) -> uuid::Uuid {
        self.0
    }

    /// Converts a UUID WebAuthn user handle back into an `AccountId`.
    pub fn from_handle(handle: &uuid::Uuid) -> Self {
        Self(*handle)
    }

    /// Deterministically generates an `AccountId` from a seed name. Test-only:
    /// production ids come from the database, never from a name.
    #[cfg(test)]
    pub fn from_seed(seed: &str) -> Self {
        Self(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_DNS,
            seed.as_bytes(),
        ))
    }
}

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "acct_{}", self.0.simple())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid account id format: expected acct_<uuid> or raw uuid")]
pub struct ParseAccountIdError;

impl std::str::FromStr for AccountId {
    type Err = ParseAccountIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let stripped = s.strip_prefix("acct_").unwrap_or(s);
        let u = uuid::Uuid::parse_str(stripped).map_err(|_| ParseAccountIdError)?;
        Ok(Self(u))
    }
}

impl std::ops::Deref for AccountId {
    type Target = uuid::Uuid;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<uuid::Uuid> for AccountId {
    fn as_ref(&self) -> &uuid::Uuid {
        &self.0
    }
}

impl From<uuid::Uuid> for AccountId {
    fn from(u: uuid::Uuid) -> Self {
        Self(u)
    }
}

impl From<&AccountId> for AccountId {
    fn from(id: &AccountId) -> Self {
        *id
    }
}

impl From<&uuid::Uuid> for AccountId {
    fn from(u: &uuid::Uuid) -> Self {
        Self(*u)
    }
}

impl From<AccountId> for uuid::Uuid {
    fn from(id: AccountId) -> Self {
        id.0
    }
}

impl Serialize for AccountId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("acct_{}", self.0.simple()))
    }
}

impl<'de> Deserialize<'de> for AccountId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// An account that has a credential of its own.
pub const STATUS_ACTIVE: &str = "active";

/// An account auto-provisioned for an MXID, reachable only through that MXID
/// until someone claims it by registering a credential.
pub const STATUS_UNCLAIMED: &str = "unclaimed";

/// Every table that carries an `owner_id` and whose rows follow a merge.
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
    "bot_message_targets",
    "notes",
    "knowledge_embeddings",
];

/// Tables carrying `owner_id` that a merge moves through a dedicated routine.
pub const OWNER_ID_MERGED_SEPARATELY: &[&str] = &["portfolio_audit_log"];

/// Tables keyed by `account_id` whose rows follow the absorbed account.
/// jobs: pending work follows the account (handlers resolve their subject from the row).
pub const ACCOUNT_ID_FOLLOWS: &[&str] = &[
    "platform_identities",
    "webauthn_credentials",
    "security_events",
    "account_rooms",
    "jobs",
    // outbox history follows the account on merge (like jobs, R23)
    "outbox_events",
    "scheduled_agent_tasks",
    "agent_runs",
    "agent_approvals",
];

/// Tables keyed by `account_id` whose rows are destroyed rather than moved.
pub const ACCOUNT_ID_DISCARDED: &[&str] = &[
    "sessions",
    "recovery_codes",
    "recovery_requests",
    "mcp_tokens",
    "passkey_enrollment_tokens",
];

/// Tables carrying `account_id` that a merge deliberately leaves alone.
pub const ACCOUNT_ID_UNTOUCHED: &[&str] = &["web_users"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub display_name: String,
    pub status: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl Account {
    pub fn is_unclaimed(&self) -> bool {
        self.status == STATUS_UNCLAIMED
    }
}

/// A Matrix user ID bound to an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixLink {
    pub mxid: String,
    pub account_id: AccountId,
    pub linked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityEventKind {
    SignedIn,
    PasskeyAdded,
    PasskeyRemoved,
    PasskeyRenamed,
    MatrixLinked,
    MatrixUnlinked,
    IdentityLinked,
    IdentityUnlinked,
    AccountsMerged,
    RecoveryCodesIssued,
    RecoveryCodeUsed,
    RecoveryRequested,
    RecoveryCancelled,
    RecoveryCompleted,
    SessionsRevoked,
    FinancialDataExported,
}

impl SecurityEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SignedIn => "signed_in",
            Self::PasskeyAdded => "passkey_added",
            Self::PasskeyRemoved => "passkey_removed",
            Self::PasskeyRenamed => "passkey_renamed",
            Self::MatrixLinked => "matrix_linked",
            Self::MatrixUnlinked => "matrix_unlinked",
            Self::IdentityLinked => "identity_linked",
            Self::IdentityUnlinked => "identity_unlinked",
            Self::AccountsMerged => "accounts_merged",
            Self::RecoveryCodesIssued => "recovery_codes_issued",
            Self::RecoveryCodeUsed => "recovery_code_used",
            Self::RecoveryRequested => "recovery_requested",
            Self::RecoveryCancelled => "recovery_cancelled",
            Self::RecoveryCompleted => "recovery_completed",
            Self::SessionsRevoked => "sessions_revoked",
            Self::FinancialDataExported => "financial_data_exported",
        }
    }

    pub fn warrants_a_dm(&self) -> bool {
        !matches!(self, Self::SignedIn)
    }

    pub fn message(&self, detail: Option<&str>) -> String {
        let suffix = detail.map(|d| format!(" ({d})")).unwrap_or_default();
        match self {
            Self::SignedIn => format!("Signed in{suffix}."),
            Self::PasskeyAdded => format!("A passkey was added to your account{suffix}."),
            Self::PasskeyRemoved => format!("A passkey was removed from your account{suffix}."),
            Self::PasskeyRenamed => format!("A passkey was renamed{suffix}."),
            Self::MatrixLinked => format!("A Matrix account was linked{suffix}."),
            Self::MatrixUnlinked => format!("A Matrix account was unlinked{suffix}."),
            Self::IdentityLinked => format!("A chat identity was linked{suffix}."),
            Self::IdentityUnlinked => format!("A chat identity was unlinked{suffix}."),
            Self::AccountsMerged => format!("Existing data was merged into your account{suffix}."),
            Self::RecoveryCodesIssued => {
                format!("New recovery codes were generated{suffix}. Your old codes no longer work.")
            }
            Self::RecoveryCodeUsed => {
                format!("A recovery code was used to sign in{suffix}.")
            }
            Self::RecoveryRequested => format!(
                "Account recovery was requested{suffix}. It will not take effect for 24 hours — \
                 if this was not you, cancel it from any signed-in device now."
            ),
            Self::RecoveryCancelled => format!("Account recovery was cancelled{suffix}."),
            Self::RecoveryCompleted => format!("Account recovery completed{suffix}."),
            Self::SessionsRevoked => format!("Other sessions were signed out{suffix}."),
            Self::FinancialDataExported => format!("Financial data was exported{suffix}."),
        }
    }
}

impl std::str::FromStr for SecurityEventKind {
    type Err = AccountsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "signed_in" => Ok(Self::SignedIn),
            "passkey_added" => Ok(Self::PasskeyAdded),
            "passkey_removed" => Ok(Self::PasskeyRemoved),
            "passkey_renamed" => Ok(Self::PasskeyRenamed),
            "matrix_linked" => Ok(Self::MatrixLinked),
            "matrix_unlinked" => Ok(Self::MatrixUnlinked),
            "identity_linked" => Ok(Self::IdentityLinked),
            "identity_unlinked" => Ok(Self::IdentityUnlinked),
            "accounts_merged" => Ok(Self::AccountsMerged),
            "recovery_codes_issued" => Ok(Self::RecoveryCodesIssued),
            "recovery_code_used" => Ok(Self::RecoveryCodeUsed),
            "recovery_requested" => Ok(Self::RecoveryRequested),
            "recovery_cancelled" => Ok(Self::RecoveryCancelled),
            "recovery_completed" => Ok(Self::RecoveryCompleted),
            "sessions_revoked" => Ok(Self::SessionsRevoked),
            "financial_data_exported" => Ok(Self::FinancialDataExported),
            _ => Err(AccountsError::Validation(format!(
                "unknown security event kind: {s}"
            ))),
        }
    }
}

impl std::fmt::Display for SecurityEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityEvent {
    pub id: i64,
    pub account_id: AccountId,
    pub kind: String,
    pub detail: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Commands that exist on the web only, and what to say about each.
pub const WEB_ONLY: &[(&str, &str)] = &[
    ("export", "Exporting financial data"),
    ("sessions", "Managing signed-in devices"),
    ("recoverycodes", "Generating recovery codes"),
    (
        "unlink",
        "Unlinking passkeys, Matrix accounts and signed-in devices",
    ),
];

pub fn web_only_command_description(cmd: &str) -> Option<&'static str> {
    let clean = cmd.trim().trim_start_matches('!').to_lowercase();
    WEB_ONLY
        .iter()
        .find(|(name, _)| *name == clean)
        .map(|(_, desc)| *desc)
}

/// Accepts the recovery code however a human typed it — lower case, spaces, missing dash.
pub fn normalise_recovery_code(code: &str) -> String {
    let cleaned: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();

    if cleaned.len() == 10 {
        format!("{}-{}", &cleaned[..5], &cleaned[5..])
    } else {
        cleaned
    }
}

/// The display name to give an account provisioned for `mxid`.
pub fn display_name_for_mxid(mxid: &str) -> String {
    mxid.trim_start_matches('@')
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(mxid)
        .to_string()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AccountsError {
    #[error("Account not found")]
    AccountNotFound,
    #[error("That command is available on the web app only: {0}")]
    WebOnlyCommand(&'static str),
    #[error("An account that already has a passkey cannot add another via Matrix")]
    FirstPasskeyOnly,
    #[error("Send this command in a direct message")]
    NotADirectMessage,
    #[error("Invalid or expired code")]
    InvalidCode,
    #[error("This Matrix account is already linked to another account")]
    AlreadyLinked,
    #[error("Invalid recovery code")]
    InvalidRecoveryCode,
    #[error("Recovery is not yet matured")]
    RecoveryNotMatured,
    #[error("Validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_account_id_handle_round_trip() {
        let id: AccountId = "acct_26ab01c0f4c19b7d854f46dca154ee89".parse().unwrap();
        let handle = id.to_handle();
        assert_eq!(AccountId::from_handle(&handle), id);
        assert_eq!(id.to_string(), "acct_26ab01c0f4c19b7d854f46dca154ee89");
    }

    #[test]
    fn test_account_id_hyphenated_and_simple() {
        let simple: AccountId = "acct_26ab01c0f4c19b7d854f46dca154ee89".parse().unwrap();
        let hyphenated: AccountId = "acct_26ab01c0-f4c1-9b7d-854f-46dca154ee89".parse().unwrap();
        let raw: AccountId = "26ab01c0-f4c1-9b7d-854f-46dca154ee89".parse().unwrap();
        assert_eq!(simple, hyphenated);
        assert_eq!(simple, raw);
        assert_eq!(format!("{simple}"), "acct_26ab01c0f4c19b7d854f46dca154ee89");
    }

    /// Anything that is not a real account id must fail closed rather than
    /// producing some other account's handle (or the nil id). The bare-hex form
    /// is deliberately absent: `FromStr` accepts a raw UUID by design.
    #[test]
    fn test_malformed_account_ids_fail_parsing() {
        for bad in [
            "acct_",
            "acct_nothex",
            "acct_notactuallyhex",
            "@user:server",
            "@evan:mbm.one",
            "",
            "acct_too_short",
        ] {
            assert!(
                bad.parse::<AccountId>().is_err(),
                "{bad} must not parse to an account id"
            );
        }
    }

    #[test]
    fn test_generated_account_ids_are_unique_and_well_formed() {
        let ids: std::collections::HashSet<String> = (0..500)
            .map(|_| AccountId::generate().to_string())
            .collect();
        assert_eq!(ids.len(), 500, "generate repeated a value");

        let id = AccountId::generate();
        let text = id.to_string();
        let hex = text.strip_prefix("acct_").expect("acct_ prefix");
        assert_eq!(hex.len(), 32);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(text.parse::<AccountId>().unwrap(), id);
        assert_eq!(AccountId::from_handle(&id.to_handle()), id);
    }

    #[test]
    fn test_display_name_extraction() {
        assert_eq!(display_name_for_mxid("@evan:mbm.one"), "evan");
        assert_eq!(display_name_for_mxid("nonsense"), "nonsense");
        assert_eq!(display_name_for_mxid("@"), "@");
    }

    /// Routine sign-ins must not DM, or the channel becomes noise and the one
    /// message that matters — a recovery warning — gets ignored with the rest.
    #[test]
    fn test_only_meaningful_events_are_notified() {
        assert!(!SecurityEventKind::SignedIn.warrants_a_dm());

        for kind in [
            SecurityEventKind::PasskeyRemoved,
            SecurityEventKind::MatrixUnlinked,
            SecurityEventKind::RecoveryRequested,
            SecurityEventKind::FinancialDataExported,
        ] {
            assert!(kind.warrants_a_dm(), "{} must be announced", kind.as_str());
        }
    }

    /// The recovery warning has to tell someone what to do about it, because it
    /// is the only thing standing between a stolen Matrix account and a stolen
    /// local one.
    #[test]
    fn test_the_recovery_warning_says_how_to_stop_it() {
        let message = SecurityEventKind::RecoveryRequested.message(None);
        assert!(message.contains("24 hours"));
        assert!(message.contains("cancel"));
    }

    /// Every kind round-trips through its wire name and says something; the
    /// identity kinds are new, the Matrix ones stay so old rows still parse.
    #[test]
    fn test_identity_kinds_have_wire_names_and_messages() {
        for (kind, wire) in [
            (SecurityEventKind::IdentityLinked, "identity_linked"),
            (SecurityEventKind::IdentityUnlinked, "identity_unlinked"),
            (SecurityEventKind::MatrixLinked, "matrix_linked"),
            (SecurityEventKind::MatrixUnlinked, "matrix_unlinked"),
        ] {
            assert_eq!(kind.as_str(), wire);
            assert_eq!(wire.parse::<SecurityEventKind>().unwrap(), kind);
            assert!(kind.message(Some("telegram:1")).contains("telegram:1"));
        }
        assert!(SecurityEventKind::IdentityLinked.warrants_a_dm());
        assert!(SecurityEventKind::IdentityUnlinked.warrants_a_dm());
    }

    #[test]
    fn test_details_are_included_when_present() {
        let with = SecurityEventKind::MatrixLinked.message(Some("@evan:mbm.one"));
        assert!(with.contains("@evan:mbm.one"));

        let without = SecurityEventKind::MatrixLinked.message(None);
        assert!(!without.contains('('));
    }

    /// A malformed MXID must still produce something renderable rather than an
    /// empty string, because this lands in a UI label.
    #[test]
    fn test_display_name_survives_a_malformed_mxid() {
        assert_eq!(display_name_for_mxid("nonsense"), "nonsense");
        assert_eq!(display_name_for_mxid("@"), "@");
        assert_eq!(display_name_for_mxid("@:server"), "@:server");
        assert_eq!(display_name_for_mxid("@a:b.c"), "a");
    }

    #[test]
    fn test_web_only_command_gate() {
        assert!(web_only_command_description("export").is_some());
        assert!(web_only_command_description("!export").is_some());
        assert!(web_only_command_description("sessions").is_some());
        assert!(web_only_command_description("unlink").is_some());
        assert!(web_only_command_description("todo").is_none());
    }

    #[test]
    fn test_security_event_kind_round_trip() {
        let kind = SecurityEventKind::SignedIn;
        assert_eq!(kind.as_str(), "signed_in");
        assert_eq!("signed_in".parse::<SecurityEventKind>().unwrap(), kind);
    }

    fn session_fixture(now: DateTime<Utc>) -> Session {
        Session {
            id_hash: "hash".to_string(),
            account_id: AccountId::from_seed("test_user_1"),
            created_at: now,
            last_seen_at: now,
            last_verified_at: now,
            restricted_until: None,
            user_agent: None,
            ip: None,
            revoked_at: None,
        }
    }

    /// C8: these are the values browsers and OpenAPI already depend on. A
    /// change here logs every user out or widens step-up; it is never a refactor.
    #[test]
    fn live_security_constants_are_pinned() {
        assert_eq!(COOKIE_NAME, "mb_session");
        assert_eq!(STEP_UP_WINDOW_MINUTES, 5);
        assert_eq!((IDLE_TIMEOUT_DAYS, ABSOLUTE_TIMEOUT_DAYS), (30, 90));
        assert_eq!(
            (
                RECOVERY_DELAY_HOURS,
                RECOVERY_WINDOW_HOURS,
                RECOVERY_RESTRICTION_HOURS
            ),
            (24, 48, 72)
        );
        assert_eq!(RECOVERY_CODE_COUNT, 10);
    }

    /// Only the SHA-256 of a session cookie is stored (AGENTS.md section 7).
    #[test]
    fn hash_token_is_sha256_hex() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Moved from the legacy `test_step_up_window_boundary` onto the domain
    /// `Session`, keeping its just-inside / just-outside assertions, plus the
    /// whole-minute boundary.
    #[test]
    fn step_up_window_boundary() {
        let now = Utc::now();
        let session = Session {
            last_verified_at: now - Duration::minutes(STEP_UP_WINDOW_MINUTES)
                + Duration::seconds(1),
            ..session_fixture(now)
        };
        assert!(session.is_recently_verified(now), "just inside the window");

        let stale = Session {
            last_verified_at: now
                - Duration::minutes(STEP_UP_WINDOW_MINUTES)
                - Duration::seconds(1),
            ..session
        };
        assert!(!stale.is_recently_verified(now), "just outside the window");

        let s = |mins| Session {
            last_verified_at: now - Duration::minutes(mins),
            ..session_fixture(now)
        };
        assert!(s(4).is_recently_verified(now));
        assert!(!s(5).is_recently_verified(now));
        assert!(!s(6).is_recently_verified(now));
    }

    #[test]
    fn restriction_expiry_is_72_hours_out() {
        let now = Utc::now();
        assert_eq!(restriction_expiry(now), now + Duration::hours(72));
    }

    #[test]
    fn test_session_live_and_verified_invariants() {
        let now = Utc::now();
        let session = Session {
            id_hash: "hash".to_string(),
            account_id: AccountId::from_seed("test_user_1"),
            created_at: now - Duration::days(5),
            last_seen_at: now - Duration::minutes(10),
            // Inside the 5 minute step-up window (T12/X7: the old 5 minute
            // value sits exactly on the boundary of the live window).
            last_verified_at: now - Duration::minutes(1),
            restricted_until: None,
            user_agent: None,
            ip: None,
            revoked_at: None,
        };

        assert!(session.is_live(now));
        assert!(session.is_recently_verified(now));
        assert!(!session.is_restricted(now));

        // Revoked session is not live
        let mut revoked = session.clone();
        revoked.revoked_at = Some(now);
        assert!(!revoked.is_live(now));

        // Idle expired session (> 30 days) is not live
        let mut idle = session.clone();
        idle.last_seen_at = now - Duration::days(31);
        assert!(!idle.is_live(now));
    }
}
