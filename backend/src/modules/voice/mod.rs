use matrix_sdk::room::Room;
use tracing::{info, warn};

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::utils::html::html_escape;

pub mod audio;
pub mod models;
pub mod prompt;
pub mod sherpa;

pub use models::{VoiceConfig, VoiceError, VoiceTranscriptionResult};

/// Processes an incoming Matrix audio or voice note message:
/// 1. Decodes audio bytes (Ogg Opus, WAV, MP3, etc.) into 16kHz mono PCM.
/// 2. Performs speech-to-text inference with sherpa-onnx and SenseVoiceSmall.
/// 3. Builds a structured semantic intent prompt with the transcription.
/// 4. Executes an AI turn with all productivity tools enabled (add_expense, add_todo, create_reminder).
/// 5. Formats and returns an HTML reply summarizing the actions taken.
#[allow(clippy::too_many_arguments)]
pub async fn process_audio_event(
    db: &Database,
    ai: &crate::config::AiEnvConfig,
    broadcaster: Option<&EventBroadcaster>,
    room: &Room,
    owner_id: &str,
    audio_bytes: &[u8],
    config: &VoiceConfig,
    stage_reporter: Option<&crate::bot::stage::StageReporter>,
) -> String {
    if audio_bytes.is_empty() {
        return "⚠️ <b>语音处理失败</b><br/>\
                • <b>原因:</b> 未接收到有效的音频数据<br/>\
                • <b>建议:</b> 请尝试重新录制并发送语音，或直接输入文字。"
            .to_string();
    }

    if let Some(reporter) = stage_reporter {
        reporter.report_stage("🎧 正在转写语音...").await;
    }

    info!(
        "Beginning voice processing for room [{}] owner [{}] ({} audio bytes)",
        room.room_id(),
        owner_id,
        audio_bytes.len()
    );

    // 1. Decode audio into 16kHz mono f32 samples
    let samples = match audio::decode_audio_to_16k_mono(audio_bytes) {
        Ok(samples) => samples,
        Err(err) => {
            warn!("Audio decoding failed for owner {}: {:?}", owner_id, err);
            return format!(
                "⚠️ <b>语音转写失败</b><br/>\
                 • <b>原因:</b> 音频解码失败或格式不兼容（{}）<br/>\
                 • <b>建议:</b> 请检查麦克风录音权限或尝试重新发送，也可直接输入文字。",
                html_escape(&err.to_string())
            );
        }
    };

    // 2. Run offline speech recognition via sherpa-onnx + SenseVoice
    let transcription =
        match sherpa::run_transcription(samples, config.model_dir.clone(), config.language.clone())
            .await
        {
            Ok(res) => res,
            Err(err) => {
                warn!(
                    "Voice transcription failed for owner {}: {:?}",
                    owner_id, err
                );
                return format!(
                    "⚠️ <b>语音转写服务异常</b><br/>\
                 • <b>原因:</b> 语音识别引擎暂时不可用（{}）<br/>\
                 • <b>建议:</b> 请稍后重试，或直接输入文字。",
                    html_escape(&err.to_string())
                );
            }
        };

    if transcription.is_empty() {
        return "⚠️ <b>语音未能识别</b><br/>\
                • <b>原因:</b> 未检测到清晰人声（声音过小或背景噪音较大）<br/>\
                • <b>建议:</b> 请尝试靠近麦克风重新录制，或直接输入文字。"
            .to_string();
    }

    info!(
        "Voice transcribed ({:.1}s): \"{}\" for owner {}",
        transcription.duration_secs, transcription.text, owner_id
    );

    if let Some(reporter) = stage_reporter {
        reporter.report_stage("🧠 正在理解与思考...").await;
    }

    // 3. Build semantic AI prompt from transcribed text
    let ai_prompt = prompt::build_voice_prompt(&transcription.text, transcription.duration_secs);

    // 4. Run AI turn with full domain tool capabilities
    match crate::modules::ai::handler::run_ai_turn_with_stage(
        db.sqlite_pool(),
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
        Ok(ai_turn) => format_voice_turn_response(&ai_turn, &transcription),
        Err(err) => {
            warn!("AI turn failed for transcribed voice: {}", err);
            format!(
                "🎙️ <b>语音内容:</b> “{}”<br/><br/>\
                 ⚠️ <b>AI 处理遇到问题</b><br/>\
                 • <b>原因:</b> {}<br/>\
                 • <b>建议:</b> 请稍后重试，或直接发送文字指令。",
                html_escape(&transcription.text),
                html_escape(&err.to_string())
            )
        }
    }
}

fn format_voice_turn_response(
    ai_turn: &crate::modules::ai::handler::AiTurnResult,
    _transcription: &VoiceTranscriptionResult,
) -> String {
    crate::modules::ai::handler::format_response(
        &ai_turn.message,
        &ai_turn.model_used,
        Some("SenseVoice"),
        &ai_turn.tool_calls_executed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ai::handler::AiTurnResult;

    #[test]
    fn test_format_voice_turn_response() {
        let ai_turn = AiTurnResult {
            conversation_id: "conv-1".to_string(),
            message: "Added task **Review budget** to your to-do list.".to_string(),
            model_used: "grok-4.6".to_string(),
            tool_calls_executed: vec!["add_todo".to_string()],
        };
        let transcription = VoiceTranscriptionResult {
            text: "Remind me to review the budget tomorrow".to_string(),
            language: Some("en".to_string()),
            duration_secs: 2.8,
        };

        let html = format_voice_turn_response(&ai_turn, &transcription);
        assert!(html.contains("Review budget"));
        assert!(html.contains("🤖"));
        assert!(html.contains("grok-4.6 • SenseVoice"));
        assert!(!html.contains("add_todo"));
    }
}
