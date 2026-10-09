//! The ONNX session and tokenizer behind `/v1/embeddings`.

use std::path::Path;

use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use thiserror::Error;
use tokenizers::{Tokenizer, TruncationParams};

use super::pooling::mean_pool_normalise;
use super::{EMBEDDING_DIM, MODEL_FILE, TOKENIZER_FILE};

/// `multilingual-e5` was trained on sequences of at most 512 tokens; longer
/// input is truncated, not rejected, because a chunk's head still says what
/// the chunk is about.
const MAX_TOKENS: usize = 512;
/// Texts per ONNX call. Padding makes cost grow with the longest text in a
/// call, so a bounded batch keeps one long chunk from inflating a whole
/// request, and bounds the activation memory of a 128-input request.
const BATCH_SIZE: usize = 16;
/// XLM-R's `<pad>` id, used only if the tokenizer does not name its own.
const FALLBACK_PAD_ID: i64 = 1;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("embedding model is not installed")]
    NotInstalled,
    #[error("embedding model could not be loaded: {0}")]
    Load(String),
    #[error("embedding inference failed: {0}")]
    Inference(String),
}

/// One vector per input text, plus how many tokens the model actually read.
pub struct Embedded {
    pub vectors: Vec<Vec<f32>>,
    pub tokens: usize,
}

pub struct EmbeddingModel {
    session: Session,
    tokenizer: Tokenizer,
    wants_token_type_ids: bool,
    pad_id: i64,
}

impl EmbeddingModel {
    /// Loads the graph and tokenizer from `dir`. Blocking: call it from
    /// `spawn_blocking`.
    pub fn load(dir: &Path) -> Result<Self, EmbedError> {
        let load = |what: &str, e: &dyn std::fmt::Display| EmbedError::Load(format!("{what}: {e}"));

        let mut tokenizer =
            Tokenizer::from_file(dir.join(TOKENIZER_FILE)).map_err(|e| load("tokenizer", &e))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(|e| load("tokenizer truncation", &e))?;
        // Padding is done per call below, to the longest text in that call.
        tokenizer.with_padding(None);
        let pad_id = tokenizer
            .token_to_id("<pad>")
            .map_or(FALLBACK_PAD_ID, i64::from);

        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(intra_op_threads()))
            .and_then(|b| b.commit_from_file(dir.join(MODEL_FILE)))
            .map_err(|e| load("onnx graph", &e))?;
        let wants_token_type_ids = session.inputs.iter().any(|i| i.name == "token_type_ids");

        Ok(Self {
            session,
            tokenizer,
            wants_token_type_ids,
            pad_id,
        })
    }

    /// Embeds `texts` in order. Blocking: call it from `spawn_blocking`.
    pub fn embed(&mut self, texts: &[String]) -> Result<Embedded, EmbedError> {
        let mut vectors = Vec::with_capacity(texts.len());
        let mut tokens = 0;
        for batch in texts.chunks(BATCH_SIZE) {
            let (batch_vectors, batch_tokens) = self.embed_batch(batch)?;
            vectors.extend(batch_vectors);
            tokens += batch_tokens;
        }
        Ok(Embedded { vectors, tokens })
    }

    fn embed_batch(&mut self, texts: &[String]) -> Result<(Vec<Vec<f32>>, usize), EmbedError> {
        let infer = |e: &dyn std::fmt::Display| EmbedError::Inference(e.to_string());

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| infer(&e))?;
        let batch = encodings.len();
        let seq = encodings.iter().map(|e| e.len()).max().unwrap_or(0);
        if seq == 0 {
            return Err(EmbedError::Inference("tokenizer produced no tokens".into()));
        }

        let mut ids = vec![self.pad_id; batch * seq];
        let mut mask = vec![0i64; batch * seq];
        for (row, encoding) in encodings.iter().enumerate() {
            let tokens = encoding.get_ids().iter().zip(encoding.get_attention_mask());
            for (col, (&id, &attended)) in tokens.enumerate() {
                ids[row * seq + col] = i64::from(id);
                mask[row * seq + col] = i64::from(attended);
            }
        }

        let shape = [batch, seq];
        let tensor = |data: Vec<i64>| Tensor::from_array((shape, data.into_boxed_slice()));
        let mut inputs = ort::inputs![
            "input_ids" => tensor(ids).map_err(|e| infer(&e))?,
            "attention_mask" => tensor(mask.clone()).map_err(|e| infer(&e))?,
        ];
        if self.wants_token_type_ids {
            let zeros = tensor(vec![0; batch * seq]).map_err(|e| infer(&e))?;
            inputs.push(("token_type_ids".into(), SessionInputValue::from(zeros)));
        }

        let outputs = self.session.run(inputs).map_err(|e| infer(&e))?;
        let first = outputs
            .values()
            .next()
            .ok_or_else(|| EmbedError::Inference("model returned no outputs".to_string()))?;
        let (out_shape, hidden) = first.try_extract_tensor::<f32>().map_err(|e| infer(&e))?;
        let expected_len = batch * seq * EMBEDDING_DIM;
        if out_shape.len() != 3
            || out_shape[2] != EMBEDDING_DIM as i64
            || hidden.len() != expected_len
        {
            return Err(EmbedError::Inference(format!(
                "model output has shape {out_shape:?}, expected [{batch}, {seq}, {EMBEDDING_DIM}]"
            )));
        }

        let vectors = hidden
            .chunks_exact(seq * EMBEDDING_DIM)
            .zip(mask.chunks_exact(seq))
            .map(|(row_hidden, row_mask)| {
                mean_pool_normalise(row_hidden, row_mask, EMBEDDING_DIM)
                    .ok_or_else(|| EmbedError::Inference("text produced an unusable vector".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let attended = mask.iter().filter(|&&m| m != 0).count();
        Ok((vectors, attended))
    }
}

/// Intra-op threads for the one shared session: half the logical cores, so a
/// batch uses real parallelism without starving the OCR, STT and TTS requests
/// this process also serves.
fn intra_op_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}
