use std::path::{Path, PathBuf};
use tracing::info;

use super::models::{OcrError, OcrResult, OcrTextBlock};

pub const DET_MODEL_NAME: &str = "ch_PP-OCRv4_det_infer.onnx";
pub const CLS_MODEL_NAME: &str = "ch_ppocr_mobile_v2.0_cls_infer.onnx";
pub const REC_MODEL_NAME: &str = "ch_PP-OCRv4_rec_infer.onnx";

const DET_MODEL_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_det_infer.onnx";
const CLS_MODEL_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv1/ch_ppocr_mobile_v2.0_cls_infer.onnx";
const REC_MODEL_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_rec_infer.onnx";

/// Resolves model directory, checking parent directory if running from backend subfolder.
pub fn resolve_model_dir(dir: &Path) -> PathBuf {
    if dir.exists() {
        dir.to_path_buf()
    } else if let Ok(current) = std::env::current_dir() {
        if let Some(parent) = current.parent() {
            let candidate = parent.join(dir);
            if candidate.exists() {
                return candidate;
            }
        }
        dir.to_path_buf()
    } else {
        dir.to_path_buf()
    }
}

/// Checks that all required ONNX models exist locally; downloads any missing models.
pub async fn ensure_models(model_dir: &Path) -> Result<(), OcrError> {
    let resolved_dir = resolve_model_dir(model_dir);
    if !resolved_dir.exists() {
        std::fs::create_dir_all(&resolved_dir)
            .map_err(|e| OcrError::ModelDownload(format!("Failed to create model dir: {}", e)))?;
    }

    let models = [
        (DET_MODEL_NAME, DET_MODEL_URL),
        (CLS_MODEL_NAME, CLS_MODEL_URL),
        (REC_MODEL_NAME, REC_MODEL_URL),
    ];

    for (name, url) in models {
        let dest = resolved_dir.join(name);
        if !dest.exists() {
            info!("OCR model {} missing; downloading from {}", name, url);
            download_model(url, &dest).await?;
            info!("OCR model {} successfully downloaded to {:?}", name, dest);
        }
    }

    Ok(())
}

async fn download_model(url: &str, dest: &Path) -> Result<(), OcrError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| OcrError::ModelDownload(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| OcrError::ModelDownload(format!("Failed to request {}: {}", url, e)))?;

    if !response.status().is_success() {
        return Err(OcrError::ModelDownload(format!(
            "Download failed with status {}: {}",
            response.status(),
            url
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| OcrError::ModelDownload(format!("Failed to read stream from {}: {}", url, e)))?;

    let tmp_dest = dest.with_extension("tmp");
    tokio::fs::write(&tmp_dest, &bytes)
        .await
        .map_err(|e| OcrError::ModelDownload(format!("Failed to write tmp model file: {}", e)))?;

    tokio::fs::rename(&tmp_dest, dest)
        .await
        .map_err(|e| OcrError::ModelDownload(format!("Failed to persist model file: {}", e)))?;

    Ok(())
}

/// Synchronous text recognition on raw image bytes using PaddleOCR ONNX models.
pub fn run_ocr_sync(image_bytes: &[u8], model_dir: &Path) -> Result<OcrResult, OcrError> {
    let dynamic_img = image::load_from_memory(image_bytes)
        .map_err(|e| OcrError::ImageDecode(e.to_string()))?;
    let rgb_img = dynamic_img.to_rgb8();

    let resolved_dir = resolve_model_dir(model_dir);
    let det_path = resolved_dir.join(DET_MODEL_NAME);
    let cls_path = resolved_dir.join(CLS_MODEL_NAME);
    let rec_path = resolved_dir.join(REC_MODEL_NAME);

    if !det_path.exists() || !cls_path.exists() || !rec_path.exists() {
        return Err(OcrError::MissingModel(format!(
            "Required ONNX models missing in {:?}. Need {}, {}, and {}",
            resolved_dir, DET_MODEL_NAME, CLS_MODEL_NAME, REC_MODEL_NAME
        )));
    }

    let det_str = det_path
        .to_str()
        .ok_or_else(|| OcrError::MissingModel("Invalid UTF-8 in det path".into()))?;
    let cls_str = cls_path
        .to_str()
        .ok_or_else(|| OcrError::MissingModel("Invalid UTF-8 in cls path".into()))?;
    let rec_str = rec_path
        .to_str()
        .ok_or_else(|| OcrError::MissingModel("Invalid UTF-8 in rec path".into()))?;

    let mut ocr = paddle_ocr_rs::ocr_lite::OcrLite::new();
    ocr.init_models(det_str, cls_str, rec_str, 2)
        .map_err(|e| OcrError::Inference(format!("{:?}", e)))?;

    // Perform OCR inference with angle correction rollback
    let res = ocr
        .detect_angle_rollback(&rgb_img, 50, 1024, 0.5, 0.3, 1.6, true, false, 0.8)
        .map_err(|e| OcrError::Inference(format!("{:?}", e)))?;

    let blocks = res
        .text_blocks
        .into_iter()
        .map(|b| OcrTextBlock {
            text: b.text,
            score: b.text_score,
        })
        .collect();

    Ok(OcrResult::new(blocks))
}

/// Asynchronous entrypoint: ensures models are present, then delegates OCR inference to a blocking threadpool.
pub async fn run_ocr(image_bytes: Vec<u8>, model_dir: PathBuf) -> Result<OcrResult, OcrError> {
    ensure_models(&model_dir).await?;

    tokio::task::spawn_blocking(move || run_ocr_sync(&image_bytes, &model_dir))
        .await
        .map_err(|e| OcrError::Join(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_corrupt_image_returns_decode_error() {
        let bad_bytes = b"not an image file at all";
        let res = run_ocr_sync(bad_bytes, Path::new("data/ocr_models"));
        assert!(matches!(res, Err(OcrError::ImageDecode(_))));
    }

    #[test]
    fn test_missing_model_returns_missing_model_error() {
        let img = image::RgbImage::new(50, 50);
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();

        let non_existent_dir = Path::new("non_existent_ocr_dir_12345");
        let res = run_ocr_sync(&buf.into_inner(), non_existent_dir);
        assert!(matches!(res, Err(OcrError::MissingModel(_))));
    }

    #[test]
    fn test_real_image_ocr_inference() {
        let fixture_candidates = [
            Path::new("tests/fixtures/test_1.png"),
            Path::new("backend/tests/fixtures/test_1.png"),
        ];
        let fixture_path = fixture_candidates.into_iter().find(|p| p.exists());
        if let Some(path) = fixture_path {
            let bytes = std::fs::read(path).expect("Failed to read fixture image");
            let model_dir = Path::new("data/ocr_models");
            let res = run_ocr_sync(&bytes, model_dir);
            match res {
                Ok(ocr_result) => {
                    assert!(!ocr_result.is_empty());
                    assert!(
                        ocr_result.combined_text.contains("paddle-ocr-rs")
                            || ocr_result.combined_text.contains("Rust")
                    );
                }
                Err(OcrError::MissingModel(_)) => {
                    // Models not present in this runner environment, graceful skip
                }
                Err(e) => panic!("Unexpected OCR error: {:?}", e),
            }
        }
    }
}
