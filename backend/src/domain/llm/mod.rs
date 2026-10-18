use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiUserConfig {
    pub owner_id: String,
    pub api_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub model_name: Option<String>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub max_history_turns: Option<i64>,
    pub updated_at: String,
    #[serde(default)]
    pub has_api_key: bool,
}

impl AiUserConfig {
    pub fn redacted(mut self) -> Self {
        self.has_api_key = self
            .api_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        self.api_key = None;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyAiKeyInput {
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
}

pub fn validate_ai_base_url(url_str: &str, allow_private: bool) -> Result<(), &'static str> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.len() > 500 {
        return Err("API base URL exceeds 500 characters");
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| "API base URL is not a valid URL")?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("API base URL must use http or https scheme");
    }

    let host_str = parsed
        .host_str()
        .ok_or("API base URL must contain a valid host")?;
    let host_lower = host_str.to_ascii_lowercase();

    let ip = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => Some(std::net::IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Some(std::net::IpAddr::V6(ip)),
        _ => None,
    };

    if host_lower == "169.254.169.254"
        || host_lower.starts_with("169.254.")
        || host_lower.starts_with("fe80:")
        || host_lower.starts_with("[fe80:")
        || ip.is_some_and(is_link_local_ip)
    {
        return Err("API base URL cannot target link-local metadata addresses");
    }

    if !allow_private {
        if host_lower.ends_with(".internal")
            || host_lower.ends_with(".local")
            || host_lower == "localhost"
            || host_lower.ends_with(".localhost")
            || host_lower == "localtest.me"
            || host_lower.ends_with(".localtest.me")
            || host_lower == "lvh.me"
            || host_lower.ends_with(".lvh.me")
        {
            return Err("API base URL cannot target internal or local domain names");
        }

        if host_lower.ends_with(".nip.io") || host_lower.ends_with(".sslip.io") {
            let prefix = host_lower
                .trim_end_matches(".nip.io")
                .trim_end_matches(".sslip.io");
            if let Ok(ip) = prefix.parse::<std::net::IpAddr>() {
                if is_disallowed_ip(ip) {
                    return Err("API base URL cannot target private or loopback IP addresses");
                }
            }
        }

        if let Some(ip) = ip {
            if is_disallowed_ip(ip) {
                return Err("API base URL cannot target private or loopback IP addresses");
            }
        } else if let Some(url::Host::Domain(domain)) = parsed.host() {
            use std::net::ToSocketAddrs;
            let port = parsed.port_or_known_default().unwrap_or(80);
            match (domain, port).to_socket_addrs() {
                Ok(addrs) => {
                    let mut count = 0;
                    for addr in addrs {
                        count += 1;
                        if is_disallowed_ip(addr.ip()) {
                            return Err(
                                "API base URL resolves to a private or loopback IP address",
                            );
                        }
                    }
                    if count == 0 {
                        return Err("API base URL host could not be resolved");
                    }
                }
                Err(_) => {
                    return Err("API base URL host could not be resolved");
                }
            }
        }
    }

    Ok(())
}

