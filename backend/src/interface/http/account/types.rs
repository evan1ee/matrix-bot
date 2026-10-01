//! Request and response shapes shared by the account endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct PasskeySummary {
    pub id: String,
    pub label: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionSummary {
    /// The session's storage key. Safe to expose — it is a hash, not the
    /// cookie, so it cannot be replayed.
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub user_agent: Option<String>,
    /// Whether this is the session making the request.
    pub current: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    /// Shown exactly once. The server keeps only hashes.
    pub codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenamePasskeyRequest {
    pub label: String,
}

// One row of the account's security log, as the web UI reads it.
//
// The domain event also carries the owning account; the wire shape does not,
// because the caller can only ever list their own.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecurityEvent {
    pub id: i64,
    pub kind: String,
    pub detail: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<&crate::domain::accounts::SecurityEvent> for SecurityEvent {
    fn from(ev: &crate::domain::accounts::SecurityEvent) -> Self {
        Self {
            id: ev.id,
            kind: ev.kind.clone(),
            detail: ev.detail.clone(),
            created_at: ev.created_at,
        }
    }
}
