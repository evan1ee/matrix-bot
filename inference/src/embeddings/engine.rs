//! Lazily loads the embedding model once and serialises access to it.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, OnceCell};

use super::model::{EmbedError, Embedded, EmbeddingModel};
use super::models_present;

/// The process-wide embedding model, shared through `AppState`.
///
/// Loaded on first use rather than at boot because provisioning downloads the
/// files in the background: the server must start (and answer `/health`)
/// before they exist, and a failed load is not cached, so the next request
/// retries once the files are fixed or finish downloading.
///
/// One session behind one lock, on purpose: `ort` needs exclusive access to a
/// session per run, and a second copy of a ~120 MB graph buys nothing for a
/// single-user server whose parallelism is already inside the batch (see
/// `intra_op_threads`). The lock is a fair FIFO queue, so concurrent requests
/// wait their turn rather than oversubscribing the CPU.
pub struct Embedder {
    dir: PathBuf,
    model: OnceCell<Arc<Mutex<EmbeddingModel>>>,
}

impl Embedder {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            model: OnceCell::new(),
        }
    }

    pub async fn embed(&self, texts: Vec<String>) -> Result<Embedded, EmbedError> {
        if !models_present(&self.dir) {
            return Err(EmbedError::NotInstalled);
        }

        let model = self
            .model
            .get_or_try_init(|| async {
                let dir = self.dir.clone();
                let loaded = tokio::task::spawn_blocking(move || EmbeddingModel::load(&dir))
                    .await
                    .map_err(|e| EmbedError::Load(format!("loader task failed: {e}")))??;
                Ok::<_, EmbedError>(Arc::new(Mutex::new(loaded)))
            })
            .await?;

        let mut guard = Arc::clone(model).lock_owned().await;
        tokio::task::spawn_blocking(move || guard.embed(&texts))
            .await
            .map_err(|e| EmbedError::Inference(format!("inference task failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::{EMBEDDING_DIM, MODEL_FILE, TOKENIZER_FILE};

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    /// Runs the real model. Needs `model.onnx` and `tokenizer.json` in
    /// `EMBEDDING_MODEL_DIR` (the files the inference server provisions), so
    /// it is opt-in: `cargo test -p matrix-bot-inference -- --ignored embeddings`.
    #[tokio::test]
    #[ignore = "needs the real model files in EMBEDDING_MODEL_DIR"]
    async fn a_translation_is_closer_than_an_unrelated_sentence() {
        let dir = std::env::var("EMBEDDING_MODEL_DIR")
            .map(PathBuf::from)
            .expect("set EMBEDDING_MODEL_DIR to the directory holding the model");
        assert!(dir.join(MODEL_FILE).is_file() && dir.join(TOKENIZER_FILE).is_file());
        let embedder = Embedder::new(dir);

        let embedded = embedder
            .embed(vec![
                "猫坐在垫子上".to_string(),
                "The cat sits on the mat".to_string(),
                "Quarterly tax filing".to_string(),
            ])
            .await
            .expect("the real model should embed three short texts");

        let [zh, en, unrelated] = &embedded.vectors[..] else {
            panic!("expected three vectors, got {}", embedded.vectors.len());
        };
        for v in [zh, en, unrelated] {
            assert_eq!(v.len(), EMBEDDING_DIM);
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4, "not unit length: {norm}");
        }
        assert!(embedded.tokens > 0);
        let translation = cosine(zh, en);
        let unrelated_pair = cosine(en, unrelated);
        assert!(
            translation > unrelated_pair,
            "translation {translation} should beat unrelated {unrelated_pair}"
        );
    }

    #[tokio::test]
    #[ignore = "needs the real model files in EMBEDDING_MODEL_DIR"]
    async fn batching_and_padding_do_not_change_a_vector() {
        let dir = std::env::var("EMBEDDING_MODEL_DIR")
            .map(PathBuf::from)
            .expect("set EMBEDDING_MODEL_DIR to the directory holding the model");
        let embedder = Embedder::new(dir);
        let probe = "The cat sits on the mat".to_string();
        let long = "word ".repeat(2_000);

        let alone = embedder.embed(vec![probe.clone()]).await.unwrap();
        // 17 inputs cross the 16-text batch boundary; the long ones force
        // padding of the short probe and exercise the 512-token truncation.
        let mut batch = vec![long.clone(), probe.clone()];
        batch.extend((0..15).map(|i| format!("filler {i}")));
        batch.push(long);
        let batched = embedder.embed(batch).await.unwrap();

        assert_eq!(batched.vectors.len(), 18);
        let similarity = cosine(&alone.vectors[0], &batched.vectors[1]);
        // Not exactly 1.0: the model is dynamically int8-quantised, which
        // scales activations per tensor across the whole batch, so a vector
        // drifts by ~0.3% with its batch-mates (measured flat from 20 to 500
        // tokens of padding). Padding leaking into the mean would instead
        // grow with pad length and fall well below this.
        assert!(
            similarity > 0.99,
            "padding changed the vector: {similarity}"
        );
        assert!(batched.tokens > alone.tokens);
    }

    #[tokio::test]
    async fn an_uninstalled_model_is_reported_not_loaded() {
        let embedder = Embedder::new(std::env::temp_dir().join("embedder-never-installed"));

        let result = embedder.embed(vec!["hi".to_string()]).await;

        assert!(matches!(result, Err(EmbedError::NotInstalled)));
    }
}
