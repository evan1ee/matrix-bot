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

impl From<crate::infrastructure::inference::ocr::OcrScan> for OcrResult {
    fn from(scan: crate::infrastructure::inference::ocr::OcrScan) -> Self {
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
#[derive(Debug, Clone)]
pub struct OcrConfig {
    pub enabled: bool,
    pub base_url: Option<String>,
}

impl Default for OcrConfig {
    fn default() -> Self {
        // `#[derive(Default)]` would give `enabled: false`, which silently
        // disables `/ocr/scan` for anyone constructing this via `..Default::default()`
        // -- the actual deployment default is enabled, matching `OCR_ENABLED=true`
        // in `.env.example`; `base_url` presence is what actually gates the feature.
        Self {
            enabled: true,
            base_url: None,
        }
    }
}
