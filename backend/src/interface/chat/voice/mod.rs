use matrix_sdk::room::Room;
use tracing::{info, warn};

use crate::db::Database;
use crate::utils::html::html_escape;

pub mod models;
pub mod prompt;

pub use models::{VoiceConfig, VoiceTranscriptionResult};

/// The voice note's length, for the prompt. A provider's figure wins when it
/// sends one; `json` responses carry none, so the Matrix event's own
/// `info.duration`, set by the sender's client, is the usual source.
fn resolve_duration_secs(provider: Option<f32>, event: Option<std::time::Duration>) -> f32 {
    provider
        .or_else(|| event.map(|d| d.as_secs_f32()))
        .unwrap_or(0.0)
}

use crate::domain::accounts::AccountId;

/// Processes an incoming Matrix audio or voice note message:
/// 1. Sends the audio to the configured speech-to-text service.
/// 2. Builds a structured semantic intent prompt with the transcription.
/// 3. Executes an AI turn with all productivity tools enabled (add_expense, add_todo, create_reminder).
/// 4. Formats and returns an HTML reply summarizing the actions taken.
#[allow(clippy::too_many_arguments)]
pub async fn process_audio_event(
    db: &Database,
    services: &crate::application::AppServices,
    ai: &crate::config::AiEnvConfig,
    room: &Room,
    owner_id: &AccountId,
    audio_bytes: &[u8],
    mime_type: Option<&str>,
    audio_duration: Option<std::time::Duration>,
    config: &VoiceConfig,
    stt: &crate::infrastructure::inference::stt::SttClient,
    stage_reporter: Option<&crate::interface::channels::matrix::stage::StageReporter>,
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

    let transcription: VoiceTranscriptionResult = match stt
        .transcribe(audio_bytes.to_vec(), mime_type, config.language_hint())
        .await
    {
        Ok(t) => {
            let duration_secs = resolve_duration_secs(t.duration, audio_duration);
            let result: VoiceTranscriptionResult = t.into();
            VoiceTranscriptionResult {
                duration_secs,
                ..result
            }
        }
        Err(err) => {
            warn!(
                "Voice transcription failed for owner {}: {:?}",
                owner_id, err
            );
            return voice_error_message(&err);
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

    // 2. Build semantic AI prompt from transcribed text
    let ai_prompt = prompt::build_voice_prompt(&transcription.text, transcription.duration_secs);

    // 3. Run AI turn with full domain tool capabilities
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
            format_voice_turn_response(&ai_turn, &transcription, stt.model())
        }
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

/// The reply for a failed transcription. Never includes the error's own text:
/// that carries the service's response body, and on a connection failure the
/// request URL, neither of which belongs in a chat room. Full detail is in the
/// `warn!` at the call site.
fn voice_error_message(err: &crate::infrastructure::inference::stt::SttServiceError) -> String {
    use crate::infrastructure::inference::stt::SttServiceError;

    match err {
        SttServiceError::NotConfigured => "⚠️ <b>语音处理不可用</b><br/>\
             • <b>原因:</b> 本部署未配置语音识别服务<br/>\
             • <b>建议:</b> 请直接输入文字，或联系管理员启用语音识别。"
            .to_string(),
        SttServiceError::Upstream { status: 400, .. } => "⚠️ <b>语音无法识别</b><br/>\
             • <b>原因:</b> 语音识别服务无法处理这段音频<br/>\
             • <b>建议:</b> 请尝试重新录制并发送语音，或直接输入文字。"
            .to_string(),
        SttServiceError::Unreachable(_) | SttServiceError::Upstream { .. } => {
            "⚠️ <b>语音处理不可用</b><br/>\
             • <b>原因:</b> 语音识别服务当前不可用<br/>\
             • <b>建议:</b> 请稍后重试，或直接输入文字。"
                .to_string()
        }
    }
}

fn format_voice_turn_response(
    ai_turn: &crate::llm::AiTurnResult,
    _transcription: &VoiceTranscriptionResult,
    engine_label: &str,
) -> String {
    crate::llm::prompt::format_response(
        &ai_turn.message,
        &ai_turn.model_used,
        Some(engine_label),
        &ai_turn.tool_calls_executed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::AiTurnResult;

    #[test]
    fn test_format_voice_turn_response() {
        let ai_turn = AiTurnResult {
            conversation_id: "conv-1".to_string(),
            message: "Added task **Review budget** to your to-do list.".to_string(),
            model_used: "grok-4.6".to_string(),
            tool_calls_executed: vec!["add_todo".to_string()],
            tool_results: vec![],
        };
        let transcription = VoiceTranscriptionResult {
            text: "Remind me to review the budget tomorrow".to_string(),
            language: Some("en".to_string()),
            duration_secs: 2.8,
        };

        let html = format_voice_turn_response(&ai_turn, &transcription, "whisper-1");
        assert!(html.contains("Review budget"));
        assert!(html.contains("🤖"));
        assert!(html.contains("grok-4.6 • whisper-1"));
        assert!(!html.contains("add_todo"));
    }

    #[test]
    fn test_no_voice_error_reply_carries_upstream_detail() {
        use crate::infrastructure::inference::stt::SttServiceError;
        let leaky = "http://inference:8090/v1/audio/transcriptions symphonia-detail";
        let cases = [
            (SttServiceError::NotConfigured, "语音处理不可用"),
            (
                SttServiceError::Unreachable(leaky.to_string()),
                "语音处理不可用",
            ),
            (
                SttServiceError::Upstream {
                    status: 400,
                    message: leaky.to_string(),
                },
                "语音无法识别",
            ),
            (
                SttServiceError::Upstream {
                    status: 503,
                    message: leaky.to_string(),
                },
                "语音处理不可用",
            ),
            (
                SttServiceError::Upstream {
                    status: 401,
                    message: leaky.to_string(),
                },
                "语音处理不可用",
            ),
        ];
        for (err, headline) in cases {
            let html = voice_error_message(&err);
            assert!(html.contains(headline), "{:?} -> {}", err, html);
            assert!(!html.contains("inference:8090"), "leaked a URL: {}", html);
            assert!(
                !html.contains("symphonia-detail"),
                "leaked detail: {}",
                html
            );
        }
    }

    #[test]
    fn test_duration_prefers_the_provider_then_the_matrix_event() {
        use std::time::Duration;
        assert_eq!(
            resolve_duration_secs(Some(2.5), Some(Duration::from_secs(9))),
            2.5
        );
        assert_eq!(
            resolve_duration_secs(None, Some(Duration::from_millis(1500))),
            1.5
        );
        assert_eq!(resolve_duration_secs(None, None), 0.0);
    }
}
