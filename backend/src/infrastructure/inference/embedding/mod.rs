//! Embedding provider traits and clients for vector generation.
//!
//! Exposes `EmbeddingProvider` with implementations for local ONNX inference,
//! remote OpenAI-compatible cloud endpoints, and a deterministic mock provider.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::domain::knowledge::search::DEFAULT_EMBEDDING_MODEL;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("Embedding provider is not configured")]
    NotConfigured,
    #[error("Embedding provider is unreachable: {0}")]
    Unreachable(String),
    #[error("Embedding provider returned {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error("Empty input provided for embedding")]
    EmptyInput,
    #[error("Invalid embedding response: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
    fn dimension(&self) -> usize {
        384
    }
}

/// Request payload matching OpenAI / local ONNX /v1/embeddings format.
#[derive(Debug, Serialize)]
struct EmbeddingApiRequest<'a> {
    input: &'a [String],
    model: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingApiResponse {
    data: Vec<EmbeddingApiData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingApiData {
    index: usize,
    embedding: Vec<f32>,
}

/// HTTP client for `matrix-bot-inference` local ONNX embedding endpoint.
#[derive(Debug, Clone)]
pub struct LocalOnnxEmbeddingClient {
    base_url: Option<String>,
    model: String,
    http: reqwest::Client,
}

impl LocalOnnxEmbeddingClient {
    pub fn new(base_url: Option<String>, model: Option<String>) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        let model = model.unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self {
            base_url,
            model,
            http,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.base_url.is_some()
    }
}

#[async_trait]
impl EmbeddingProvider for LocalOnnxEmbeddingClient {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        let base = self
            .base_url
            .as_ref()
            .ok_or(EmbeddingError::NotConfigured)?;
        let url = format!("{base}/v1/embeddings");

        let res = self
            .http
            .post(&url)
            .json(&EmbeddingApiRequest {
                input: texts,
                model: &self.model,
            })
            .send()
            .await
            .map_err(|e| EmbeddingError::Unreachable(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status().as_u16();
            let message = res.text().await.unwrap_or_default();
            return Err(EmbeddingError::Upstream { status, message });
        }

        let mut body: EmbeddingApiResponse = res
            .json()
            .await
            .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

        body.data.sort_by_key(|d| d.index);
        Ok(body.data.into_iter().map(|d| d.embedding).collect())
    }
}

/// HTTP client for OpenAI-compatible cloud embedding endpoints.
#[derive(Debug, Clone)]
pub struct CloudEmbeddingClient {
    endpoint: Option<String>,
    api_key: Option<String>,
    model: String,
    http: reqwest::Client,
}

impl CloudEmbeddingClient {
    pub fn new(endpoint: Option<String>, api_key: Option<String>, model: Option<String>) -> Self {
        let endpoint = endpoint
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        let api_key = api_key.filter(|k| !k.trim().is_empty());
        let model = model.unwrap_or_else(|| "text-embedding-3-small".to_string());
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self {
            endpoint,
            api_key,
            model,
            http,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for CloudEmbeddingClient {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or(EmbeddingError::NotConfigured)?;
        let url = if endpoint.ends_with("/embeddings") {
            endpoint.clone()
        } else {
            format!("{endpoint}/embeddings")
        };

        let mut req = self.http.post(&url).json(&EmbeddingApiRequest {
            input: texts,
            model: &self.model,
        });

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let res = req
            .send()
            .await
            .map_err(|e| EmbeddingError::Unreachable(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status().as_u16();
            let message = res.text().await.unwrap_or_default();
            return Err(EmbeddingError::Upstream { status, message });
        }

        let mut body: EmbeddingApiResponse = res
            .json()
            .await
            .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

        body.data.sort_by_key(|d| d.index);
        Ok(body.data.into_iter().map(|d| d.embedding).collect())
    }
}

/// Deterministic in-memory embedding provider for tests and offline usage.
#[derive(Debug, Clone, Default)]
pub struct MockEmbeddingProvider {
    dimension: usize,
}

impl MockEmbeddingProvider {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl EmbeddingProvider for MockEmbeddingProvider {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        let dim = if self.dimension == 0 {
            384
        } else {
            self.dimension
        };
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let mut vec = vec![0.0f32; dim];
            for (i, byte) in text.as_bytes().iter().enumerate() {
                vec[i % dim] += *byte as f32 / 255.0;
            }
            let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            } else {
                vec[0] = 1.0;
            }
            results.push(vec);
        }

        Ok(results)
    }

    fn dimension(&self) -> usize {
        if self.dimension == 0 {
            384
        } else {
            self.dimension
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::test_support::spawn_server;
    use axum::{routing::post, Json, Router};

    #[tokio::test]
    async fn test_mock_embedding_provider_normalizes_and_dimension() {
        let provider = MockEmbeddingProvider::new(384);
        assert_eq!(provider.dimension(), 384);

        let texts = vec!["hello world".to_string(), "second brain".to_string()];
        let embeddings = provider.embed(&texts).await.expect("embeddings");
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 384);
        assert_eq!(embeddings[1].len(), 384);

        // Vector magnitude is 1.0 (L2 normalized)
        let norm: f32 = embeddings[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[tokio::test]
    async fn test_local_onnx_client_parses_response() {
        let app = Router::new().route(
            "/v1/embeddings",
            post(|Json(req): Json<serde_json::Value>| async move {
                let inputs = req["input"].as_array().unwrap();
                let data: Vec<_> = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        serde_json::json!({
                            "object": "embedding",
                            "index": i,
                            "embedding": vec![0.5f32; 384]
                        })
                    })
                    .collect();

                Json(serde_json::json!({
                    "object": "list",
                    "data": data,
                    "model": "multilingual-e5-small",
                    "usage": { "prompt_tokens": 10, "total_tokens": 10 }
                }))
            }),
        );

        let base_url = spawn_server(app).await;
        let client = LocalOnnxEmbeddingClient::new(Some(base_url), None);
        assert!(client.is_configured());

        let result = client
            .embed(&["note title".to_string()])
            .await
            .expect("embed call");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 384);
        assert_eq!(result[0][0], 0.5);
    }

    #[tokio::test]
    async fn test_local_onnx_client_not_configured() {
        let client = LocalOnnxEmbeddingClient::new(None, None);
        assert!(!client.is_configured());

        let err = client.embed(&["test".to_string()]).await.unwrap_err();
        assert!(matches!(err, EmbeddingError::NotConfigured));
    }
}
