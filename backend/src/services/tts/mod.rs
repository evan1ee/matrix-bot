//! Client for text-to-speech providers.
//!
//! Speaks OpenAI's `POST {base}/audio/speech` and asks for Ogg Opus, the
//! format that makes a Matrix client draw a voice bubble. `TtsDialect::ElevenLabs`
//! shapes the same request the way ElevenLabs wants it. The bundled
//! inference server adds two headers (`x-duration-ms`, `x-waveform`) that a
//! voice message needs and raw audio does not carry. A provider that omits
//! them is still fine: the duration is read from the Ogg stream itself, and
//! the bubble is drawn without amplitude bars.

use reqwest::header::HeaderMap;
use serde::Serialize;
use std::fmt;
use std::io::Cursor;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// Bounds a hung connection. Kokoro on a CPU takes seconds for a sentence;
/// hosted providers take less.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

const DURATION_HEADER: &str = "x-duration-ms";
const WAVEFORM_HEADER: &str = "x-waveform";
/// The largest value a Matrix waveform bucket may hold.
pub const WAVEFORM_MAX: u16 = 1024;
/// Ogg Opus granule positions are always in 48 kHz units (RFC 7845 section 4).
const OPUS_GRANULE_RATE: u64 = 48_000;

/// Synthesised audio and the metadata a Matrix voice message wants.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedSpeech {
    pub audio: Vec<u8>,
    pub duration: Option<Duration>,
    pub waveform: Option<Vec<u16>>,
}

#[derive(Debug, Error)]
pub enum TtsServiceError {
    #[error("Text-to-speech service is not configured")]
    NotConfigured,
    #[error("Text-to-speech service is unreachable: {0}")]
    Unreachable(String),
    #[error("Text-to-speech service returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

#[derive(Serialize)]
struct OpenAiSpeechRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'static str,
}

/// How a provider wants a speech request shaped. Most providers speak
/// OpenAI's shape and need only a base URL. ElevenLabs differs on four axes
/// at once (auth header, URL, where the voice goes, body field names), which
/// a base URL cannot reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TtsDialect {
    #[default]
    OpenAi,
    ElevenLabs,
}

impl TtsDialect {
    /// Parses `TTS_DIALECT`. Blank means the default.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "openai" => Ok(Self::OpenAi),
            "elevenlabs" => Ok(Self::ElevenLabs),
            other => Err(format!(
                "Unknown TTS_DIALECT '{}'; expected openai or elevenlabs",
                other
            )),
        }
    }
}

/// ElevenLabs returns this as Ogg Opus, so the granule fallback applies.
const ELEVENLABS_OUTPUT_FORMAT: &str = "opus_48000_64";

#[derive(Serialize)]
struct ElevenLabsSpeechRequest<'a> {
    text: &'a str,
    model_id: &'a str,
}

/// An absent `base_url` is a normal state: text-to-speech is not configured
/// in this deployment, and every call reports `NotConfigured` so the caller
/// can degrade that one feature.
#[derive(Clone)]
pub struct TtsClient {
    base_url: Option<String>,
    api_key: Option<String>,
    model: String,
    dialect: TtsDialect,
    http: reqwest::Client,
}

/// Written by hand so the API key never reaches a `{:?}` log line.
impl fmt::Debug for TtsClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtsClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("dialect", &self.dialect)
            .finish()
    }
}

