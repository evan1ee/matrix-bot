//! Text-to-speech: `!say <text>` replies with a spoken voice message.
//!
//! The speaking happens elsewhere, through inference services: the bundled
//! inference server's Kokoro, or any OpenAI-compatible or ElevenLabs
//! provider, chosen by config. This module is the bot's half: preparing
//! the text (`text.rs`), deciding whose voice speaks, and sending the result
//! into the room as a voice message.

use async_trait::async_trait;
use tracing::warn;

use crate::db::Database;
use crate::infrastructure::inference::tts::{TtsClient, TtsServiceError};
use crate::interface::channels::message::{Attachment, AttachmentKind, Reply};
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape;

pub mod text;

/// Matches the other modules' 500-character command-input ceiling.
pub const MAX_SPEAK_CHARS: usize = 500;

/// The voice `!say` speaks in: the sender's own choice, or the operator's
/// `TTS_VOICE` for someone who never chose.
fn resolve_voice(house_default: &str, saved: Option<&str>) -> String {
    saved.unwrap_or(house_default).to_string()
}

/// What the sender is told when synthesis fails. Fixed text: the upstream
/// body can carry internal URLs and engine detail, which is logged instead.
fn failure_message(err: &TtsServiceError) -> String {
    match err {
        TtsServiceError::Upstream {
            status: 400 | 404 | 422,
            ..
        } => "⚠️ <b>语音合成失败</b><br/>\
             • <b>原因:</b> 语音合成服务拒绝了这段文字或所选的声音<br/>\
             • <b>建议:</b> 发送 <code>!settings voice none</code> 恢复默认声音后重试。"
            .to_string(),
        TtsServiceError::NotConfigured => "⚠️ <b>语音合成不可用</b><br/>\
             • <b>原因:</b> 本部署未配置语音合成服务<br/>\
             • <b>建议:</b> 请联系管理员设置 <code>TTS_BASE_URL</code>。"
            .to_string(),
        TtsServiceError::Unreachable(_) | TtsServiceError::Upstream { .. } => {
            "⚠️ <b>语音合成不可用</b><br/>\
             • <b>原因:</b> 语音合成服务当前不可用<br/>\
             • <b>建议:</b> 请稍后重试。"
                .to_string()
        }
    }
}

pub struct TtsModule {
    tts: TtsClient,
    default_voice: String,
}

impl TtsModule {
    pub fn new(tts: TtsClient, default_voice: String) -> Self {
        Self { tts, default_voice }
    }

    async fn handle_say(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
    ) -> Reply {
        let text = text::speakable(ctx.raw_args.trim());
        if text.is_empty() {
            return Reply::from(
                "⚠️ <b>没有可朗读的文字</b><br/>\
                 • <b>用法:</b> <code>!say &lt;文字&gt;</code>"
                    .to_string(),
            );
        }
        if text.chars().count() > MAX_SPEAK_CHARS {
            return Reply::from(format!(
                "⚠️ <b>文字过长</b><br/>\
                 • <b>原因:</b> 超过 {} 字符上限，请缩短后重试。",
                MAX_SPEAK_CHARS
            ));
        }

        let snapshot = services.app_config.snapshot();
        if snapshot.is_customized(crate::domain::config::SettingKey::InferenceTtsEnabled)
            && !snapshot.inference_tts_enabled()
        {
            return Reply::from(failure_message(&TtsServiceError::NotConfigured));
        }

        let house_voice =
            if snapshot.is_customized(crate::domain::config::SettingKey::InferenceTtsVoice) {
                snapshot.inference_tts_voice()
            } else {
                &self.default_voice
            };
        let settings = services.settings.get_or_default(&ctx.owner_id).await;
        let voice = resolve_voice(house_voice, settings.tts_voice.as_deref());

        let base_url = Some(snapshot.inference_tts_base_url()).filter(|s| !s.is_empty());
        let model = if snapshot.is_customized(crate::domain::config::SettingKey::InferenceTtsModel)
        {
            Some(snapshot.inference_tts_model())
        } else {
            None
        };
        let dialect =
            if snapshot.is_customized(crate::domain::config::SettingKey::InferenceTtsDialect) {
                crate::infrastructure::inference::tts::TtsDialect::parse(
                    snapshot.inference_tts_dialect(),
                )
                .ok()
            } else {
                None
            };
        let api_key = match services
            .secrets
            .get_decrypted_secret(crate::domain::secrets::SecretProvider::Tts, "default")
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

        let effective_tts = self.tts.with_overrides(base_url, api_key, model, dialect);

        let speech = match effective_tts.synthesize(&text, &voice).await {
            Ok(speech) => speech,
            Err(err) => {
                warn!("TTS synthesis failed: {}", err);
                return Reply::from(failure_message(&err));
            }
        };

        // The adapter delivers the clip as a voice message in whatever form its
        // platform has; the caption is the reply text.
        Reply::html(format!("🔊 <b>已朗读:</b> “{}”", html_escape(&text))).with_attachment(
            Attachment {
                kind: AttachmentKind::Audio,
                mime: "audio/ogg".to_owned(),
                filename: "voice-message.ogg".to_owned(),
                duration_ms: speech.duration.map(|d| d.as_millis() as u64),
                waveform: speech.waveform,
                bytes: speech.audio,
            },
        )
    }
}

