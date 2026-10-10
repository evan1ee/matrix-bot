//! Provider connection testers for external services.
//!
//! Validates external credentials without persistent side-effects, enforcing
//! SSRF protections, disabled HTTP redirects, and URL-redacted error reporting.

use std::time::Duration;

use crate::domain::secrets::SecretProvider;
use crate::infrastructure::web::ssrf::is_safe_target_url;

const TEST_TIMEOUT: Duration = Duration::from_secs(8);

/// Concrete HTTP connectivity tester for external provider credentials.
#[derive(Debug, Clone)]
pub struct CredentialTester {
    openai_base_url: String,
    anthropic_base_url: String,
    tavily_url: String,
    allow_private: bool,
}

impl Default for CredentialTester {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialTester {
    /// Creates a production tester targeting canonical provider endpoints with SSRF guard enabled.
    pub fn new() -> Self {
        Self {
            openai_base_url: "https://api.openai.com/v1".to_string(),
            anthropic_base_url: "https://api.anthropic.com/v1".to_string(),
            tavily_url: "https://api.tavily.com/search".to_string(),
            allow_private: false,
        }
    }

    /// Creates a tester pointed at a local loopback mock server for unit/integration tests.
    #[cfg(test)]
    pub fn for_test(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            openai_base_url: format!("{base}/v1"),
            anthropic_base_url: format!("{base}/v1"),
            tavily_url: format!("{base}/search"),
            allow_private: true,
        }
    }

    /// Probes the external provider for `provider` using `api_key`.
    ///
    /// Returns:
    /// - `Ok(Some(()))` when a live connectivity probe succeeded (`"healthy"`).
    /// - `Ok(None)` when the provider has no standalone probe endpoint (`"unknown"`).
    /// - `Err(reason)` when the probe failed or the credential was rejected (`"unhealthy"`).
    pub async fn test_provider(
        &self,
        provider: SecretProvider,
        api_key: &str,
    ) -> Result<Option<()>, String> {
        match provider {
            SecretProvider::OpenAi => self
                .test_llm(&self.openai_base_url, false, api_key)
                .await
                .map(Some),
            SecretProvider::Anthropic => self
                .test_llm(&self.anthropic_base_url, true, api_key)
                .await
                .map(Some),
            SecretProvider::Tavily => self.test_tavily(&self.tavily_url, api_key).await.map(Some),
            SecretProvider::Stt | SecretProvider::Tts | SecretProvider::Generic => Ok(None),
        }
    }

    async fn test_llm(
        &self,
        endpoint: &str,
        is_anthropic: bool,
        api_key: &str,
    ) -> Result<(), String> {
        let key = api_key.trim();
        if key.is_empty() {
            return Err("LLM API key cannot be empty".to_string());
        }

        if !self.allow_private && !is_safe_target_url(endpoint).await {
            return Err("Invalid provider endpoint: disallowed or private target URL".to_string());
        }

        let client = reqwest::Client::builder()
            .timeout(TEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e.without_url()))?;

        let base = endpoint.trim().trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{base}/models")
        };

        let mut req = client.get(&url);
        if is_anthropic {
            req = req
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Connection to LLM provider failed: {}", e.without_url()))?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(format!(
                "LLM provider rejected credential with status {status}"
            ))
        }
    }

    async fn test_tavily(&self, endpoint: &str, api_key: &str) -> Result<(), String> {
        let key = api_key.trim();
        if key.is_empty() {
            return Err("Tavily API key cannot be empty".to_string());
        }

        if !self.allow_private && !is_safe_target_url(endpoint).await {
            return Err("Invalid provider endpoint: disallowed or private target URL".to_string());
        }

        let client = reqwest::Client::builder()
            .timeout(TEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e.without_url()))?;

        let payload = serde_json::json!({
            "api_key": key,
            "query": "ping",
            "max_results": 1,
        });

        let resp = client
            .post(endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Connection to Tavily failed: {}", e.without_url()))?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(format!("Tavily rejected credential with status {status}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_one_shot_http(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let resp = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_local_mock_server_success_and_rejection() {
        let ok_url = spawn_one_shot_http("HTTP/1.1 200 OK").await;
        let tester = CredentialTester::for_test(&ok_url);
        assert_eq!(
            tester
                .test_provider(SecretProvider::OpenAi, "sk-live-key-1234567890")
                .await
                .unwrap(),
            Some(())
        );

        let unauthorized_url = spawn_one_shot_http("HTTP/1.1 401 Unauthorized").await;
        let bad_tester = CredentialTester::for_test(&unauthorized_url);
        let err = bad_tester
            .test_provider(SecretProvider::Anthropic, "sk-ant-invalid-key")
            .await
            .unwrap_err();
        assert!(err.contains("401"));

        // Providers without standalone probe endpoints return Ok(None)
        assert_eq!(
            tester
                .test_provider(SecretProvider::Generic, "any-key")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn test_empty_keys_and_ssrf_rejected() {
        let prod_tester = CredentialTester::new();
        assert!(prod_tester
            .test_provider(SecretProvider::OpenAi, "   ")
            .await
            .is_err());
        assert!(prod_tester
            .test_provider(SecretProvider::Tavily, "")
            .await
            .is_err());

        // Production tester refuses loopback/private URLs via SSRF guard
        let mut private_disallowed = CredentialTester::for_test("http://127.0.0.1:8080");
        private_disallowed.allow_private = false;
        let err = private_disallowed
            .test_provider(SecretProvider::OpenAi, "sk-valid-looking-key")
            .await
            .unwrap_err();
        assert!(err.contains("Invalid provider endpoint"), "{err}");
    }
}
