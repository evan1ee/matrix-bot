use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use tracing::warn;

use super::AppState;
use crate::sherpa::VoiceError;

/// The `response_format` values this server supports. OpenAI defines more
/// (`text`, `srt`, `vtt`); the bot asks only for `verbose_json`, and `json` is
/// OpenAI's default when the field is absent.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ResponseFormat {
    Json,
    VerboseJson,
}

/// The `json` response: text only.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TranscriptionText {
    pub text: String,
}

/// The `verbose_json` response, as OpenAI defines it minus `segments`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TranscriptionVerbose {
    pub task: &'static str,
    pub language: Option<String>,
    pub duration: f32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum TranscriptionBody {
    Json(TranscriptionText),
    Verbose(TranscriptionVerbose),
}

impl ResponseFormat {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("json") => Ok(Self::Json),
            Some("verbose_json") => Ok(Self::VerboseJson),
            Some(other) => Err(format!(
                "Unsupported response_format '{}'; expected json or verbose_json",
                other
            )),
        }
    }
}

struct TranscriptionRequest {
    file: Vec<u8>,
    language: Option<String>,
    format: ResponseFormat,
}

/// Normalises the optional `language` field: blank means absent, case is
/// folded, and anything SenseVoice cannot do is the caller's error.
fn parse_language(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(language) = raw
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| !l.is_empty())
    else {
        return Ok(None);
    };
    if crate::sherpa::SENSEVOICE_LANGUAGES.contains(&language.as_str()) {
        Ok(Some(language))
    } else {
        Err(format!(
            "Unsupported language '{}'; this server supports {}",
            language,
            crate::sherpa::SENSEVOICE_LANGUAGES.join(", ")
        ))
    }
}

/// Reads the multipart form. `model` and every other OpenAI field are accepted
/// and ignored: this server has exactly one model.
async fn read_request(
    mut multipart: Multipart,
) -> Result<TranscriptionRequest, (StatusCode, String)> {
    let bad = |message: String| (StatusCode::BAD_REQUEST, message);
    let mut file = None;
    let mut language = None;
    let mut format = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(format!("Malformed multipart body: {}", e)))?
    {
        let name = field.name().map(str::to_owned);
        match name.as_deref() {
            Some("file") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| bad(format!("Failed to read 'file': {}", e)))?;
                file = Some(bytes.to_vec());
            }
            Some("language") => {
                language = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad(format!("Failed to read 'language': {}", e)))?,
                );
            }
            Some("response_format") => {
                format = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad(format!("Failed to read 'response_format': {}", e)))?,
                );
            }
            _ => {}
        }
    }

    let file = file
        .filter(|f| !f.is_empty())
        .ok_or_else(|| bad("Missing or empty 'file' field".to_string()))?;
    let format = ResponseFormat::parse(format.as_deref()).map_err(bad)?;
    let language = parse_language(language).map_err(bad)?;

    Ok(TranscriptionRequest {
        file,
        language,
        format,
    })
}

