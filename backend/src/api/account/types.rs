//! Request and response shapes shared by the account endpoints.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::accounts::MergePreview;
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
pub struct LinkCodeResponse {
    pub code: String,
    pub expires_in_seconds: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnlinkRequest {
    pub mxid: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PendingMerge {
    pub from_account: String,
    pub preview: MergePreview,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfirmMergeRequest {
    pub from_account: String,
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

