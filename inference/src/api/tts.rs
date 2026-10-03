use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::time::Duration;
use tracing::warn;

use super::AppState;
use crate::tts::TtsError;

/// OpenAI's own input ceiling for `/audio/speech`.
pub const MAX_INPUT_CHARS: usize = 4096;

/// Extension headers carrying what a Matrix voice message needs and raw
/// audio does not say. Third-party providers omit them, which is legal; the
/// bot then reads the duration from the Ogg stream and sends no waveform.
const DURATION_HEADER: &str = "x-duration-ms";
const WAVEFORM_HEADER: &str = "x-waveform";

/// How many buckets Matrix's `waveform` field expects for a voice message.
const WAVEFORM_BUCKETS: usize = 100;
/// The largest value a waveform bucket may hold, per the Matrix spec.
const WAVEFORM_MAX: f32 = 1024.0;

/// OpenAI's speech request. `model` and any other field OpenAI defines that
/// is not named here, such as `speed`, is accepted and ignored: this server
/// has one model.
#[derive(Debug, Deserialize)]
pub struct SpeechRequest {
    pub input: String,
    pub voice: String,
    #[serde(default)]
    pub response_format: Option<String>,
}

/// Checks the request before the model gate, so the caller's error is
/// reported whether or not the model is installed. Returns the trimmed input
/// and the Kokoro speaker id.
fn validate(request: &SpeechRequest) -> Result<(String, i64), String> {
    match request.response_format.as_deref() {
        None | Some("opus") => {}
        Some(other) => {
            return Err(format!(
                "Unsupported response_format '{}'; this server only produces opus",
                other
            ))
        }
    }

    let input = request.input.trim();
    if input.is_empty() {
        return Err("'input' is empty".to_string());
    }
    if input.chars().count() > MAX_INPUT_CHARS {
        return Err(format!("'input' exceeds {} characters", MAX_INPUT_CHARS));
    }
    // sherpa calls `CString::new(text).unwrap()`, which panics on an embedded
    // NUL; other control characters serve no purpose in speech either. `\n`,
    // `\r` and `\t` are allowed through since ordinary text can carry them.
    if input
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    {
        return Err("'input' contains control characters".to_string());
    }

    let speaker_id = request.voice.trim().parse::<i64>().map_err(|_| {
        format!(
            "Unknown voice '{}'; this server's voices are Kokoro speaker ids (0, 1, 2, ...)",
            request.voice
        )
    })?;

    Ok((input.to_string(), speaker_id))
}

