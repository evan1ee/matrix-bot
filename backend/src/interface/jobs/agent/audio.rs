//! Direct Kokoro TTS audio digest compilation.
//!
//! Extracts audio briefing scripts from completed agent runs (e.g. weekly bookmark
//! digests), synthesizes them via TTS (fail-open), and prepares audio attachments.

use tracing::warn;

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::config::SettingKey;
use crate::domain::secrets::SecretProvider;
use crate::infrastructure::inference::tts::{TtsClient, TtsDialect};
use crate::interface::channels::message::{Attachment, AttachmentKind};

/// Case-insensitively extracts the script inside `<audio_script>...</audio_script>`.
///
/// Returns `Some(trimmed_script)` if present and non-empty; otherwise `None`.
pub fn extract_audio_script(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let start_tag = "<audio_script>";
    let end_tag = "</audio_script>";
    let start_idx = lower.find(start_tag)?;
    let content_start = start_idx + start_tag.len();
    let end_idx = lower[content_start..].find(end_tag)?;
    let content_end = content_start + end_idx;
    let script = text[content_start..content_end].trim();
    if script.is_empty() {
        None
    } else {
        Some(script.to_string())
    }
}

/// Case-insensitively removes all `<audio_script>...</audio_script>` blocks from `text`.
///
/// Returns the trimmed remaining text. If no `<audio_script>` tags exist, returns the original text.
pub fn strip_audio_script(text: &str) -> String {
    let start_tag = "<audio_script>";
    let end_tag = "</audio_script>";
    let mut current = text;
    let mut output = String::new();
    let mut stripped_any = false;

    loop {
        let lower = current.to_ascii_lowercase();
        let Some(start_idx) = lower.find(start_tag) else {
            break;
        };
        let content_start = start_idx + start_tag.len();
        if let Some(end_offset) = lower[content_start..].find(end_tag) {
            output.push_str(&current[..start_idx]);
            let block_end = content_start + end_offset + end_tag.len();
            current = &current[block_end..];
            stripped_any = true;
        } else {
            break;
        }
    }

    if stripped_any {
        output.push_str(current);
        let trimmed = output.trim();
        let mut cleaned = String::with_capacity(trimmed.len());
        let mut consecutive_newlines = 0;
        for ch in trimmed.chars() {
            if ch == '\n' {
                consecutive_newlines += 1;
                if consecutive_newlines <= 2 {
                    cleaned.push(ch);
                }
            } else if ch == '\r' {
                if consecutive_newlines < 2 {
                    cleaned.push(ch);
                }
            } else {
                consecutive_newlines = 0;
                cleaned.push(ch);
            }
        }
        cleaned
    } else {
        text.to_string()
    }
}

