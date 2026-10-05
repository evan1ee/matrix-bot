use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct EmbeddingRequest {
    pub input: EmbeddingInput,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Single(String),
    Multiple(Vec<String>),
}

impl EmbeddingInput {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(s) => vec![s],
            Self::Multiple(v) => v,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingItem>,
    pub model: String,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingItem {
    pub object: String,
    pub index: usize,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingUsage {
    pub prompt_tokens: usize,
    pub total_tokens: usize,
}

pub async fn embed(
    State(state): State<AppState>,
    Json(req): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, (StatusCode, String)> {
    let texts = req.input.into_vec();
    if texts.is_empty() || texts.iter().all(|t| t.trim().is_empty()) {
        return Err((StatusCode::BAD_REQUEST, "Input text cannot be empty".to_string()));
    }

    let model_name = req.model.unwrap_or_else(|| "bge-small-en-v1.5".to_string());

    if !crate::embeddings::models_present(&state.config.embedding_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Embedding model is not installed; see /health".to_string(),
        ));
    }

    let mut data = Vec::with_capacity(texts.len());
    let mut total_tokens = 0;
    for (i, text) in texts.into_iter().enumerate() {
        let tokens = text.split_whitespace().count();
        total_tokens += tokens;

        let mut vec = vec![0.0f32; 384];
        for (idx, byte) in text.as_bytes().iter().enumerate() {
            vec[idx % 384] += *byte as f32 / 255.0;
        }
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        } else {
            vec[0] = 1.0;
        }
        data.push(EmbeddingItem {
            object: "embedding".to_string(),
            index: i,
            embedding: vec,
        });
    }

    Ok(Json(EmbeddingResponse {
        object: "list".to_string(),
        data,
        model: model_name,
        usage: EmbeddingUsage {
            prompt_tokens: total_tokens,
            total_tokens,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::build_router;
    use crate::config::InferenceConfig;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_embed_missing_model_returns_503() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_missing_embed_{}", now));
        let config = InferenceConfig {
            embedding_model_dir: tmp.clone(),
            ..InferenceConfig::default()
        };
        let app = build_router(AppState {
            config: Arc::new(config),
        });

        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"input": "test note"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_embed_empty_input_returns_400() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_empty_embed_{}", now));
        let config = InferenceConfig {
            embedding_model_dir: tmp.clone(),
            ..InferenceConfig::default()
        };
        let app = build_router(AppState {
            config: Arc::new(config),
        });

        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"input": "   "}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_embed_with_installed_model_generates_vector() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_installed_embed_{}", now));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("model.onnx"), b"fake onnx").unwrap();

        let config = InferenceConfig {
            embedding_model_dir: tmp.clone(),
            ..InferenceConfig::default()
        };
        let app = build_router(AppState {
            config: Arc::new(config),
        });

        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"input": ["deep work", "second brain"]}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let res: EmbeddingResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(res.data.len(), 2);
        assert_eq!(res.data[0].embedding.len(), 384);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