/// Maps an engine error to a status. Exhaustive on purpose: a new
/// `VoiceError` variant must be a compile error here, not a silent 500.
fn status_for(err: &VoiceError) -> StatusCode {
    match err {
        VoiceError::AudioDecode(_) => StatusCode::BAD_REQUEST,
        VoiceError::MissingModel(_) | VoiceError::ModelDownload(_) => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        VoiceError::Inference(_) | VoiceError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `POST /v1/audio/transcriptions` — OpenAI's transcription endpoint, served
/// by SenseVoice. The audio is decoded before the model gate, so a malformed
/// upload is a 400 whether or not models are installed: the caller's error is
/// reported before the server's state.
pub async fn transcribe(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<TranscriptionBody>, (StatusCode, String)> {
    let request = read_request(multipart).await?;

    let samples = crate::audio::decode_audio_to_16k_mono(&request.file)
        .map_err(|e| (status_for(&e), e.to_string()))?;

    if !crate::sherpa::models_present(&state.config.voice_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Speech-to-text models are not installed on this inference server".to_string(),
        ));
    }

    let language = request.language.unwrap_or_else(|| "auto".to_string());
    let result =
        crate::sherpa::run_transcription(samples, state.config.voice_model_dir.clone(), language)
            .await
            .map_err(|e| {
                warn!("SenseVoice transcription failed: {:?}", e);
                (status_for(&e), e.to_string())
            })?;

    let body = match request.format {
        ResponseFormat::Json => TranscriptionBody::Json(TranscriptionText { text: result.text }),
        ResponseFormat::VerboseJson => TranscriptionBody::Verbose(TranscriptionVerbose {
            task: "transcribe",
            language: result.language,
            duration: result.duration_secs,
            text: result.text,
        }),
    };
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    const STT_VERBOSE_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/stt_transcription_verbose.json"
    ));
    const STT_JSON_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/stt_transcription_json.json"
    ));
    const BOUNDARY: &str = "matrix-bot-test-boundary";

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
                voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
            }),
        }
    }

    /// A multipart/form-data body; each part is (name, filename, bytes).
    fn multipart_body(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, bytes) in parts {
            body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
            let disposition = match filename {
                Some(f) => format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\r\n",
                    name, f
                ),
                None => format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name),
            };
            body.extend_from_slice(disposition.as_bytes());
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());
        body
    }

    async fn post_transcription(parts: &[(&str, Option<&str>, &[u8])]) -> StatusCode {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/audio/transcriptions")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", BOUNDARY),
            )
            .body(Body::from(multipart_body(parts)))
            .unwrap();
        crate::routes::build_router(state_with_missing_models())
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn test_undecodable_audio_is_a_400_even_without_models() {
        let status = post_transcription(&[("file", Some("a.ogg"), b"not audio")]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_decodable_audio_without_models_is_a_503() {
        let wav = crate::audio::test_audio::silent_wav_16k_mono(1.0);
        let status =
            post_transcription(&[("file", Some("a.wav"), &wav), ("model", None, b"whisper-1")])
                .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_a_request_without_a_file_is_a_400() {
        let status = post_transcription(&[("model", None, b"whisper-1")]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_an_unsupported_response_format_is_a_400() {
        let wav = crate::audio::test_audio::silent_wav_16k_mono(1.0);
        let status = post_transcription(&[
            ("file", Some("a.wav"), &wav),
            ("response_format", None, b"srt"),
        ])
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_parse_language_accepts_supported_values_case_insensitively() {
        assert_eq!(parse_language(None), Ok(None));
        assert_eq!(parse_language(Some("  ".to_string())), Ok(None));
        assert_eq!(
            parse_language(Some("ZH".to_string())),
            Ok(Some("zh".to_string()))
        );
        assert_eq!(
            parse_language(Some(" yue ".to_string())),
            Ok(Some("yue".to_string()))
        );
        assert_eq!(
            parse_language(Some("auto".to_string())),
            Ok(Some("auto".to_string()))
        );
        assert!(parse_language(Some("fr".to_string())).is_err());
    }

    #[tokio::test]
    async fn test_an_unsupported_language_is_a_400_not_a_500() {
        let wav = crate::audio::test_audio::silent_wav_16k_mono(1.0);
        let status =
            post_transcription(&[("file", Some("a.wav"), &wav), ("language", None, b"fr")]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_response_format_defaults_to_json_as_openai_does() {
        assert_eq!(ResponseFormat::parse(None), Ok(ResponseFormat::Json));
        assert_eq!(
            ResponseFormat::parse(Some("json")),
            Ok(ResponseFormat::Json)
        );
        assert_eq!(
            ResponseFormat::parse(Some("verbose_json")),
            Ok(ResponseFormat::VerboseJson)
        );
        assert!(ResponseFormat::parse(Some("srt")).is_err());
    }

    #[test]
    fn test_status_for_every_voice_error() {
        let s = |e: VoiceError| status_for(&e);
        assert_eq!(
            s(VoiceError::AudioDecode("x".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            s(VoiceError::MissingModel("x".into())),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            s(VoiceError::ModelDownload("x".into())),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            s(VoiceError::Inference("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            s(VoiceError::Io("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_verbose_body_matches_the_shared_wire_contract() {
        let body = TranscriptionBody::Verbose(TranscriptionVerbose {
            task: "transcribe",
            language: Some("zh".to_string()),
            duration: 1.5,
            text: "今天天气真好。".to_string(),
        });

        let expected: serde_json::Value =
            serde_json::from_str(STT_VERBOSE_CONTRACT).expect("the contract is valid JSON");
        assert_eq!(serde_json::to_value(&body).unwrap(), expected);
    }

    #[test]
    fn test_json_body_is_text_only() {
        let body = TranscriptionBody::Json(TranscriptionText {
            text: "今天天气真好。".to_string(),
        });

        let expected: serde_json::Value =
            serde_json::from_str(STT_JSON_CONTRACT).expect("the contract is valid JSON");
        assert_eq!(serde_json::to_value(&body).unwrap(), expected);
    }
}