/// Compiles an extracted audio script into an Ogg Opus [`Attachment`] using Kokoro TTS.
///
/// Gracefully degrades and returns `None` if TTS is unconfigured, disabled, empty, or fails.
pub async fn compile_audio_digest(
    services: &AppServices,
    owner: &AccountId,
    script: &str,
    custom_voice: Option<&str>,
) -> Option<Attachment> {
    let snapshot = services.app_config.snapshot();
    if snapshot.is_customized(SettingKey::InferenceTtsEnabled) && !snapshot.inference_tts_enabled() {
        return None;
    }

    let base_url = Some(snapshot.inference_tts_base_url()).filter(|s| !s.is_empty())?;

    let model = if snapshot.is_customized(SettingKey::InferenceTtsModel) {
        Some(snapshot.inference_tts_model())
    } else {
        None
    };

    let dialect = if snapshot.is_customized(SettingKey::InferenceTtsDialect) {
        TtsDialect::parse(snapshot.inference_tts_dialect()).ok()
    } else {
        None
    };

    let api_key = match services
        .secrets
        .get_decrypted_secret(SecretProvider::Tts, "default")
        .await
    {
        Ok(sec) => sec.map(|s| s.expose_secret().to_string()),
        Err(e) => {
            warn!(
                error = %e,
                "Failed to decrypt TTS secret from vault; falling back to default"
            );
            None
        }
    };

    let house_voice = if snapshot.is_customized(SettingKey::InferenceTtsVoice) {
        snapshot.inference_tts_voice()
    } else {
        "alloy"
    };

    let settings = services.settings.get_or_default(owner).await;
    let custom_voice = custom_voice.filter(|v| !v.trim().is_empty());
    let voice = custom_voice.unwrap_or_else(|| settings.tts_voice.as_deref().unwrap_or(house_voice));

    let speakable = crate::interface::chat::tts::text::speakable(script);
    if speakable.is_empty() {
        return None;
    }

    let base_client = TtsClient::new(None, None, "kokoro".to_string(), TtsDialect::OpenAi);
    let tts = base_client.with_overrides(Some(base_url), api_key, model, dialect);

    match tts.synthesize(&speakable, voice).await {
        Ok(speech) => Some(Attachment {
            kind: AttachmentKind::Audio,
            mime: "audio/ogg".to_string(),
            filename: "weekly-digest.ogg".to_string(),
            bytes: speech.audio,
            duration_ms: speech.duration.map(|d| d.as_millis() as u64),
            waveform: speech.waveform,
        }),
        Err(err) => {
            warn!(
                error = %err,
                "Failed to synthesize audio digest via TTS; falling back to text"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::test_support::spawn_server;
    use axum::{http::StatusCode, routing::post, Router};

    #[test]
    fn test_extract_audio_script_valid() {
        let text = "Here is the digest:\n\n<audio_script>Welcome to your weekly podcast briefing!</audio_script>\n\nHope you enjoyed.";
        assert_eq!(
            extract_audio_script(text),
            Some("Welcome to your weekly podcast briefing!".to_string())
        );

        let multiline = "<audio_script>\n  Line 1 of podcast.\n  Line 2 of podcast.\n</audio_script>";
        assert_eq!(
            extract_audio_script(multiline),
            Some("Line 1 of podcast.\n  Line 2 of podcast.".to_string())
        );
    }

    #[test]
    fn test_extract_audio_script_case_insensitive() {
        let upper = "Intro <AUDIO_SCRIPT>All uppercase tags</AUDIO_SCRIPT> Outro";
        assert_eq!(
            extract_audio_script(upper),
            Some("All uppercase tags".to_string())
        );

        let mixed = "Intro <Audio_Script>Mixed case tags</audio_script> Outro";
        assert_eq!(
            extract_audio_script(mixed),
            Some("Mixed case tags".to_string())
        );
    }

    #[test]
    fn test_extract_audio_script_none_when_missing_or_empty() {
        assert_eq!(extract_audio_script("No audio script in this text."), None);
        assert_eq!(extract_audio_script("<audio_script></audio_script>"), None);
        assert_eq!(
            extract_audio_script("<audio_script>   \n\t   </audio_script>"),
            None
        );
        assert_eq!(
            extract_audio_script("<audio_script>Unclosed script"),
            None
        );
        assert_eq!(
            extract_audio_script("Unopened script</audio_script>"),
            None
        );
    }

    #[test]
    fn test_strip_audio_script_removes_block_cleanly() {
        let text = "Here is the digest:\n\n<audio_script>Secret audio script</audio_script>\n\nHope you enjoyed.";
        let stripped = strip_audio_script(text);
        assert_eq!(
            stripped,
            "Here is the digest:\n\nHope you enjoyed."
        );
    }

    #[test]
    fn test_strip_audio_script_case_insensitive() {
        let text = "Prefix <AUDIO_SCRIPT>Secret</AUDIO_SCRIPT> Suffix";
        assert_eq!(strip_audio_script(text), "Prefix  Suffix");
    }

    #[test]
    fn test_strip_audio_script_no_tags_returns_original() {
        let text = "Simple text without tags.";
        assert_eq!(strip_audio_script(text), text);

        let unclosed = "Text with <audio_script> unclosed.";
        assert_eq!(strip_audio_script(unclosed), unclosed);
    }

    #[tokio::test]
    async fn test_compile_audio_digest_unconfigured_returns_none() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_tts_unconfigured");
        crate::db::pool::seed_account(&pool, &owner).await;

        let result = compile_audio_digest(&services, &owner, "Hello world", None).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_compile_audio_digest_disabled_returns_none() {
        use crate::domain::config::SettingValue;
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_tts_disabled");
        crate::db::pool::seed_account(&pool, &owner).await;

        services
            .app_config
            .update_setting(
                &owner,
                SettingKey::InferenceTtsEnabled,
                SettingValue::Boolean(false),
            )
            .await
            .unwrap();
        services
            .app_config
            .update_setting(
                &owner,
                SettingKey::InferenceTtsBaseUrl,
                SettingValue::String("http://127.0.0.1:9999".to_string()),
            )
            .await
            .unwrap();

        let result = compile_audio_digest(&services, &owner, "Hello world", None).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_compile_audio_digest_empty_or_emoji_only_returns_none() {
        use crate::domain::config::SettingValue;
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_tts_emoji");
        crate::db::pool::seed_account(&pool, &owner).await;

        services
            .app_config
            .update_setting(
                &owner,
                SettingKey::InferenceTtsBaseUrl,
                SettingValue::String("http://127.0.0.1:9999".to_string()),
            )
            .await
            .unwrap();

        let result = compile_audio_digest(&services, &owner, "🔊 ✅ ❓", None).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_compile_audio_digest_success() {
        use crate::domain::config::SettingValue;
        let fake_audio = b"fake-ogg-opus-bytes".to_vec();
        let app = Router::new().route(
            "/audio/speech",
            post({
                let audio = fake_audio.clone();
                move || {
                    let audio = audio.clone();
                    async move {
                        let mut resp = axum::response::Response::new(axum::body::Body::from(audio));
                        resp.headers_mut().insert(
                            "x-duration-ms",
                            axum::http::HeaderValue::from_static("4500"),
                        );
                        resp.headers_mut().insert(
                            "x-waveform",
                            axum::http::HeaderValue::from_static("100,200,300"),
                        );
                        resp
                    }
                }
            }),
        );
        let base_url = spawn_server(app).await;

        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_tts_success");
        crate::db::pool::seed_account(&pool, &owner).await;

        services
            .app_config
            .update_setting(
                &owner,
                SettingKey::InferenceTtsBaseUrl,
                SettingValue::String(base_url),
            )
            .await
            .unwrap();

        let result = compile_audio_digest(
            &services,
            &owner,
            "Welcome to this week's bookmarks digest!",
            Some("custom-voice-1"),
        )
        .await;

        let attachment = result.expect("should produce audio attachment");
        assert_eq!(attachment.kind, AttachmentKind::Audio);
        assert_eq!(attachment.mime, "audio/ogg");
        assert_eq!(attachment.filename, "weekly-digest.ogg");
        assert_eq!(attachment.bytes, fake_audio);
        assert_eq!(attachment.duration_ms, Some(4500));
        assert_eq!(attachment.waveform, Some(vec![100, 200, 300]));
    }

    #[tokio::test]
    async fn test_compile_audio_digest_fallback_on_tts_failure() {
        use crate::domain::config::SettingValue;
        let app = Router::new().route(
            "/audio/speech",
            post(|| async {
                (StatusCode::INTERNAL_SERVER_ERROR, "TTS engine crashed")
            }),
        );
        let base_url = spawn_server(app).await;

        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed("test_tts_fail");
        crate::db::pool::seed_account(&pool, &owner).await;

        services
            .app_config
            .update_setting(
                &owner,
                SettingKey::InferenceTtsBaseUrl,
                SettingValue::String(base_url),
            )
            .await
            .unwrap();

        let result = compile_audio_digest(
            &services,
            &owner,
            "This will fail TTS synthesis",
            None,
        )
        .await;

        assert!(result.is_none(), "should fail-open and return None on upstream error");
    }
}
