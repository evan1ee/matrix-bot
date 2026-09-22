/// A single detected text block with its confidence score.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrTextBlock {
    pub text: String,
    pub score: f32,
}

/// OCR output as the business layer consumes it.
#[derive(Debug, Clone, Default)]
pub struct OcrResult {
    pub blocks: Vec<OcrTextBlock>,
    pub combined_text: String,
}

impl OcrResult {
    pub fn is_empty(&self) -> bool {
        self.combined_text.trim().is_empty()
    }
}

impl From<crate::services::ocr::OcrScan> for OcrResult {
    fn from(scan: crate::services::ocr::OcrScan) -> Self {
        Self {
            combined_text: scan.text,
            blocks: scan
                .blocks
                .into_iter()
                .map(|b| OcrTextBlock {
                    text: b.text,
                    score: b.score,
                })
                .collect(),
        }
    }
}

/// Configuration for the OCR module.
///
/// `base_url` absent means OCR is not available in this deployment; the
/// feature degrades and nothing else is affected.
#[derive(Debug, Clone, Default)]
pub struct OcrConfig {
    pub enabled: bool,
    pub base_url: Option<String>,
}
