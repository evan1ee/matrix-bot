use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::embeddings::{EmbedError, MODEL_NAME};

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

/// Most inputs one request may carry. The backend sends one note's chunks at a
/// time, so this bounds a runaway caller, not a real workload.
pub const MAX_INPUTS: usize = 128;
/// Longest single input, in characters. 512 tokens is a few thousand
/// characters at most; this is generous headroom, not a tuned limit.
pub const MAX_INPUT_CHARS: usize = 20_000;

pub async fn embed(
    State(state): State<AppState>,
    Json(req): Json<EmbeddingRequest>,
) -> Result<Json<EmbeddingResponse>, (StatusCode, String)> {
    // Checked before anything else, even before the model is installed:
    // answering a request for another model with this model's vectors would
    // silently corrupt an index built against the one it asked for.
    if let Some(requested) = req.model.as_deref().filter(|m| *m != MODEL_NAME) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Unsupported model '{requested}'; this server runs '{MODEL_NAME}'"),
        ));
    }

    let texts = req.input.into_vec();
    if texts.is_empty() || texts.iter().all(|t| t.trim().is_empty()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Input text cannot be empty".to_string(),
        ));
    }
    if texts.len() > MAX_INPUTS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("At most {MAX_INPUTS} inputs per request"),
        ));
    }

    if texts.iter().any(|t| t.chars().count() > MAX_INPUT_CHARS) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Each input is limited to {MAX_INPUT_CHARS} characters"),
        ));
    }

    let embedded = state.embedder.embed(texts).await.map_err(|err| match err {
        EmbedError::NotInstalled => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Embedding model is not installed; see /health".to_string(),
        ),
        EmbedError::Load(detail) => {
            tracing::error!("Embedding model failed to load: {detail}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Embedding model is installed but could not be loaded".to_string(),
            )
        }
        EmbedError::Inference(detail) => {
            tracing::error!("Embedding inference failed: {detail}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Embedding failed".to_string(),
            )
        }
    })?;

    let data = embedded
        .vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| EmbeddingItem {
            object: "embedding".to_string(),
            index,
            embedding,
        })
        .collect();

    Ok(Json(EmbeddingResponse {
        object: "list".to_string(),
        data,
        model: MODEL_NAME.to_string(),
        usage: EmbeddingUsage {
            prompt_tokens: embedded.tokens,
            total_tokens: embedded.tokens,
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_missing_embed_{}", now));
        let config = InferenceConfig {
            embedding_model_dir: tmp.clone(),
            ..InferenceConfig::default()
        };
        let app = build_router(AppState::new(Arc::new(config)));

        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"input": "test note"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_embed_empty_input_returns_400() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_empty_embed_{}", now));
        let config = InferenceConfig {
            embedding_model_dir: tmp.clone(),
            ..InferenceConfig::default()
        };
        let app = build_router(AppState::new(Arc::new(config)));

        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"input": "   "}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("test_embed_{tag}_{now}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn post_embeddings(dir: &std::path::Path, body: &str) -> StatusCode {
        let config = InferenceConfig {
            embedding_model_dir: dir.to_path_buf(),
            ..InferenceConfig::default()
        };
        let app = build_router(AppState::new(Arc::new(config)));
        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn test_embed_for_a_different_model_returns_400() {
        let dir = scratch_dir("other_model");

        let status =
            post_embeddings(&dir, r#"{"input": "hi", "model": "bge-small-en-v1.5"}"#).await;

        // A client asking for another model must be told so even before the
        // model is installed: silently answering with different vectors would
        // corrupt a vector index built against the model it asked for.
        assert_eq!(status, StatusCode::BAD_REQUEST);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_embed_with_only_the_onnx_file_returns_503() {
        let dir = scratch_dir("no_tokenizer");
        std::fs::write(dir.join("model.onnx"), b"x").unwrap();

        let status = post_embeddings(&dir, r#"{"input": "hi"}"#).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_embed_with_corrupt_model_files_returns_503_not_a_panic() {
        let dir = scratch_dir("corrupt");
        std::fs::write(dir.join("model.onnx"), b"not an onnx graph").unwrap();
        std::fs::write(dir.join("tokenizer.json"), b"{ not a tokenizer").unwrap();

        let status = post_embeddings(&dir, r#"{"input": "hi"}"#).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_embed_with_too_many_inputs_returns_400() {
        let dir = scratch_dir("too_many");
        let inputs = vec!["x"; MAX_INPUTS + 1];
        let body = serde_json::json!({ "input": inputs }).to_string();

        let status = post_embeddings(&dir, &body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_embed_with_an_oversized_input_returns_400() {
        let dir = scratch_dir("oversized");
        let body = serde_json::json!({ "input": "x".repeat(MAX_INPUT_CHARS + 1) }).to_string();

        let status = post_embeddings(&dir, &body).await;

        // The model reads 512 tokens at most; tokenizing megabytes first would
        // only burn CPU before truncating it away.
        assert_eq!(status, StatusCode::BAD_REQUEST);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    #[ignore = "needs the real model files in EMBEDDING_MODEL_DIR"]
    async fn test_embed_through_the_router_with_the_real_model() {
        let dir = std::path::PathBuf::from(
            std::env::var("EMBEDDING_MODEL_DIR").expect("set EMBEDDING_MODEL_DIR"),
        );
        let app = build_router(AppState::new(Arc::new(InferenceConfig {
            embedding_model_dir: dir,
            ..InferenceConfig::default()
        })));
        let req = Request::post("/v1/embeddings")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"input": ["query: 猫", "passage: The cat sits on the mat"], "model": "multilingual-e5-small"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let res: EmbeddingResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(res.model, "multilingual-e5-small");
        assert_eq!(res.data.len(), 2);
        assert_eq!(res.data[1].index, 1);
        for item in &res.data {
            assert_eq!(item.embedding.len(), 384);
            let norm = item.embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4);
        }
        assert!(res.usage.prompt_tokens > 0);
    }
}
