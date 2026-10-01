use matrix_sdk::room::Room;
use tracing::{info, warn};

use crate::db::Database;
use crate::utils::html::html_escape;

pub mod models;
pub mod prompt;

pub use models::{OcrConfig, OcrResult, OcrTextBlock};

use crate::domain::accounts::AccountId;

/// Processes an incoming Matrix image attachment:
/// 1. POSTs the decrypted image bytes to the inference server's `/ocr` and
///    gets back detection and recognition results.
/// 2. If text is extracted, synthesizes a structured semantic prompt with the text and user caption.
/// 3. Executes an AI turn with all domain tools enabled (add_expense, add_todo, create_reminder).
/// 4. Returns an HTML reply summarizing the actions taken and persisted to the database.
#[allow(clippy::too_many_arguments)]
pub async fn process_image_event(
    db: &Database,
    services: &crate::application::AppServices,
    ai: &crate::config::AiEnvConfig,
    room: &Room,
    owner_id: &AccountId,
    caption: Option<&str>,
    image_bytes: &[u8],
    ocr: &crate::infrastructure::inference::ocr::OcrClient,
    stage_reporter: Option<&crate::interface::channels::matrix::stage::StageReporter>,
) -> String {
    if image_bytes.is_empty() {
        return "⚠️ <b>图片处理失败</b><br/>\
                • <b>原因:</b> 未接收到有效的图片数据<br/>\
                • <b>建议:</b> 请尝试重新上传图片，或直接输入文字。"
            .to_string();
    }

    if let Some(reporter) = stage_reporter {
        reporter.report_stage("🔍 正在识别图片文字 (OCR)...").await;
    }

    info!(
        "Beginning OCR processing for room [{}] owner [{}] ({} image bytes)",
        room.room_id(),
        owner_id,
        image_bytes.len()
    );

    let ocr_result: OcrResult = match ocr.scan(image_bytes.to_vec()).await {
        Ok(scan) => scan.into(),
        Err(err) => {
            warn!("OCR extraction failed for owner {}: {:?}", owner_id, err);
            return ocr_error_message(&err);
        }
    };

    if ocr_result.is_empty() {
        return "⚠️ <b>未能识别出文字</b><br/>\
                • <b>原因:</b> 图片中未检测到清晰文字（可能光照不足、模糊或角度过倾斜）<br/>\
                • <b>建议:</b> 请在光线明亮处重新拍照，或直接输入文字描述。"
            .to_string();
    }

    info!(
        "OCR detected {} text blocks ({} characters) for owner {}",
        ocr_result.blocks.len(),
        ocr_result.combined_text.len(),
        owner_id
    );

    if let Some(reporter) = stage_reporter {
        reporter.report_stage("🧠 正在理解与思考...").await;
    }

    let ai_prompt = prompt::build_ocr_prompt(caption, &ocr_result.combined_text);

    let progress = stage_reporter.map(|r| r as &dyn crate::llm::progress::ProgressSink);
    let turn_ctx = crate::llm::LlmTurnContext::new(
        services,
        &db.pool,
        ai,
        owner_id,
        room.room_id().as_str(),
        progress,
    );
    match crate::llm::run_turn(&turn_ctx, &ai_prompt, None).await {
        Ok(ai_turn) => {
            crate::interface::channels::matrix::reaction_targets::record_ai_tool_targets(
                &db.pool,
                progress.and_then(|p| p.message_ref()).as_deref(),
                room.room_id().as_str(),
                owner_id,
                &ai_turn.tool_results,
            )
            .await;
            format_ocr_turn_response(&ai_turn, &ocr_result)
        }
        Err(err) => {
            warn!("AI turn failed for OCR text: {}", err);
            format!(
                "📄 <b>提取的文字内容:</b><br/><pre>{}</pre><br/>\
                 ⚠️ <b>AI 处理遇到问题</b><br/>\
                 • <b>原因:</b> {}<br/>\
                 • <b>建议:</b> 请稍后重试，或直接发送文字指令。",
                html_escape(&ocr_result.combined_text),
                html_escape(&err.to_string())
            )
        }
    }
}