impl TtsClient {
    pub fn new(
        base_url: Option<String>,
        api_key: Option<String>,
        model: String,
        dialect: TtsDialect,
    ) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        // reqwest 0.11 strips only Authorization/Cookie/Proxy-Authorization/
        // WWW-Authenticate on a cross-host redirect, so a custom header like
        // `xi-api-key` would still be forwarded to whatever host a 3xx points
        // at, and a 307/308 would re-post the user's text there too. Neither
        // provider needs redirects, so following them is a pure liability.
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            base_url,
            api_key,
            model,
            dialect,
            http,
        }
    }

    /// Shapes the outgoing request per `self.dialect`.
    fn build_request(
        &self,
        base: &str,
        text: &str,
        voice: &str,
    ) -> Result<reqwest::RequestBuilder, TtsServiceError> {
        match self.dialect {
            TtsDialect::OpenAi => {
                let mut request =
                    self.http
                        .post(format!("{}/audio/speech", base))
                        .json(&OpenAiSpeechRequest {
                            model: &self.model,
                            input: text,
                            voice,
                            response_format: "opus",
                        });
                if let Some(key) = &self.api_key {
                    request = request.bearer_auth(key);
                }
                Ok(request)
            }
            TtsDialect::ElevenLabs => {
                let invalid = |detail: String| {
                    TtsServiceError::Unreachable(format!(
                        "Invalid TTS_BASE_URL {}: {}",
                        base, detail
                    ))
                };
                let mut url = reqwest::Url::parse(base).map_err(|e| invalid(e.to_string()))?;
                url.path_segments_mut()
                    .map_err(|_| invalid("it cannot carry a path".to_string()))?
                    .push("text-to-speech")
                    .push(voice);
                url.query_pairs_mut()
                    .append_pair("output_format", ELEVENLABS_OUTPUT_FORMAT);

                let mut request = self.http.post(url).json(&ElevenLabsSpeechRequest {
                    text,
                    model_id: &self.model,
                });
                if let Some(key) = &self.api_key {
                    let mut value = reqwest::header::HeaderValue::from_str(key).map_err(|_| {
                        TtsServiceError::Unreachable(
                            "TTS_API_KEY is not a valid header value".to_string(),
                        )
                    })?;
                    value.set_sensitive(true);
                    request = request.header("xi-api-key", value);
                }
                Ok(request)
            }
        }
    }

    /// Speaks `text` in `voice` and returns Ogg Opus audio.
    pub async fn synthesize(
        &self,
        text: &str,
        voice: &str,
    ) -> Result<SynthesizedSpeech, TtsServiceError> {
        let base = self
            .base_url
            .as_ref()
            .ok_or(TtsServiceError::NotConfigured)?;

        let request = self.build_request(base, text, voice)?;

        let response = request.send().await.map_err(|e| {
            warn!("Text-to-speech request failed: {}", e);
            TtsServiceError::Unreachable(e.to_string())
        })?;

        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(TtsServiceError::Upstream {
                status: status.as_u16(),
                message,
            });
        }

        let headers = response.headers().clone();
        let audio = response
            .bytes()
            .await
            .map_err(|e| TtsServiceError::Unreachable(format!("Failed to read the audio: {}", e)))?
            .to_vec();

        let duration = header_duration(&headers).or_else(|| ogg_opus_duration(&audio));
        let waveform = header_waveform(&headers);
        Ok(SynthesizedSpeech {
            audio,
            duration,
            waveform,
        })
    }
}

fn header_duration(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(DURATION_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_millis)
}

/// The bundled server's waveform, or `None` if the header is absent or not a
/// list of in-range buckets. A malformed waveform is dropped, not sent: a
/// client renders a bubble without bars, but may reject an out-of-spec one.
fn header_waveform(headers: &HeaderMap) -> Option<Vec<u16>> {
    let raw = headers.get(WAVEFORM_HEADER)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.split(',')
        .map(|bar| {
            bar.trim()
                .parse::<u16>()
                .ok()
                .filter(|v| *v <= WAVEFORM_MAX)
        })
        .collect()
}

