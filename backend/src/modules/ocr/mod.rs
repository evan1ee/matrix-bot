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
pub async fn process_image_event(
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
    room: &Room,
    owner_id: &str,
    caption: Option<&str>,
    image_bytes: &[u8],
    model_dir: &Path,
) -> String {
    if image_bytes.is_empty() {
        return "❌ <b>Empty image data received.</b>".to_string();
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
                "⚠️ <b>OCR Processing Error:</b> {}<br/>\
                 <em>Please ensure ONNX models are available or try again with a clearer image.</em>",
                html_escape(&err.to_string())
            );
        }
    };

    if ocr_result.is_empty() {
        return "🔍 <b>No readable text detected in the image.</b><br/>\
                If this was an expense receipt or task note, please ensure good lighting and that text is clearly focused."
            .to_string();
    }

    info!(
        "OCR detected {} text blocks ({} characters) for owner {}",
        ocr_result.blocks.len(),
        ocr_result.combined_text.len(),
        owner_id
    );

    let ai_prompt = prompt::build_ocr_prompt(caption, &ocr_result.combined_text);

    match crate::modules::ai::handler::run_ai_turn(
        &db.pool,
        broadcaster,
        owner_id,
        room.room_id().as_str(),
        None,
        &ai_prompt,
    )
    .await
    {
        Ok(ai_turn) => format_ocr_turn_response(&ai_turn, &ocr_result),
        Err(err) => {
            warn!("AI turn failed for OCR text: {}", err);
            format!(
                "📄 <b>Extracted Text:</b><br/><pre>{}</pre><br/>\
                 ⚠️ <em>Could not process with AI assistant: {}</em>",
                html_escape(&ocr_result.combined_text),
                html_escape(&err)
            )
        }
    }
}

fn format_ocr_turn_response(
    ai_turn: &crate::modules::ai::handler::AiTurnResult,
    ocr_result: &OcrResult,
) -> String {
    let html_rendered = crate::modules::webhook::renderer::markdown_to_html(&ai_turn.message);
    format!(
        "{}<br/><font color=\"gray\" size=\"-2\">📷 <em>Extracted {} lines via PaddleOCR • {}</em></font>",
        html_rendered,
        ocr_result.blocks.len(),
        html_escape(&ai_turn.model_used)
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
        assert!(html.contains("Extracted 2 lines via PaddleOCR"));
        assert!(html.contains("grok-4.6"));
    }
}
