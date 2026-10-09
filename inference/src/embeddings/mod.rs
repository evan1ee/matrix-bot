use std::path::Path;

mod engine;
mod model;
mod pooling;
mod provision;

pub use engine::Embedder;
pub use model::EmbedError;
pub use provision::ensure_models;

/// The only model this server runs. Clients must ask for it by this name (or
/// not name one): vectors from different models are not comparable.
pub const MODEL_NAME: &str = "multilingual-e5-small";
/// Width of the vectors the model produces, and of the backend's
/// `vector(384)` column.
pub const EMBEDDING_DIM: usize = 384;
pub const MODEL_FILE: &str = "model.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";

/// Checks whether both the graph and its tokenizer are installed.
pub fn models_present(dir: &Path) -> bool {
    dir.join(MODEL_FILE).is_file() && dir.join(TOKENIZER_FILE).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_present_detects_files() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("test_embed_models_{}", now));
        std::fs::create_dir_all(&tmp).unwrap();

        assert!(!models_present(&tmp));

        std::fs::write(tmp.join("model.onnx"), b"fake onnx").unwrap();
        assert!(
            !models_present(&tmp),
            "the graph alone cannot embed text: the tokenizer is required too"
        );

        std::fs::write(tmp.join("tokenizer.json"), b"{}").unwrap();
        assert!(models_present(&tmp));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
