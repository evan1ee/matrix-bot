use std::path::PathBuf;
use thiserror::Error;

/// A single detected text block from an image with its confidence score.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrTextBlock {
    pub text: String,
    pub score: f32,
}

/// The aggregated result of OCR recognition across all detected blocks.
#[derive(Debug, Clone, Default)]
pub struct OcrResult {
    pub blocks: Vec<OcrTextBlock>,
    pub combined_text: String,
}

impl OcrResult {
    pub fn new(blocks: Vec<OcrTextBlock>) -> Self {
        let combined_text = blocks
            .iter()
            .map(|b| b.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            blocks,
            combined_text,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.combined_text.trim().is_empty()
    }
}

/// Configuration for the OCR module.
#[derive(Debug, Clone)]
pub struct OcrConfig {
    pub enabled: bool,
    pub model_dir: PathBuf,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_dir: PathBuf::from("data/ocr_models"),
        }
    }
}

/// Errors that can occur during OCR processing.
#[derive(Error, Debug)]
pub enum OcrError {
    #[error("OCR feature is disabled in configuration")]
    Disabled,
    #[error("Failed to decode image bytes: {0}")]
    ImageDecode(String),
    #[error("Missing OCR model file: {0}")]
    MissingModel(String),
    #[error("Failed to download OCR model: {0}")]
    ModelDownload(String),
    #[error("PaddleOCR inference error: {0}")]
    Inference(String),
    #[error("Worker thread execution error: {0}")]
    Join(String),
}