fn is_link_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4()) {
                if v4.is_link_local() {
                    return true;
                }
            }
            (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

pub fn is_disallowed_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped().or_else(|| v6.to_ipv4()) {
                if is_disallowed_ip(std::net::IpAddr::V4(v4)) {
                    return true;
                }
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

impl VerifyAiKeyInput {
    pub fn validate(&self, allow_private: bool) -> Result<(), &'static str> {
        if let Some(ref url_str) = self.api_base_url {
            validate_ai_base_url(url_str, allow_private)?;
        }
        if let Some(ref key) = self.api_key {
            if key.len() > 500 {
                return Err("API key exceeds 500 characters");
            }
        }
        if let Some(ref model) = self.model_name {
            if model.len() > 500 {
                return Err("Model name exceeds 500 characters");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyAiKeyResponse {
    pub valid: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateAiConfigInput {
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub max_history_turns: Option<i64>,
}

impl UpdateAiConfigInput {
    pub fn validate(&self, allow_private: bool) -> Result<(), &'static str> {
        if let Some(ref url_str) = self.api_base_url {
            validate_ai_base_url(url_str, allow_private)?;
        }
        if let Some(ref key) = self.api_key {
            if key.len() > 500 {
                return Err("API key exceeds 500 characters");
            }
        }
        if let Some(ref model) = self.model_name {
            if model.len() > 500 {
                return Err("Model name exceeds 500 characters");
            }
        }
        if let Some(ref prompt) = self.system_prompt {
            if prompt.len() > 2000 {
                return Err("System prompt exceeds 2000 characters");
            }
        }
        if let Some(temp) = self.temperature {
            if !(0.0..=2.0).contains(&temp) {
                return Err("Temperature must be between 0.0 and 2.0");
            }
        }
        if let Some(tokens) = self.max_tokens {
            if tokens <= 0 || tokens > 32768 {
                return Err("Max tokens must be between 1 and 32768");
            }
        }
        if let Some(turns) = self.max_history_turns {
            if !(1..=100).contains(&turns) {
                return Err("Max history turns must be between 1 and 100");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConversation {
    pub id: String,
    pub owner_id: String,
    pub room_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatRequest {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub room_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatResponse {
    pub conversation_id: String,
    pub message: String,
    pub model_used: String,
    pub tool_calls_executed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConversationWithMessages {
    pub conversation: AiConversation,
    pub messages: Vec<AiMessage>,
}

/// How one tool call of a turn ended, as shown to a watching client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStepOutcome {
    Ok,
    Failed,
    NeedsConfirmation,
}

impl ToolStepOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::NeedsConfirmation => "needs_confirmation",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            "needs_confirmation" => Some(Self::NeedsConfirmation),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_step_outcome_round_trips_through_its_string_form() {
        for outcome in [
            ToolStepOutcome::Ok,
            ToolStepOutcome::Failed,
            ToolStepOutcome::NeedsConfirmation,
        ] {
            assert_eq!(ToolStepOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(ToolStepOutcome::parse("x"), None);
    }

    #[test]
    fn test_validate_api_base_url() {
        let valid = UpdateAiConfigInput {
            api_base_url: Some("https://api.openai.com/v1".to_string()),
            ..Default::default()
        };
        assert!(valid.validate(false).is_ok());

        let invalid_scheme = UpdateAiConfigInput {
            api_base_url: Some("ftp://example.com".to_string()),
            ..Default::default()
        };
        assert_eq!(
            invalid_scheme.validate(false).unwrap_err(),
            "API base URL must use http or https scheme"
        );

        let ssrf_metadata = UpdateAiConfigInput {
            api_base_url: Some("http://169.254.169.254/latest/meta-data".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_metadata.validate(false).unwrap_err(),
            "API base URL cannot target link-local metadata addresses"
        );

        let ssrf_loopback = UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1:8000/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_loopback.validate(false).unwrap_err(),
            "API base URL cannot target private or loopback IP addresses"
        );

        let ssrf_localhost = UpdateAiConfigInput {
            api_base_url: Some("http://localhost:11434/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_localhost.validate(false).unwrap_err(),
            "API base URL cannot target internal or local domain names"
        );

        let ssrf_internal = UpdateAiConfigInput {
            api_base_url: Some("http://service.internal/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_internal.validate(false).unwrap_err(),
            "API base URL cannot target internal or local domain names"
        );

        let ssrf_dns = UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1.nip.io:8000/v1".to_string()),
            ..Default::default()
        };
        assert!(ssrf_dns
            .validate(false)
            .unwrap_err()
            .contains("private or loopback IP address"));

        let ssrf_localtest = UpdateAiConfigInput {
            api_base_url: Some("http://localtest.me:8000/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_localtest.validate(false).unwrap_err(),
            "API base URL cannot target internal or local domain names"
        );

        let ssrf_mapped_v6 = UpdateAiConfigInput {
            api_base_url: Some("http://[::ffff:127.0.0.1]:8000/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_mapped_v6.validate(false).unwrap_err(),
            "API base URL cannot target private or loopback IP addresses"
        );

        let ssrf_mapped_metadata = UpdateAiConfigInput {
            api_base_url: Some("http://[::ffff:169.254.169.254]/latest/meta-data".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_mapped_metadata.validate(true).unwrap_err(),
            "API base URL cannot target link-local metadata addresses"
        );

        let ssrf_ula_v6 = UpdateAiConfigInput {
            api_base_url: Some("http://[fd00::1]:8000/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_ula_v6.validate(false).unwrap_err(),
            "API base URL cannot target private or loopback IP addresses"
        );

        let unresolvable_dns = UpdateAiConfigInput {
            api_base_url: Some(format!("https://{}.example.com/v1", "a".repeat(64))),
            ..Default::default()
        };
        assert_eq!(
            unresolvable_dns.validate(false).unwrap_err(),
            "API base URL host could not be resolved"
        );
    }

    #[test]
    fn test_validate_max_history_turns() {
        let valid = UpdateAiConfigInput {
            max_history_turns: Some(10),
            ..Default::default()
        };
        assert!(valid.validate(false).is_ok());

        let too_low = UpdateAiConfigInput {
            max_history_turns: Some(0),
            ..Default::default()
        };
        assert_eq!(
            too_low.validate(false).unwrap_err(),
            "Max history turns must be between 1 and 100"
        );

        let too_high = UpdateAiConfigInput {
            max_history_turns: Some(101),
            ..Default::default()
        };
        assert_eq!(
            too_high.validate(false).unwrap_err(),
            "Max history turns must be between 1 and 100"
        );
    }
}