/// Reads an Ogg Opus stream's playable length from its container alone: the
/// last page's granule position, minus the `OpusHead` pre-skip, in 48 kHz
/// units. `None` for anything that is not Ogg Opus.
pub fn ogg_opus_duration(bytes: &[u8]) -> Option<Duration> {
    let mut reader = ogg::reading::PacketReader::new(Cursor::new(bytes));
    let mut pre_skip: Option<u64> = None;
    let mut last_granule: Option<u64> = None;

    while let Ok(Some(packet)) = reader.read_packet() {
        if pre_skip.is_none() {
            let data = &packet.data;
            if data.len() < 12 || &data[..8] != b"OpusHead" {
                return None;
            }
            pre_skip = Some(u64::from(u16::from_le_bytes([data[10], data[11]])));
        }
        last_granule = Some(packet.absgp_page());
    }

    let samples = last_granule?.checked_sub(pre_skip?)?;
    Some(Duration::from_millis(
        samples.checked_mul(1000)? / OPUS_GRANULE_RATE,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_support::spawn_server;
    use axum::{
        http::{HeaderMap as AxumHeaders, StatusCode},
        routing::post,
        Json, Router,
    };
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    const REQUEST_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_request.json"
    ));
    const HEADERS_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_headers.json"
    ));

    /// A minimal Ogg Opus stream: OpusHead with `pre_skip`, OpusTags, and two
    /// audio pages ending at `final_granule`. The packets are not real Opus;
    /// only the container is read.
    fn fake_ogg_opus(pre_skip: u16, final_granule: u64) -> Vec<u8> {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let mut buf = Vec::new();
        {
            let mut writer = PacketWriter::new(Cursor::new(&mut buf));
            let mut head = b"OpusHead".to_vec();
            head.push(1);
            head.push(1);
            head.extend_from_slice(&pre_skip.to_le_bytes());
            head.extend_from_slice(&24_000u32.to_le_bytes());
            head.extend_from_slice(&0i16.to_le_bytes());
            head.push(0);
            writer
                .write_packet(head, 7, PacketWriteEndInfo::EndPage, 0)
                .unwrap();
            writer
                .write_packet(
                    b"OpusTags\0\0\0\0\0\0\0\0".to_vec(),
                    7,
                    PacketWriteEndInfo::EndPage,
                    0,
                )
                .unwrap();
            writer
                .write_packet(
                    vec![0u8; 8],
                    7,
                    PacketWriteEndInfo::EndPage,
                    final_granule / 2,
                )
                .unwrap();
            writer
                .write_packet(
                    vec![0u8; 8],
                    7,
                    PacketWriteEndInfo::EndStream,
                    final_granule,
                )
                .unwrap();
        }
        buf
    }

    #[derive(Debug, Default)]
    struct Captured {
        authorization: Option<String>,
        body: serde_json::Value,
    }

    /// An OpenAI-shaped fake that records the request and answers with
    /// `audio` plus `extra_headers`. Returns the base URL including `/v1`.
    async fn fake_openai(
        captured: Arc<Mutex<Captured>>,
        audio: Vec<u8>,
        extra_headers: Vec<(String, String)>,
    ) -> String {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(
                move |headers: AxumHeaders, Json(body): Json<serde_json::Value>| {
                    let captured = captured.clone();
                    let audio = audio.clone();
                    let extra_headers = extra_headers.clone();
                    async move {
                        *captured.lock().unwrap() = Captured {
                            authorization: headers
                                .get("authorization")
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned),
                            body,
                        };
                        let mut response =
                            axum::response::Response::new(axum::body::Body::from(audio));
                        for (name, value) in extra_headers {
                            response.headers_mut().insert(
                                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                                axum::http::HeaderValue::from_str(&value).unwrap(),
                            );
                        }
                        response
                    }
                },
            ),
        );
        format!("{}/v1", spawn_server(app).await)
    }

    fn contract_headers() -> Vec<(String, String)> {
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(HEADERS_CONTRACT).unwrap();
        map.into_iter()
            .map(|(k, v)| (k, v.as_str().unwrap().to_string()))
            .collect()
    }

    #[tokio::test]
    async fn test_an_unconfigured_client_reports_not_configured() {
        let client = TtsClient::new(None, None, "kokoro".to_string(), TtsDialect::OpenAi);
        assert!(matches!(
            client.synthesize("hi", "0").await,
            Err(TtsServiceError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn test_sends_the_contract_body_and_reads_the_contract_headers() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = fake_openai(captured.clone(), b"OggS-audio".to_vec(), contract_headers()).await;
        let client = TtsClient::new(Some(base), None, "kokoro".to_string(), TtsDialect::OpenAi);
        let contract: serde_json::Value = serde_json::from_str(REQUEST_CONTRACT).unwrap();

        let speech = client
            .synthesize(
                contract["input"].as_str().unwrap(),
                contract["voice"].as_str().unwrap(),
            )
            .await
            .expect("synthesis should succeed");

        let seen = captured.lock().unwrap();
        assert_eq!(seen.body, contract);
        assert!(
            seen.authorization.is_none(),
            "no key means no Authorization header"
        );
        assert_eq!(speech.audio, b"OggS-audio");
        assert_eq!(speech.duration, Some(Duration::from_millis(1500)));
        assert_eq!(speech.waveform, Some(vec![0, 512, 1024]));
    }

    /// The path every third-party provider takes: no extension headers.
    #[tokio::test]
    async fn test_without_extension_headers_the_duration_comes_from_the_ogg_granule() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let ogg = fake_ogg_opus(312, 2 * 48_000 + 312);
        let base = fake_openai(captured, ogg, Vec::new()).await;
        let client = TtsClient::new(
            Some(base),
            Some("sk-test".to_string()),
            "tts-1".to_string(),
            TtsDialect::OpenAi,
        );

        let speech = client.synthesize("hi", "alloy").await.unwrap();

        assert_eq!(speech.duration, Some(Duration::from_secs(2)));
        assert_eq!(speech.waveform, None);
    }

    #[tokio::test]
    async fn test_sends_a_bearer_token_when_configured() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = fake_openai(captured.clone(), Vec::new(), Vec::new()).await;
        let client = TtsClient::new(
            Some(base),
            Some("sk-test".to_string()),
            "tts-1".to_string(),
            TtsDialect::OpenAi,
        );

        client.synthesize("hi", "alloy").await.unwrap();

        assert_eq!(
            captured.lock().unwrap().authorization.as_deref(),
            Some("Bearer sk-test")
        );
    }

    #[test]
    fn test_ogg_opus_duration_subtracts_pre_skip_and_rejects_non_ogg() {
        assert_eq!(
            ogg_opus_duration(&fake_ogg_opus(312, 48_000 + 312)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(ogg_opus_duration(b"ID3 an mp3, not ogg"), None);
        assert_eq!(ogg_opus_duration(&[]), None);
    }

    #[test]
    fn test_ogg_opus_duration_does_not_panic_on_an_overflowing_granule() {
        assert_eq!(ogg_opus_duration(&fake_ogg_opus(312, u64::MAX - 1)), None);
    }

    #[test]
    fn test_a_malformed_waveform_header_is_ignored_not_trusted() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(WAVEFORM_HEADER, "1,2,banana".parse().unwrap());
        assert_eq!(header_waveform(&headers), None);
        headers.insert(WAVEFORM_HEADER, "1,2,5000".parse().unwrap());
        assert_eq!(header_waveform(&headers), None, "above 1024 is out of spec");
        headers.insert(WAVEFORM_HEADER, "".parse().unwrap());
        assert_eq!(header_waveform(&headers), None);
    }

    #[tokio::test]
    async fn test_an_upstream_error_carries_its_status_and_message() {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "model is not installed") }),
        );
        let base = format!("{}/v1", spawn_server(app).await);
        let client = TtsClient::new(Some(base), None, "kokoro".to_string(), TtsDialect::OpenAi);

        match client.synthesize("hi", "0").await {
            Err(TtsServiceError::Upstream { status, message }) => {
                assert_eq!(status, 503);
                assert!(message.contains("not installed"));
            }
            other => panic!("expected an upstream error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_an_unreachable_service_is_not_a_panic() {
        let client = TtsClient::new(
            Some("http://127.0.0.1:1/v1".to_string()),
            None,
            "kokoro".to_string(),
            TtsDialect::OpenAi,
        );
        assert!(matches!(
            client.synthesize("hi", "0").await,
            Err(TtsServiceError::Unreachable(_))
        ));
    }

    #[test]
    fn test_debug_output_never_contains_the_api_key() {
        let client = TtsClient::new(
            Some("https://api.openai.com/v1".to_string()),
            Some("sk-very-secret".to_string()),
            "tts-1".to_string(),
            TtsDialect::OpenAi,
        );
        let debug = format!("{:?}", client);
        assert!(!debug.contains("sk-very-secret"), "leaked: {}", debug);
        assert!(debug.contains("redacted"));
    }

    #[derive(Debug, Default, Clone)]
    struct Seen {
        path: String,
        query: Option<String>,
        authorization: Option<String>,
        xi_api_key: Option<String>,
        body: serde_json::Value,
    }

    /// Accepts any POST path and records what arrived.
    async fn recording_server(seen: Arc<Mutex<Seen>>) -> String {
        let app = Router::new().fallback(
            move |uri: axum::http::Uri,
                  headers: AxumHeaders,
                  Json(body): Json<serde_json::Value>| {
                let seen = seen.clone();
                async move {
                    let header = |name: &str| {
                        headers
                            .get(name)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned)
                    };
                    *seen.lock().unwrap() = Seen {
                        path: uri.path().to_string(),
                        query: uri.query().map(str::to_owned),
                        authorization: header("authorization"),
                        xi_api_key: header("xi-api-key"),
                        body,
                    };
                    fake_ogg_opus(312, 48_000 + 312)
                }
            },
        );
        format!("{}/v1", spawn_server(app).await)
    }

    #[test]
    fn test_dialect_parse() {
        assert_eq!(TtsDialect::parse(""), Ok(TtsDialect::OpenAi));
        assert_eq!(TtsDialect::parse("openai"), Ok(TtsDialect::OpenAi));
        assert_eq!(
            TtsDialect::parse(" ElevenLabs "),
            Ok(TtsDialect::ElevenLabs)
        );
        assert!(TtsDialect::parse("azure").is_err());
    }

    /// The spec's dialect test: one input, two shapes on the wire.
    #[tokio::test]
    async fn test_openai_and_elevenlabs_shape_the_same_input_differently() {
        let openai_seen = Arc::new(Mutex::new(Seen::default()));
        let openai = TtsClient::new(
            Some(recording_server(openai_seen.clone()).await),
            Some("k-1".to_string()),
            "tts-1".to_string(),
            TtsDialect::OpenAi,
        );
        openai.synthesize("hello", "voice123").await.unwrap();

        let eleven_seen = Arc::new(Mutex::new(Seen::default()));
        let eleven = TtsClient::new(
            Some(recording_server(eleven_seen.clone()).await),
            Some("k-1".to_string()),
            "eleven_multilingual_v2".to_string(),
            TtsDialect::ElevenLabs,
        );
        let speech = eleven.synthesize("hello", "voice123").await.unwrap();

        let o = openai_seen.lock().unwrap().clone();
        assert_eq!(o.path, "/v1/audio/speech");
        assert_eq!(o.authorization.as_deref(), Some("Bearer k-1"));
        assert_eq!(o.xi_api_key, None);
        assert_eq!(
            o.body,
            serde_json::json!({ "model": "tts-1", "input": "hello", "voice": "voice123", "response_format": "opus" })
        );

        let e = eleven_seen.lock().unwrap().clone();
        assert_eq!(e.path, "/v1/text-to-speech/voice123");
        assert_eq!(e.query.as_deref(), Some("output_format=opus_48000_64"));
        assert_eq!(e.xi_api_key.as_deref(), Some("k-1"));
        assert_eq!(e.authorization, None);
        assert_eq!(
            e.body,
            serde_json::json!({ "text": "hello", "model_id": "eleven_multilingual_v2" })
        );
        assert_eq!(
            speech.duration,
            Some(Duration::from_secs(1)),
            "Ogg fallback applies"
        );
    }

    /// The voice is a path segment under ElevenLabs, so it must not be able
    /// to add segments of its own.
    #[tokio::test]
    async fn test_an_elevenlabs_voice_is_one_encoded_path_segment() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let client = TtsClient::new(
            Some(recording_server(seen.clone()).await),
            None,
            "m".to_string(),
            TtsDialect::ElevenLabs,
        );
        client.synthesize("hi", "a/../b").await.unwrap();
        assert_eq!(seen.lock().unwrap().path, "/v1/text-to-speech/a%2F..%2Fb");
    }

    #[derive(Debug, Default)]
    struct RedirectTarget {
        was_hit: bool,
        xi_api_key: Option<String>,
    }

    /// A 307 that would re-post the request, and carry the key, to a second
    /// host if the client followed it. `TtsClient` must not follow it.
    #[tokio::test]
    async fn test_a_redirect_is_not_followed_and_never_carries_the_key() {
        let b_seen = Arc::new(Mutex::new(RedirectTarget::default()));
        let b_app = Router::new().fallback({
            let b_seen = b_seen.clone();
            move |headers: AxumHeaders| {
                let b_seen = b_seen.clone();
                async move {
                    *b_seen.lock().unwrap() = RedirectTarget {
                        was_hit: true,
                        xi_api_key: headers
                            .get("xi-api-key")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned),
                    };
                    StatusCode::OK
                }
            }
        });
        let b_base = spawn_server(b_app).await;
        let location = format!("{}/v1/text-to-speech/v", b_base);

        let a_app = Router::new().route(
            "/v1/text-to-speech/v",
            post(move || {
                let location = location.clone();
                async move {
                    axum::response::Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header("Location", location)
                        .body(axum::body::Body::empty())
                        .unwrap()
                }
            }),
        );
        let a_base = format!("{}/v1", spawn_server(a_app).await);

        let client = TtsClient::new(
            Some(a_base),
            Some("k-secret".to_string()),
            "m".to_string(),
            TtsDialect::ElevenLabs,
        );

        match client.synthesize("hi", "v").await {
            Err(TtsServiceError::Upstream { status, .. }) => assert_eq!(status, 307),
            other => panic!("expected a 307 upstream error, got {:?}", other),
        }

        let seen = b_seen.lock().unwrap();
        assert!(!seen.was_hit, "the redirect target must never be contacted");
        assert_eq!(seen.xi_api_key, None);
    }
}
