use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::run::AgentRunId;
use super::AgentError;
use crate::domain::accounts::AccountId;

/// Default time-to-live for a sensitive action human-in-the-loop approval.
pub const APPROVAL_TTL_HOURS: i64 = 1;

/// Unique identifier for an interactive human approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalId(Uuid);

impl ApprovalId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn into_inner(self) -> Uuid {
        self.0
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for ApprovalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "appr_{}", self.0.simple())
    }
}

impl From<Uuid> for ApprovalId {
    fn from(u: Uuid) -> Self {
        Self(u)
    }
}

/// Resolution status of an interactive approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn is_resolved(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

impl fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ApprovalStatus {
    type Err = AgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "expired" => Ok(Self::Expired),
            other => Err(AgentError::InvalidStatus(other.to_string())),
        }
    }
}

/// An interactive human approval request for sensitive tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    pub run_id: AgentRunId,
    pub account_id: AccountId,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub status: ApprovalStatus,
    pub expires_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ApprovalRequest {
    pub fn new(
        run_id: AgentRunId,
        account_id: AccountId,
        tool_name: impl Into<String>,
        tool_input: serde_json::Value,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: ApprovalId::generate(),
            run_id,
            account_id,
            tool_name: tool_name.into(),
            tool_input,
            status: ApprovalStatus::Pending,
            expires_at: now + Duration::hours(APPROVAL_TTL_HOURS),
            resolved_at: None,
            created_at: now,
        }
    }

    /// Whether this approval request is past its expiry.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.status == ApprovalStatus::Pending && now >= self.expires_at
    }

    /// Approves the request.
    pub fn approve(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if self.is_expired(now) {
            self.status = ApprovalStatus::Expired;
            return Err(AgentError::ApprovalExpired);
        }
        if self.status.is_resolved() {
            return Err(AgentError::ApprovalAlreadyResolved);
        }
        self.status = ApprovalStatus::Approved;
        self.resolved_at = Some(now);
        Ok(())
    }

    /// Rejects the request.
    pub fn reject(&mut self, now: DateTime<Utc>) -> Result<(), AgentError> {
        if self.is_expired(now) {
            self.status = ApprovalStatus::Expired;
            return Err(AgentError::ApprovalExpired);
        }
        if self.status.is_resolved() {
            return Err(AgentError::ApprovalAlreadyResolved);
        }
        self.status = ApprovalStatus::Rejected;
        self.resolved_at = Some(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_request_lifecycle_and_expiry() {
        let now = Utc::now();
        let account = AccountId::from_seed("test-agent");
        let run_id = AgentRunId::generate();
        let mut req = ApprovalRequest::new(
            run_id,
            account,
            "portfolio_execute_trade",
            serde_json::json!({"symbol": "AAPL", "quantity": 10}),
            now,
        );

        assert_eq!(req.status, ApprovalStatus::Pending);
        assert!(!req.is_expired(now));

        // Expired after 1 hour + 1 second
        let future = now + Duration::hours(1) + Duration::seconds(1);
        assert!(req.is_expired(future));

        // Approval fails if expired
        let err_exp = req.approve(future);
        assert_eq!(err_exp, Err(AgentError::ApprovalExpired));
        assert_eq!(req.status, ApprovalStatus::Expired);

        // Fresh request approves cleanly
        let mut fresh = ApprovalRequest::new(
            run_id,
            account,
            "portfolio_execute_trade",
            serde_json::json!({}),
            now,
        );
        assert!(fresh.approve(now + Duration::minutes(10)).is_ok());
        assert_eq!(fresh.status, ApprovalStatus::Approved);
        assert!(fresh.resolved_at.is_some());

        // Cannot double resolve
        assert_eq!(
            fresh.reject(now + Duration::minutes(15)),
            Err(AgentError::ApprovalAlreadyResolved)
        );
    }
}
