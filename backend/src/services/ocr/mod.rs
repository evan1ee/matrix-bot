use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// How long a single OCR call may take. PaddleOCR on a CPU is seconds, not
/// milliseconds, so this is generous; it exists to bound a hung connection,
/// not to police inference speed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OcrScanBlock {
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OcrScan {
    pub text: String,
    pub blocks: Vec<OcrScanBlock>,
}

#[derive(Debug, Error)]
pub enum OcrServiceError {
    #[error("OCR service is not configured")]
    NotConfigured,
    #[error("OCR service is unreachable: {0}")]
    Unreachable(String),
    #[error("OCR service returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

/// HTTP client for the inference server's OCR endpoint.
///
/// An absent `base_url` is a normal state, not an error condition: it means
/// OCR is not configured for this deployment, and every call reports
/// `NotConfigured` so the caller can degrade that one feature.
#[derive(Debug, Clone)]
pub struct OcrClient {
    base_url: Option<String>,
    http: reqwest::Client,
}

impl OcrClient {
    pub fn new(base_url: Option<String>) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());

        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self { base_url, http }
    }

    pub fn is_configured(&self) -> bool {
        self.base_url.is_some()
    }

    /// Sends the image bytes untouched and returns the recognised text.
    pub async fn scan(&self, image_bytes: Vec<u8>) -> Result<OcrScan, OcrServiceError> {
        let base = self
            .base_url
            .as_ref()
            .ok_or(OcrServiceError::NotConfigured)?;

        let response = self
            .http
            .post(format!("{}/ocr", base))
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(image_bytes)
            .send()
            .await
            .map_err(|e| {
                warn!("OCR service request failed: {}", e);
                OcrServiceError::Unreachable(e.to_string())
            })?;

        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(OcrServiceError::Upstream {
                status: status.as_u16(),
                message,
            });
        }

        response
            .json::<OcrScan>()
            .await
            .map_err(|e| OcrServiceError::Upstream {
                status: status.as_u16(),
                message: format!("Malformed OCR response: {}", e),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::json;

    /// Starts a throwaway server on an ephemeral port and returns its base URL.
    /// Uses axum and tokio, both already backend dependencies, rather than
    /// adding a mock-HTTP crate.
    async fn spawn_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("ephemeral port should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have an address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_an_unconfigured_client_reports_not_configured() {
        let client = OcrClient::new(None);

        assert!(!client.is_configured());
        assert!(matches!(
            client.scan(vec![1, 2, 3]).await,
            Err(OcrServiceError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn test_a_successful_scan_is_parsed() {
        let app = Router::new().route(
            "/ocr",
            post(|| async {
                Json(json!({
                    "text": "SHAKE SHACK\nTotal $16.50",
                    "blocks": [
                        { "text": "SHAKE SHACK", "score": 0.98 },
                        { "text": "Total $16.50", "score": 0.99 }
                    ]
                }))
            }),
        );
        let base = spawn_server(app).await;
        let client = OcrClient::new(Some(base));

        let scan = client
            .scan(vec![1, 2, 3])
            .await
            .expect("scan should succeed");

        assert_eq!(scan.text, "SHAKE SHACK\nTotal $16.50");
        assert_eq!(scan.blocks.len(), 2);
        assert_eq!(scan.blocks[0].text, "SHAKE SHACK");
    }

    #[tokio::test]
    async fn test_an_upstream_error_carries_its_status_and_message() {
        let app = Router::new().route(
            "/ocr",
            post(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "OCR models are not installed on this inference server",
                )
            }),
        );
        let base = spawn_server(app).await;
        let client = OcrClient::new(Some(base));

        let err = client
            .scan(vec![1, 2, 3])
            .await
            .expect_err("503 should be an error");

        match err {
            OcrServiceError::Upstream { status, message } => {
                assert_eq!(status, 503);
                assert!(message.contains("not installed"));
            }
            other => panic!("expected an upstream error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_an_unreachable_service_is_not_a_panic() {
        // Port 1 on loopback refuses immediately rather than hanging.
        let client = OcrClient::new(Some("http://127.0.0.1:1".to_string()));

        let err = client
            .scan(vec![1, 2, 3])
            .await
            .expect_err("a refused connection should error");

        assert!(matches!(err, OcrServiceError::Unreachable(_)));
    }

    #[tokio::test]
    async fn test_a_trailing_slash_in_the_base_url_does_not_double_up() {
        let app = Router::new().route(
            "/ocr",
            post(|| async { Json(json!({ "text": "ok", "blocks": [] })) }),
        );
        let base = spawn_server(app).await;
        let client = OcrClient::new(Some(format!("{}/", base)));

        let scan = client
            .scan(vec![1])
            .await
            .expect("a trailing slash should be tolerated");

        assert_eq!(scan.text, "ok");
    }
}
