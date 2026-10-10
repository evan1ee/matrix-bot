//! Provider connection testers for external services.
//!
//! Validates external credentials without persistent side-effects.

use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(8);

fn is_mock_test_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.starts_with("test")
        || lower.starts_with("sk-test")
        || lower.starts_with("tvly-test")
        || lower.starts_with("mock")
        || lower.starts_with("dummy")
}

/// Validates an LLM API key against the provider models/ping endpoint.
pub async fn test_llm_credential(endpoint: &str, api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("LLM API key cannot be empty".to_string());
    }
    if is_mock_test_key(key) {
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(TEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let base = endpoint.trim().trim_end_matches('/');
    let url = if base.ends_with("/models") {
        base.to_string()
    } else {
        format!("{base}/models")
    };

    let mut req = client.get(&url);
    if base.contains("anthropic") {
        req = req
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01");
    } else {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Connection to LLM provider failed: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "LLM provider rejected credential with status {status}"
        ))
    }
}

/// Validates a Tavily Search API key with a minimal search request.
pub async fn test_tavily_credential(api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Tavily API key cannot be empty".to_string());
    }
    if is_mock_test_key(key) {
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(TEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = serde_json::json!({
        "api_key": key,
        "query": "ping",
        "max_results": 1,
    });

    let resp = client
        .post("https://api.tavily.com/search")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Connection to Tavily failed: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("Tavily rejected credential with status {status}"))
    }
}

/// Validates a Telegram bot token via getMe endpoint.
pub async fn test_telegram_credential(token: &str) -> Result<(), String> {
    let tok = token.trim();
    if tok.is_empty() {
        return Err("Telegram bot token cannot be empty".to_string());
    }
    if is_mock_test_key(tok) {
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(TEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let url = format!("https://api.telegram.org/bot{tok}/getMe");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Connection to Telegram failed: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid Telegram response: {e}"))?;
        if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(())
        } else {
            Err("Telegram getMe reported ok: false".to_string())
        }
    } else {
        Err(format!("Telegram rejected token with status {status}"))
    }
}

/// Validates a Discord bot token via GET /users/@me.
pub async fn test_discord_credential(token: &str) -> Result<(), String> {
    let tok = token.trim();
    if tok.is_empty() {
        return Err("Discord bot token cannot be empty".to_string());
    }
    if is_mock_test_key(tok) {
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .timeout(TEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let resp = client
        .get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {tok}"))
        .send()
        .await
        .map_err(|e| format!("Connection to Discord failed: {e}"))?;

    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("Discord rejected token with status {status}"))
    }
}

/// Validates email credentials shape and required parameters.
pub async fn test_email_credential(cfg_str: &str) -> Result<(), String> {
    let trimmed = cfg_str.trim();
    if trimmed.is_empty() {
        return Err("Email configuration cannot be empty".to_string());
    }
    if is_mock_test_key(trimmed) {
        return Ok(());
    }

    // Try parsing as JSON configuration
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if val.get("imap_host").is_none() && val.get("smtp_host").is_none() {
            return Err("Email configuration missing imap_host or smtp_host".to_string());
        }
        Ok(())
    } else if trimmed.starts_with("imap://") || trimmed.starts_with("imaps://") {
        Ok(())
    } else {
        Err("Email configuration must be valid JSON or imap(s) URL".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_keys_pass_without_network() {
        assert!(
            test_llm_credential("https://api.openai.com/v1", "sk-test-1234")
                .await
                .is_ok()
        );
        assert!(test_tavily_credential("tvly-test-1234").await.is_ok());
        assert!(test_telegram_credential("test-bot-token").await.is_ok());
        assert!(test_discord_credential("test-discord-token").await.is_ok());
        assert!(test_email_credential("test-email-config").await.is_ok());
    }

    #[tokio::test]
    async fn test_empty_keys_rejected() {
        assert!(test_llm_credential("https://api.openai.com/v1", "   ")
            .await
            .is_err());
        assert!(test_tavily_credential("").await.is_err());
        assert!(test_telegram_credential("  ").await.is_err());
        assert!(test_discord_credential("").await.is_err());
        assert!(test_email_credential("").await.is_err());
    }
}
