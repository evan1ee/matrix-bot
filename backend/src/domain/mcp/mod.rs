use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An MCP personal access token for an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToken {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub token_hash: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl McpToken {
    /// Whether this token has expired.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|exp| now > exp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_mcp_token_expiration() {
        let now = Utc::now();
        let token = McpToken {
            id: "tok_1".to_string(),
            account_id: "acct_1".to_string(),
            name: "test".to_string(),
            token_hash: "hash".to_string(),
            prefix: "pref".to_string(),
            created_at: now,
            last_used_at: None,
            expires_at: Some(now - Duration::seconds(1)),
        };
        assert!(token.is_expired(now));

        let unexpired = McpToken {
            expires_at: Some(now + Duration::days(1)),
            ..token
        };
        assert!(!unexpired.is_expired(now));
    }
}
