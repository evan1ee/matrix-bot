use axum::{
    extract::{Multipart, State},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::interface::http::auth::types::{ApiState, AuthUser};
use crate::modules::voice::VoiceTranscriptionResult;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VoiceTranscribeResponse {
    pub text: String,
    pub language: Option<String>,
    pub duration_secs: f32,
}

/// Transcribes an uploaded voice recording through the configured speech-to-text service.
#[utoipa::path(
    post,
    path = "/api/v1/voice/transcribe",
    tag = "Voice",
    responses(
        (status = 200, description = "Audio transcribed successfully", body = VoiceTranscribeResponse),
        (status = 400, description = "Invalid or empty audio payload, or audio the transcription service could not process"),
        (status = 500, description = "Voice transcription failed"),
        (status = 503, description = "Voice transcription is disabled, not configured, or temporarily unavailable")
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

    let mut upload: Option<(Vec<u8>, Option<String>)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read multipart payload: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "audio" || name.is_empty() {
            let content_type = field.content_type().map(str::to_owned);
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to read audio bytes: {}", e)))?;
            upload = Some((bytes.to_vec(), content_type));
            break;
        }
    }

    let (bytes, content_type) = upload
        .filter(|(bytes, _)| !bytes.is_empty())
        .ok_or_else(|| ApiError::bad_request("No audio file provided in multipart request"))?;

    info!("Transcribing voice audio ({} bytes)", bytes.len());

    let result: VoiceTranscriptionResult = state
        .stt
        .transcribe(
            bytes,
            content_type.as_deref(),
            state.voice_config.language_hint(),
        )
        .await
        .map_err(voice_error_response)?
        .into();

    Ok(Json(VoiceTranscribeResponse {
        text: result.text,
        language: result.language,
        duration_secs: result.duration_secs,
    }))
}

/// Maps a transcription failure to a response. Every body is a fixed string:
/// the error's own text carries the service's response body and, on a
/// connection failure, the request URL. Detail goes to the log instead.
fn voice_error_response(err: crate::infrastructure::inference::stt::SttServiceError) -> ApiError {
    use crate::infrastructure::inference::stt::SttServiceError;

    match err {
        SttServiceError::NotConfigured => ApiError::service_unavailable(
            "Voice transcription service is not configured on this server",
        ),
        SttServiceError::Upstream { status, message } => {
            warn!(
                "Voice transcription failed upstream ({}): {}",
                status, message
            );
            match status {
                400 => ApiError::bad_request(
                    "The transcription service could not process the uploaded audio",
                ),
                503 => ApiError::service_unavailable(
                    "The voice transcription service is temporarily unavailable",
                ),
                _ => ApiError::internal("Voice transcription failed"),
            }
        }
        SttServiceError::Unreachable(detail) => {
            warn!("Voice transcription service unreachable: {}", detail);
            ApiError::internal("Voice transcription failed")
        }
    }
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

    use crate::infrastructure::inference::stt::SttServiceError;
    use axum::http::StatusCode;

    #[test]
    fn test_voice_error_response_status_for_every_arm() {
        let status = |err: SttServiceError| voice_error_response(err).status;
        let upstream = |code: u16| SttServiceError::Upstream {
            status: code,
            message: "detail".to_string(),
        };

        assert_eq!(
            status(SttServiceError::NotConfigured),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(status(upstream(400)), StatusCode::BAD_REQUEST);
        assert_eq!(status(upstream(503)), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(status(upstream(401)), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            status(SttServiceError::Unreachable("refused".to_string())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_voice_error_response_never_relays_upstream_text() {
        let decode = voice_error_response(SttServiceError::Upstream {
            status: 400,
            message: "symphonia-detail: unsupported codec".to_string(),
        });
        assert!(!decode.message.contains("symphonia-detail"));

        let unreachable = voice_error_response(SttServiceError::Unreachable(
            "error sending request for url (http://inference:8090/v1/audio/transcriptions)"
                .to_string(),
        ));
        assert!(!unreachable.message.contains("inference:8090"));
    }
}
