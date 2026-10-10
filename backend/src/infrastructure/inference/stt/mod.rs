use serde::Deserialize;
use std::fmt;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// Bounds a hung connection; SenseVoice on a CPU and hosted providers both
/// answer a voice note well inside this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI's `json` transcription response. `language` and `duration` are
/// optional because `json` carries text only; a provider that includes them
/// anyway is honoured.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SttTranscription {
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub duration: Option<f32>,
}

#[derive(Debug, Error)]
pub enum SttServiceError {
    #[error("Speech-to-text service is not configured")]
    NotConfigured,
    #[error("Speech-to-text service is unreachable: {0}")]
    Unreachable(String),
    #[error("Speech-to-text service returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

/// Client for any OpenAI-compatible `POST {base}/audio/transcriptions`: the
/// bundled inference server, OpenAI, Groq, speaches. `base_url` includes the
/// version prefix (`.../v1`), the way OpenAI SDKs take it.
///
/// An absent `base_url` is a normal state, not an error condition: it means
/// transcription is not configured, and every call reports `NotConfigured` so
/// the caller can degrade that one feature.
#[derive(Clone)]
pub struct SttClient {
    base_url: Option<String>,
    api_key: Option<String>,
    model: String,
    http: reqwest::Client,
}

/// Written by hand so the bearer token never reaches a `{:?}` log line.
impl fmt::Debug for SttClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SttClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .finish()
    }
}

impl SttClient {
    pub fn new(base_url: Option<String>, api_key: Option<String>, model: String) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());

        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self {
            base_url,
            api_key,
            model,
            http,
        }
    }

    /// The model name sent with each request, for callers that label output.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns a cloned client with dynamic runtime overrides applied when provided.
    pub fn with_overrides(
        &self,
        base_url: Option<&str>,
        api_key: Option<String>,
        model: Option<&str>,
    ) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
            .or_else(|| self.base_url.clone());
        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| self.api_key.clone());
        let model = model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.model.clone());
        Self {
            base_url,
            api_key,
            model,
            http: self.http.clone(),
        }
    }

    /// Sends the audio untouched and returns its transcription.
    ///
    /// The part's filename carries an extension derived from `mime_type`,
    /// because OpenAI identifies the audio format from it; the bundled server
    /// probes the bytes and ignores it. `language` is sent only when given.
    pub async fn transcribe(
        &self,
        audio: Vec<u8>,
        mime_type: Option<&str>,
        language: Option<&str>,
    ) -> Result<SttTranscription, SttServiceError> {
        let base = self
            .base_url
            .as_ref()
            .ok_or(SttServiceError::NotConfigured)?;

        let file = reqwest::multipart::Part::bytes(audio)
            .file_name(format!("audio.{}", file_extension_for(mime_type)));
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json");
        if let Some(language) = language {
            form = form.text("language", language.to_string());
        }

        let mut request = self
            .http
            .post(format!("{}/audio/transcriptions", base))
            .multipart(form);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await.map_err(|e| {
            warn!("Speech-to-text request failed: {}", e);
            SttServiceError::Unreachable(e.to_string())
        })?;

        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(SttServiceError::Upstream {
                status: status.as_u16(),
                message,
            });
        }

        response
            .json::<SttTranscription>()
            .await
            .map_err(|e| SttServiceError::Upstream {
                status: status.as_u16(),
                message: format!("Malformed transcription response: {}", e),
            })
    }
}

