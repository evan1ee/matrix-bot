pub mod paddle;

#[allow(unused_imports)]
pub use paddle::{
    ensure_models, models_present, resolve_model_dir, run_ocr, run_ocr_sync, OcrError, OcrResult,
    OcrTextBlock, CLS_MODEL_NAME, DET_MODEL_NAME, REC_MODEL_NAME,
};
