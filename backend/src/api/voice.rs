use axum::{
    extract::{Multipart, State},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::voice::{audio, sherpa};

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VoiceTranscribeResponse {
    pub text: String,
    pub language: Option<String>,
    pub duration_secs: f32,
}

/// Transcribes an uploaded voice audio recording using local SenseVoiceSmall inference.
#[utoipa::path(
    post,
    path = "/api/v1/voice/transcribe",
    tag = "Voice",
    responses(
        (status = 200, description = "Audio transcribed successfully", body = VoiceTranscribeResponse),
        (status = 400, description = "Invalid or empty audio payload"),
        (status = 503, description = "Voice transcription engine is disabled")
    ),
    security(("session_cookie" = []))
)]
pub async fn transcribe_voice(
    _user: AuthUser,
    State(state): State<ApiState>,
    mut multipart: Multipart,
) -> Result<Json<VoiceTranscribeResponse>, ApiError> {
    if !state.voice_config.enabled {
        return Err(ApiError::service_unavailable(
            "Voice transcription is disabled in server configuration",
        ));
    }

    let mut audio_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read multipart payload: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "audio" || name.is_empty() {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to read audio bytes: {}", e)))?;
            audio_bytes = Some(bytes.to_vec());
            break;
        }
    }

    let bytes = audio_bytes
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::bad_request("No audio file provided in multipart request"))?;

    info!("Transcribing voice audio ({} bytes)", bytes.len());

    let samples = audio::decode_audio_to_16k_mono(&bytes)
        .map_err(|err| ApiError::bad_request(format!("Audio decoding failed: {}", err)))?;

    let model_dir = state.voice_config.model_dir.clone();
    let lang = state.voice_config.language.clone();

    let result = tokio::task::spawn_blocking(move || {
        sherpa::run_transcription_sync(&samples, &model_dir, &lang)
    })
    .await
    .map_err(|e| ApiError::internal(format!("Transcription task panicked: {}", e)))?
    .map_err(|err| {
        warn!("SenseVoice inference failed: {:?}", err);
        ApiError::internal(format!("Voice transcription failed: {}", err))
    })?;

    Ok(Json(VoiceTranscribeResponse {
        text: result.text,
        language: result.language,
        duration_secs: result.duration_secs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcribe_response_serialization() {
        let resp = VoiceTranscribeResponse {
            text: "Hello world".to_string(),
            language: Some("en".to_string()),
            duration_secs: 1.25,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Hello world"));
        assert!(json.contains("1.25"));
    }
}