/// Maps a failed `OcrClient::scan` call onto the HTML reply sent into the
/// Matrix room, without ever including the upstream detail string.
///
/// `OcrServiceError::Unreachable`'s `Display` embeds the request URL (so it
/// names the internal inference hostname and port), and
/// `OcrServiceError::Upstream`'s `message` is the inference server's raw
/// response body -- neither belongs in a chat room. This mirrors
/// `api/ocr.rs::ocr_error_response`, which already keeps both out of the web
/// API's response; the full detail still reaches `warn!` at the call site.
///
/// Per the spec's Degradation section, an unset base URL and an unreachable
/// service answer with the same plain "unavailable" message -- as does a
/// `503` upstream, since that means the inference server itself has no
/// models installed, not that this one image failed. Only a `400` (the
/// image could not be decoded) is a genuine per-image failure, and it keeps
/// its own distinct, actionable message: the user can reasonably retry with
/// a different image.
fn ocr_error_message(err: &crate::infrastructure::inference::ocr::OcrServiceError) -> String {
    use crate::infrastructure::inference::ocr::OcrServiceError;

    match err {
        OcrServiceError::NotConfigured => "⚠️ <b>图片处理不可用</b><br/>\
             • <b>原因:</b> 本部署未配置文字识别服务<br/>\
             • <b>建议:</b> 请直接输入文字，或联系管理员启用 OCR。"
            .to_string(),
        OcrServiceError::Upstream { status: 400, .. } => "⚠️ <b>图片无法识别</b><br/>\
             • <b>原因:</b> 图片格式无法解析<br/>\
             • <b>建议:</b> 请尝试重新拍摄或上传清晰的图片，或直接输入文字。"
            .to_string(),
        OcrServiceError::Unreachable(_) | OcrServiceError::Upstream { .. } => {
            "⚠️ <b>图片处理不可用</b><br/>\
             • <b>原因:</b> 文字识别服务当前不可用<br/>\
             • <b>建议:</b> 请稍后重试，或直接输入文字。"
                .to_string()
        }
    }
}

fn format_ocr_turn_response(ai_turn: &crate::llm::AiTurnResult, _ocr_result: &OcrResult) -> String {
    crate::llm::prompt::format_response(
        &ai_turn.message,
        &ai_turn.model_used,
        Some("PaddleOCR"),
        &ai_turn.tool_calls_executed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::ocr::OcrServiceError;
    use crate::llm::AiTurnResult;

    /// The regression this guards against: `Unreachable`'s `Display` embeds
    /// the request URL (host + port of the internal inference service), and
    /// `Upstream`'s `message` is the inference server's raw response body.
    /// Neither may ever reach a Matrix room.
    #[test]
    fn test_no_upstream_detail_string_reaches_the_returned_html() {
        let leaky_url = "http://inference:8090/ocr";
        let leaky_body = "panicked at inference/src/paddle.rs:42: onnx runtime blew up";

        let cases = [
            OcrServiceError::NotConfigured,
            OcrServiceError::Unreachable(format!(
                "OCR service is unreachable: error sending request for url ({leaky_url})"
            )),
            OcrServiceError::Upstream {
                status: 503,
                message: leaky_body.to_string(),
            },
            OcrServiceError::Upstream {
                status: 500,
                message: leaky_body.to_string(),
            },
        ];

        for case in cases {
            let html = ocr_error_message(&case);
            assert!(
                !html.contains("inference:8090") && !html.contains(leaky_url),
                "leaked the internal inference URL for {:?}: {}",
                case,
                html
            );
            assert!(
                !html.contains("paddle.rs") && !html.contains("onnx runtime"),
                "leaked the upstream response body for {:?}: {}",
                case,
                html
            );
        }
    }

    #[test]
    fn test_a_400_upstream_error_gets_its_own_actionable_message_without_the_decode_detail() {
        let err = OcrServiceError::Upstream {
            status: 400,
            message: "image crate: Format error decoding Png: invalid checksum".to_string(),
        };

        let html = ocr_error_message(&err);

        assert!(!html.contains("Png"));
        assert!(!html.contains("checksum"));
        assert!(html.contains("图片无法识别"));
    }

    #[test]
    fn test_unreachable_and_503_share_the_not_configured_unavailable_shape() {
        let not_configured = ocr_error_message(&OcrServiceError::NotConfigured);
        let unreachable = ocr_error_message(&OcrServiceError::Unreachable(
            "connection refused".to_string(),
        ));
        let upstream_503 = ocr_error_message(&OcrServiceError::Upstream {
            status: 503,
            message: "no models installed".to_string(),
        });

        assert!(not_configured.contains("图片处理不可用"));
        assert!(unreachable.contains("图片处理不可用"));
        assert!(upstream_503.contains("图片处理不可用"));
    }

    #[test]
    fn test_format_ocr_turn_response() {
        let ai_turn = AiTurnResult {
            conversation_id: "conv-1".to_string(),
            message: "Recorded **$16.50** expense for **Shake Shack**.".to_string(),
            model_used: "grok-4.6".to_string(),
            tool_calls_executed: vec!["add_expense".to_string()],
            tool_results: vec![],
        };
        let ocr_result = OcrResult {
            blocks: vec![
                OcrTextBlock {
                    text: "SHAKE SHACK".to_string(),
                    score: 0.98,
                },
                OcrTextBlock {
                    text: "Total $16.50".to_string(),
                    score: 0.99,
                },
            ],
            combined_text: "SHAKE SHACK\nTotal $16.50".to_string(),
        };

        let html = format_ocr_turn_response(&ai_turn, &ocr_result);
        assert!(html.contains("Shake Shack"));
        assert!(html.contains("🤖"));
        assert!(html.contains("grok-4.6 • PaddleOCR"));
        assert!(!html.contains("add_expense"));
    }
}