/// The file extension OpenAI needs to recognise an audio format. Matrix voice
/// notes are Ogg Opus, which is also the fallback for anything unrecognised.
pub fn file_extension_for(mime_type: Option<&str>) -> &'static str {
    let essence = mime_type
        .and_then(|m| m.split(';').next())
        .map(|m| m.trim().to_ascii_lowercase());

    match essence.as_deref() {
        Some("audio/mpeg") | Some("audio/mp3") => "mp3",
        Some("audio/mp4") | Some("audio/x-m4a") | Some("audio/m4a") => "m4a",
        Some("audio/wav") | Some("audio/x-wav") | Some("audio/wave") => "wav",
        Some("audio/webm") => "webm",
        Some("audio/flac") | Some("audio/x-flac") => "flac",
        _ => "ogg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::test_support::spawn_server;
    use axum::{
        extract::Multipart,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::{Arc, Mutex};

    const STT_JSON_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/stt_transcription_json.json"
    ));

    /// What the fake server saw: the Authorization header and every
    /// multipart field as (name, filename, bytes).
    #[derive(Debug, Default)]
    struct Captured {
        authorization: Option<String>,
        fields: Vec<(String, Option<String>, Vec<u8>)>,
    }

    impl Captured {
        fn field(&self, name: &str) -> Option<&(String, Option<String>, Vec<u8>)> {
            self.fields.iter().find(|(n, _, _)| n == name)
        }
    }

    /// A fake OpenAI-compatible server that records the request and answers
    /// with the shared contract. Returns the base URL including `/v1`.
    async fn capturing_server(captured: Arc<Mutex<Captured>>) -> String {
        let app = Router::new().route(
            "/v1/audio/transcriptions",
            post(move |headers: HeaderMap, mut multipart: Multipart| {
                let captured = captured.clone();
                async move {
                    let mut record = Captured {
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned),
                        fields: Vec::new(),
                    };
                    while let Some(field) = multipart.next_field().await.unwrap() {
                        let name = field.name().unwrap_or_default().to_owned();
                        let filename = field.file_name().map(str::to_owned);
                        let bytes = field.bytes().await.unwrap().to_vec();
                        record.fields.push((name, filename, bytes));
                    }
                    *captured.lock().unwrap() = record;
                    (
                        StatusCode::OK,
                        [("content-type", "application/json")],
                        STT_JSON_CONTRACT,
                    )
                }
            }),
        );
        format!("{}/v1", spawn_server(app).await)
    }

    fn contract_transcription() -> SttTranscription {
        SttTranscription {
            text: "今天天气真好。".to_string(),
            language: None,
            duration: None,
        }
    }

    #[tokio::test]
    async fn test_an_unconfigured_client_reports_not_configured() {
        let client = SttClient::new(None, None, "sensevoice-small".to_string());
        assert!(matches!(
            client.transcribe(vec![1, 2, 3], None, None).await,
            Err(SttServiceError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn test_sends_the_audio_filename_model_and_json_format() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = capturing_server(captured.clone()).await;
        let client = SttClient::new(Some(base), None, "sensevoice-small".to_string());

        let result = client
            .transcribe(vec![7, 8, 9], Some("audio/ogg; codecs=opus"), None)
            .await
            .expect("transcription should succeed");

        assert_eq!(result, contract_transcription());
        let seen = captured.lock().unwrap();
        let (_, filename, bytes) = seen.field("file").expect("a file part was sent");
        assert_eq!(bytes.as_slice(), &[7u8, 8, 9]);
        assert_eq!(filename.as_deref(), Some("audio.ogg"));
        assert_eq!(seen.field("model").unwrap().2, b"sensevoice-small");
        assert_eq!(seen.field("response_format").unwrap().2, b"json");
        assert!(
            seen.field("language").is_none(),
            "no hint means no language field"
        );
        assert!(
            seen.authorization.is_none(),
            "no key means no Authorization header"
        );
    }

    #[tokio::test]
    async fn test_sends_the_language_hint_and_bearer_token_when_given() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = capturing_server(captured.clone()).await;
        let client = SttClient::new(
            Some(base),
            Some("sk-test".to_string()),
            "whisper-1".to_string(),
        );

        client
            .transcribe(vec![1], Some("audio/mpeg"), Some("zh"))
            .await
            .expect("transcription should succeed");

        let seen = captured.lock().unwrap();
        assert_eq!(seen.authorization.as_deref(), Some("Bearer sk-test"));
        assert_eq!(seen.field("language").unwrap().2, b"zh");
        assert_eq!(seen.field("file").unwrap().1.as_deref(), Some("audio.mp3"));
    }

    #[test]
    fn test_the_shared_wire_contract_deserialises_into_stt_transcription() {
        let parsed: SttTranscription =
            serde_json::from_str(STT_JSON_CONTRACT).expect("the contract should parse");
        assert_eq!(parsed, contract_transcription());
    }

    #[tokio::test]
    async fn test_an_upstream_error_carries_its_status_and_message() {
        let app = Router::new().route(
            "/v1/audio/transcriptions",
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "models are not installed") }),
        );
        let base = format!("{}/v1", spawn_server(app).await);
        let client = SttClient::new(Some(base), None, "sensevoice-small".to_string());

        match client.transcribe(vec![1], None, None).await {
            Err(SttServiceError::Upstream { status, message }) => {
                assert_eq!(status, 503);
                assert!(message.contains("not installed"));
            }
            other => panic!("expected an upstream error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_an_unreachable_service_is_not_a_panic() {
        let client = SttClient::new(
            Some("http://127.0.0.1:1/v1".to_string()),
            None,
            "sensevoice-small".to_string(),
        );
        assert!(matches!(
            client.transcribe(vec![1], None, None).await,
            Err(SttServiceError::Unreachable(_))
        ));
    }

    #[test]
    fn test_debug_output_never_contains_the_api_key() {
        let client = SttClient::new(
            Some("https://api.openai.com/v1".to_string()),
            Some("sk-very-secret".to_string()),
            "whisper-1".to_string(),
        );
        let debug = format!("{:?}", client);
        assert!(!debug.contains("sk-very-secret"), "leaked: {}", debug);
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn test_file_extension_for_known_and_unknown_mime_types() {
        assert_eq!(file_extension_for(Some("audio/ogg; codecs=opus")), "ogg");
        assert_eq!(file_extension_for(Some("audio/mpeg")), "mp3");
        assert_eq!(file_extension_for(Some("audio/x-m4a")), "m4a");
        assert_eq!(file_extension_for(Some("audio/mp4")), "m4a");
        assert_eq!(file_extension_for(Some("AUDIO/WAV")), "wav");
        assert_eq!(file_extension_for(Some("audio/webm")), "webm");
        assert_eq!(file_extension_for(Some("audio/flac")), "flac");
        assert_eq!(file_extension_for(Some("application/x-unknown")), "ogg");
        assert_eq!(file_extension_for(None), "ogg");
    }
}
