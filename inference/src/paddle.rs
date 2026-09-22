use std::path::Path;

/// Whether every ONNX model OCR needs is present on disk.
///
/// Task 2 replaces this file wholesale with the module moved from the backend;
/// this stub exists so `/health` can be tested before the models arrive.
pub fn models_present(_dir: &Path) -> bool {
    false
}