/// Exhaustive on purpose: a new `TtsError` variant must be a compile error
/// here, not a silent 500.
fn status_for(err: &TtsError) -> StatusCode {
    match err {
        TtsError::ModelMissing(_) | TtsError::ModelDownload(_) => StatusCode::SERVICE_UNAVAILABLE,
        TtsError::Engine(_) | TtsError::Encode(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Approximates a waveform for Matrix's voice-message `waveform` field:
/// `WAVEFORM_BUCKETS` buckets, each the peak absolute amplitude within that
/// slice of the clip, scaled to `0..=1024`.
///
/// Indexed by proportional start/end rather than a fixed chunk size, so this
/// always returns exactly `WAVEFORM_BUCKETS` values regardless of how many
/// samples come in — a fixed chunk size sized for the average case silently
/// under- or over-shoots the bucket count at the edges.
fn compute_waveform(samples: &[f32]) -> Vec<u16> {
    if samples.is_empty() {
        return vec![0; WAVEFORM_BUCKETS];
    }

    let len = samples.len();
    (0..WAVEFORM_BUCKETS)
        .map(|i| {
            let start = i * len / WAVEFORM_BUCKETS;
            let end = (((i + 1) * len / WAVEFORM_BUCKETS).max(start + 1)).min(len);
            let peak = samples[start..end]
                .iter()
                .fold(0.0_f32, |acc, &s| acc.max(s.abs()));
            (peak.clamp(0.0, 1.0) * WAVEFORM_MAX).round() as u16
        })
        .collect()
}

fn speech_headers(duration: Duration, waveform: &[u16]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/ogg"));
    headers.insert(
        DURATION_HEADER,
        HeaderValue::from(duration.as_millis() as u64),
    );
    let bars = waveform
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if let Ok(value) = HeaderValue::from_str(&bars) {
        headers.insert(WAVEFORM_HEADER, value);
    }
    headers
}

/// `POST /v1/audio/speech`: OpenAI's text-to-speech endpoint, served by
/// Kokoro. The body is Ogg Opus, which is what makes a Matrix client draw a
/// voice bubble rather than a file card.
pub async fn speak(
    State(state): State<AppState>,
    Json(request): Json<SpeechRequest>,
) -> Result<Response, (StatusCode, String)> {
    let (input, speaker_id) = validate(&request).map_err(|m| (StatusCode::BAD_REQUEST, m))?;

    if !crate::tts::model_present(&state.config.tts_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "The text-to-speech model is not installed on this inference server".to_string(),
        ));
    }

    let fail = |err: TtsError| {
        warn!("Kokoro synthesis failed: {}", err);
        (status_for(&err), err.to_string())
    };

    let speech = crate::tts::synthesize(state.config.tts_model_dir.clone(), input, speaker_id)
        .await
        .map_err(fail)?;
    let ogg = crate::tts::encode_ogg_opus(&speech.samples, speech.sample_rate)
        .map_err(|e| fail(TtsError::Encode(e)))?;

    let headers = speech_headers(speech.duration(), &compute_waveform(&speech.samples));
    Ok((headers, ogg).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;

    const REQUEST_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_request.json"
    ));
    const HEADERS_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_headers.json"
    ));

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
                voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
                tts_model_dir: std::path::PathBuf::from("/nonexistent/tts/models"),
            }),
        }
    }

    async fn post_speech(body: serde_json::Value) -> StatusCode {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/audio/speech")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        crate::api::build_router(state_with_missing_models())
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    #[test]
    fn test_the_shared_request_contract_deserialises() {
        let request: SpeechRequest = serde_json::from_str(REQUEST_CONTRACT).unwrap();
        assert_eq!(request.voice, "0");
        assert_eq!(request.response_format.as_deref(), Some("opus"));
        assert_eq!(validate(&request).unwrap().1, 0);
    }

    #[test]
    fn test_speech_headers_match_the_shared_contract() {
        let headers = speech_headers(Duration::from_millis(1500), &[0, 512, 1024]);
        let expected: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(HEADERS_CONTRACT).unwrap();
        for (name, value) in expected {
            assert_eq!(
                headers.get(name.as_str()).and_then(|v| v.to_str().ok()),
                value.as_str(),
                "header {}",
                name
            );
        }
        assert_eq!(headers.get("content-type").unwrap(), "audio/ogg");
    }

    #[tokio::test]
    async fn test_a_valid_request_without_the_model_is_a_503() {
        let body = serde_json::from_str(REQUEST_CONTRACT).unwrap();
        assert_eq!(post_speech(body).await, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_input_errors_are_400s_even_without_the_model() {
        let long = "字".repeat(MAX_INPUT_CHARS + 1);
        let cases = [
            serde_json::json!({ "input": "   ", "voice": "0" }),
            serde_json::json!({ "input": long, "voice": "0" }),
            serde_json::json!({ "input": "hi", "voice": "alloy" }),
            serde_json::json!({ "input": "hi", "voice": "0", "response_format": "mp3" }),
            serde_json::json!({ "input": "hi\u{0000}there", "voice": "0" }),
        ];
        for body in cases {
            assert_eq!(
                post_speech(body.clone()).await,
                StatusCode::BAD_REQUEST,
                "{}",
                body
            );
        }
    }

    #[test]
    fn test_omitted_response_format_means_opus() {
        let request: SpeechRequest =
            serde_json::from_value(serde_json::json!({ "input": "hi", "voice": "3" })).unwrap();
        assert_eq!(validate(&request), Ok(("hi".to_string(), 3)));
    }

    #[test]
    fn test_status_for_every_tts_error() {
        let s = |e: TtsError| status_for(&e);
        assert_eq!(
            s(TtsError::ModelMissing("x".into())),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            s(TtsError::ModelDownload("x".into())),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            s(TtsError::Engine("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            s(TtsError::Encode("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_waveform_has_exactly_100_buckets_within_range() {
        let samples: Vec<f32> = (0..2000)
            .map(|i| ((i as f32) / 2000.0 * std::f32::consts::TAU).sin())
            .collect();
        let waveform = compute_waveform(&samples);
        assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
        assert!(waveform.iter().all(|&v| v <= 1024));
        assert!(waveform.iter().any(|&v| v > 0), "a sine wave is not silent");
    }

    #[test]
    fn test_silence_produces_an_all_zero_waveform() {
        let samples = vec![0.0_f32; 5000];
        let waveform = compute_waveform(&samples);
        assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
        assert!(waveform.iter().all(|&v| v == 0));
    }

    #[test]
    fn test_waveform_on_empty_samples_is_all_zero() {
        let waveform = compute_waveform(&[]);
        assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
        assert!(waveform.iter().all(|&v| v == 0));
    }

    #[test]
    fn test_waveform_handles_fewer_samples_than_buckets() {
        let samples = vec![1.0_f32; 7];
        let waveform = compute_waveform(&samples);
        assert_eq!(waveform.len(), WAVEFORM_BUCKETS);
        assert!(waveform.iter().all(|&v| v <= 1024));
    }
}