#[async_trait]
impl BotModule for TtsModule {
    fn name(&self) -> &str {
        "Text-to-Speech"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![CommandHelp {
            name: "say".to_string(),
            usage: "!say <text>".to_string(),
            description: "Speak a line of text back as a voice message".to_string(),
        }]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        if ctx.command != "say" {
            return None;
        }

        Some(self.handle_say(ctx, services).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::tts::TtsDialect;
    use std::sync::{Arc, Mutex};

    fn module_with(tts: TtsClient, default_voice: &str) -> TtsModule {
        TtsModule::new(tts, default_voice.to_string())
    }

    fn unconfigured() -> TtsClient {
        TtsClient::new(None, None, "kokoro".to_string(), TtsDialect::OpenAi)
    }

    fn ctx_for(command: &str, raw_args: &str) -> CommandContext {
        crate::interface::chat::test_support::context(
            command,
            raw_args,
            &format!("!{} {}", command, raw_args),
        )
    }

    async fn memory_db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        Database { pool }
    }

    #[test]
    fn test_advertises_the_say_command() {
        let module = module_with(unconfigured(), "0");
        assert_eq!(module.name(), "Text-to-Speech");
        let commands = module.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "say");
    }

    #[tokio::test]
    async fn test_ignores_commands_it_does_not_claim() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        assert!(module
            .handle_command(
                &ctx_for("todo", "list"),
                &crate::application::AppServices::new(&db.pool),
                &db
            )
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_an_unconfigured_service_degrades_with_a_message() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let reply = module
            .handle_command(
                &ctx_for("say", "hello there"),
                &crate::application::AppServices::new(&db.pool),
                &db,
            )
            .await
            .unwrap();
        assert!(reply.html_body().contains("语音合成不可用"));
    }

    #[tokio::test]
    async fn test_emoji_only_text_is_refused_before_any_request() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let reply = module
            .handle_command(
                &ctx_for("say", "✅🎉"),
                &crate::application::AppServices::new(&db.pool),
                &db,
            )
            .await
            .unwrap();
        assert!(reply.html_body().contains("没有可朗读的文字"));
    }

    #[tokio::test]
    async fn test_overlong_text_is_refused_before_any_request() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let long = "字".repeat(MAX_SPEAK_CHARS + 1);
        let reply = module
            .handle_command(
                &ctx_for("say", &long),
                &crate::application::AppServices::new(&db.pool),
                &db,
            )
            .await
            .unwrap();
        assert!(reply.html_body().contains("文字过长"));
    }

    /// Synthesis succeeds against a fake provider; the module answers with the
    /// caption and the audio as an attachment, and sends nothing itself. The
    /// request proves which voice was resolved.
    #[tokio::test]
    async fn test_a_user_without_a_saved_voice_is_spoken_in_the_house_default() {
        use axum::{routing::post, Json, Router};
        let voice_seen = Arc::new(Mutex::new(None::<String>));
        let seen = voice_seen.clone();
        let app = Router::new().route(
            "/v1/audio/speech",
            post(move |Json(body): Json<serde_json::Value>| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = body["voice"].as_str().map(str::to_owned);
                    vec![1u8, 2, 3]
                }
            }),
        );
        let base = format!(
            "{}/v1",
            crate::infrastructure::inference::test_support::spawn_server(app).await
        );
        let tts = TtsClient::new(Some(base), None, "kokoro".to_string(), TtsDialect::OpenAi);
        let module = module_with(tts, "7");
        let db = memory_db().await;

        let reply = module
            .handle_command(
                &ctx_for("say", "hello"),
                &crate::application::AppServices::new(&db.pool),
                &db,
            )
            .await
            .unwrap();

        assert_eq!(reply.html_body(), "🔊 <b>已朗读:</b> “hello”");
        let [audio] = reply.content.attachments.as_slice() else {
            panic!("expected exactly one attachment");
        };
        assert_eq!(audio.kind, AttachmentKind::Audio);
        assert_eq!(audio.mime, "audio/ogg");
        assert_eq!(audio.filename, "voice-message.ogg");
        assert_eq!(audio.bytes, vec![1u8, 2, 3]);
        assert_eq!(voice_seen.lock().unwrap().as_deref(), Some("7"));
    }

    #[test]
    fn test_failure_messages_never_echo_the_upstream_body() {
        let leaky = "http://inference:8090/v1/audio/speech sherpa-internal-detail";
        let cases = [
            (TtsServiceError::NotConfigured, "语音合成不可用"),
            (
                TtsServiceError::Unreachable(leaky.to_string()),
                "语音合成不可用",
            ),
            (
                TtsServiceError::Upstream {
                    status: 503,
                    message: leaky.to_string(),
                },
                "语音合成不可用",
            ),
            (
                TtsServiceError::Upstream {
                    status: 400,
                    message: leaky.to_string(),
                },
                "!settings voice none",
            ),
            (
                TtsServiceError::Upstream {
                    status: 422,
                    message: leaky.to_string(),
                },
                "!settings voice none",
            ),
        ];
        for (err, expected) in cases {
            let reply = failure_message(&err);
            assert!(reply.contains(expected), "{:?} -> {}", err, reply);
            assert!(
                !reply.contains("sherpa-internal-detail"),
                "leaked: {}",
                reply
            );
        }
    }

    #[test]
    fn test_resolve_voice_prefers_the_saved_choice() {
        assert_eq!(resolve_voice("5", None), "5");
        assert_eq!(resolve_voice("5", Some("77")), "77");
    }

    /// Speaker 0 is a real voice: an explicit 0 must win over a non-zero
    /// house default.
    #[test]
    fn test_resolve_voice_honours_an_explicit_zero() {
        assert_eq!(resolve_voice("44", Some("0")), "0");
    }
}
