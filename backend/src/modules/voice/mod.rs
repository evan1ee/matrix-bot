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
pub async fn process_audio_event(
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
    room: &Room,
    owner_id: &str,
    audio_bytes: &[u8],
    config: &VoiceConfig,
) -> String {
    if !config.enabled {
        return "🎙️ <i>Voice transcription is currently disabled on this bot.</i>".to_string();
    }

    if audio_bytes.is_empty() {
        return "❌ <b>Empty audio data received.</b>".to_string();
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
                "⚠️ <b>Voice Decoding Error:</b> {}<br/>\
                 <em>Please ensure the audio format is supported (Ogg Opus, WAV, MP3, AAC).</em>",
                html_escape(&err.to_string())
            );
        }
    };

    // 2. Run offline speech recognition via sherpa-onnx + SenseVoice
    let transcription = match sherpa::run_transcription(
        samples,
        config.model_dir.clone(),
        config.language.clone(),
    )
    .await
    {
        Ok(res) => res,
        Err(err) => {
            warn!("Voice transcription failed for owner {}: {:?}", owner_id, err);
            return format!(
                "⚠️ <b>Voice Recognition Error:</b> {}<br/>\
                 <em>Please ensure SenseVoice models are properly installed.</em>",
                html_escape(&err.to_string())
            );
        }
    };

    if transcription.is_empty() {
        return "🎙️ <b>No clear speech detected in voice message.</b><br/>\
                <em>The audio may have been silent, background noise, or too quiet to recognize.</em>"
            .to_string();
    }

    info!(
        "Voice transcribed ({:.1}s): \"{}\" for owner {}",
        transcription.duration_secs, transcription.text, owner_id
    );

    // 3. Build semantic AI prompt from transcribed text
    let ai_prompt = prompt::build_voice_prompt(&transcription.text, transcription.duration_secs);

    // 4. Run AI turn with full domain tool capabilities
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
        Ok(ai_turn) => format_voice_turn_response(&ai_turn, &transcription),
        Err(err) => {
            warn!("AI turn failed for transcribed voice: {}", err);
            format!(
                "🎙️ <b>Voice Message:</b> \"{}\"<br/>\
                 ⚠️ <em>Could not process with AI assistant: {}</em>",
                html_escape(&transcription.text),
                html_escape(&err)
            )
        }
    }
}

fn format_voice_turn_response(
    ai_turn: &crate::modules::ai::handler::AiTurnResult,
    transcription: &VoiceTranscriptionResult,
) -> String {
    let html_rendered = crate::modules::webhook::renderer::markdown_to_html(&ai_turn.message);
    format!(
        "{}<br/><font color=\"gray\" size=\"-2\">🎙️ <em>Transcribed {:.1}s voice note via SenseVoice • \"{}\" • {}</em></font>",
        html_rendered,
        transcription.duration_secs,
        html_escape(&transcription.text),
        html_escape(&ai_turn.model_used)
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
        assert!(html.contains("Transcribed 2.8s voice note via SenseVoice"));
        assert!(html.contains("Remind me to review the budget tomorrow"));
        assert!(html.contains("grok-4.6"));
    }
}
