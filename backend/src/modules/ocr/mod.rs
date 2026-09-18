use matrix_sdk::room::Room;
use std::path::Path;
use tracing::{info, warn};

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::utils::html::html_escape;

pub mod models;
pub mod paddle;
pub mod prompt;

pub use models::{OcrConfig, OcrError, OcrResult, OcrTextBlock};

/// Processes an incoming Matrix image attachment:
/// 1. Runs PaddleOCR detection and text recognition on the decrypted image bytes.
/// 2. If text is extracted, synthesizes a structured semantic prompt with the text and user caption.
/// 3. Executes an AI turn with all domain tools enabled (add_expense, add_todo, create_reminder).
/// 4. Returns an HTML reply summarizing the actions taken and persisting to SQLite.
#[allow(clippy::too_many_arguments)]
pub async fn process_image_event(
    db: &Database,
    ai: &crate::config::AiEnvConfig,
    broadcaster: Option<&EventBroadcaster>,
    room: &Room,
    owner_id: &str,
    caption: Option<&str>,
    image_bytes: &[u8],
    model_dir: &Path,
    stage_reporter: Option<&crate::bot::stage::StageReporter>,
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

    let ocr_result = match paddle::run_ocr(image_bytes.to_vec(), model_dir.to_path_buf()).await {
        Ok(res) => res,
        Err(err) => {
            warn!("OCR extraction failed for owner {}: {:?}", owner_id, err);
            return format!(
                "⚠️ <b>文字识别失败</b><br/>\
                 • <b>原因:</b> 图片文字识别遇到异常（{}）<br/>\
                 • <b>建议:</b> 请尝试重新拍摄清晰平整的图片，或直接输入文字。",
                html_escape(&err.to_string())
            );
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

    match crate::modules::ai::handler::run_ai_turn_with_stage(
        &db.pool,
        ai,
        broadcaster,
        owner_id,
        room.room_id().as_str(),
        None,
        &ai_prompt,
        stage_reporter,
    )
    .await
    {
        Ok(ai_turn) => format_ocr_turn_response(&ai_turn, &ocr_result),
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

fn format_ocr_turn_response(
    ai_turn: &crate::modules::ai::handler::AiTurnResult,
    _ocr_result: &OcrResult,
) -> String {
    crate::modules::ai::handler::format_response(
        &ai_turn.message,
        &ai_turn.model_used,
        Some("PaddleOCR"),
        &ai_turn.tool_calls_executed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ai::handler::AiTurnResult;

    #[test]
    fn test_format_ocr_turn_response() {
        let ai_turn = AiTurnResult {
            conversation_id: "conv-1".to_string(),
            message: "Recorded **$16.50** expense for **Shake Shack**.".to_string(),
            model_used: "grok-4.6".to_string(),
            tool_calls_executed: vec!["add_expense".to_string()],
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
