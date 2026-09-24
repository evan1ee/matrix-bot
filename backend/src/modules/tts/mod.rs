//! Text-to-speech: `!say <text>` replies with a spoken voice message.
//!
//! The speaking happens elsewhere, through `services::tts`: the bundled
//! inference server's Kokoro, or any OpenAI-compatible or ElevenLabs
//! provider, chosen by config. This module is the bot's half: preparing
//! the text (`text.rs`), deciding whose voice speaks, and sending the result
//! into the room as a voice message.

use async_trait::async_trait;
use matrix_sdk::attachment::{AttachmentConfig, AttachmentInfo, BaseAudioInfo};
use matrix_sdk::ruma::{RoomId, UInt};
use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, warn};

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::settings::SettingsRepository;
use crate::modules::{BotModule, CommandContext, CommandHelp};
use crate::services::tts::{SynthesizedSpeech, TtsClient, TtsServiceError};
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
    /// The shared handle to the bot's own Matrix client. `AiModule` and
    /// `AccountModule` reach Matrix the same way; see `modules/ai/mod.rs`.
    client: Arc<RwLock<Option<Client>>>,
    tts: TtsClient,
    default_voice: String,
}

impl TtsModule {
    pub fn new(client: Arc<RwLock<Option<Client>>>, tts: TtsClient, default_voice: String) -> Self {
        Self {
            client,
            tts,
            default_voice,
        }
    }

    async fn handle_say(&self, ctx: &CommandContext, db: &Database) -> String {
        let text = text::speakable(ctx.raw_args.trim());
        if text.is_empty() {
            return "⚠️ <b>没有可朗读的文字</b><br/>\
                    • <b>用法:</b> <code>!say &lt;文字&gt;</code>"
                .to_string();
        }
        if text.chars().count() > MAX_SPEAK_CHARS {
            return format!(
                "⚠️ <b>文字过长</b><br/>\
                 • <b>原因:</b> 超过 {} 字符上限，请缩短后重试。",
                MAX_SPEAK_CHARS
            );
        }

        let settings = SettingsRepository::get_or_default(&db.pool, &ctx.owner_id).await;
        let voice = resolve_voice(&self.default_voice, settings.tts_voice.as_deref());

        let speech = match self.tts.synthesize(&text, &voice).await {
            Ok(speech) => speech,
            Err(err) => {
                warn!("TTS synthesis failed: {}", err);
                return failure_message(&err);
            }
        };

        match self.send_voice_message(&ctx.room_id, speech).await {
            Ok(()) => format!("🔊 <b>已朗读:</b> “{}”", html_escape(&text)),
            Err(err) => {
                error!("Failed to send TTS voice message: {}", err);
                "⚠️ <b>语音发送失败</b><br/>\
                 • <b>原因:</b> 语音已生成，但发送到房间时出错<br/>\
                 • <b>建议:</b> 请稍后重试。"
                    .to_string()
            }
        }
    }

    /// Uploads the Ogg Opus clip and sends it as a voice message.
    ///
    /// `AttachmentInfo::Voice` is what makes a Matrix client draw a voice
    /// bubble rather than a file card. The waveform is `None` when the
    /// provider sent none, and a client then draws the bubble without bars.
    async fn send_voice_message(
        &self,
        room_id: &str,
        speech: SynthesizedSpeech,
    ) -> Result<(), String> {
        let client = self.client.read().await.clone();
        let client = client.ok_or_else(|| "Matrix client is not connected".to_string())?;

        let parsed_room_id = <&RoomId>::try_from(room_id)
            .map_err(|_| format!("Invalid Matrix room ID: {}", room_id))?;
        let room = client
            .get_room(parsed_room_id)
            .ok_or_else(|| format!("Bot is not joined to room {}", room_id))?;

        let content_type = "audio/ogg"
            .parse::<mime_guess::mime::Mime>()
            .map_err(|e| format!("Invalid MIME type: {}", e))?;

        let info = AttachmentInfo::Voice {
            audio_info: BaseAudioInfo {
                duration: speech.duration,
                size: UInt::new(speech.audio.len() as u64),
            },
            waveform: speech.waveform,
        };

        room.send_attachment(
            "voice-message.ogg",
            &content_type,
            speech.audio,
            AttachmentConfig::new().info(info),
        )
        .await
        .map_err(|e| format!("Matrix upload/send failed: {:?}", e))?;

        Ok(())
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
        db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        if ctx.command != "say" {
            return None;
        }

        Some(self.handle_say(ctx, db).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::tts::TtsDialect;
    use std::sync::Mutex;

    fn module_with(tts: TtsClient, default_voice: &str) -> TtsModule {
        TtsModule::new(Arc::new(RwLock::new(None)), tts, default_voice.to_string())
    }

    fn unconfigured() -> TtsClient {
        TtsClient::new(None, None, "kokoro".to_string(), TtsDialect::OpenAi)
    }

    fn ctx_for(command: &str, raw_args: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            owner_id: "@alice:a".to_string(),
            sender_mxid: "@alice:a".to_string(),
            sender_name: "alice".to_string(),
            command: command.to_string(),
            raw_args: raw_args.to_string(),
            full_body: format!("!{} {}", command, raw_args),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn memory_db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
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
            .handle_command(&ctx_for("todo", "list"), &db, None)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_an_unconfigured_service_degrades_with_a_message() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let reply = module
            .handle_command(&ctx_for("say", "hello there"), &db, None)
            .await
            .unwrap();
        assert!(reply.contains("语音合成不可用"), "{}", reply);
    }

    #[tokio::test]
    async fn test_emoji_only_text_is_refused_before_any_request() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let reply = module
            .handle_command(&ctx_for("say", "✅🎉"), &db, None)
            .await
            .unwrap();
        assert!(reply.contains("没有可朗读的文字"), "{}", reply);
    }

    #[tokio::test]
    async fn test_overlong_text_is_refused_before_any_request() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let long = "字".repeat(MAX_SPEAK_CHARS + 1);
        let reply = module
            .handle_command(&ctx_for("say", &long), &db, None)
            .await
            .unwrap();
        assert!(reply.contains("文字过长"), "{}", reply);
    }

    /// Synthesis succeeds against a fake provider; sending then fails only
    /// because no Matrix client is connected in tests. The request proves
    /// which voice was resolved.
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
                    Vec::<u8>::new()
                }
            }),
        );
        let base = format!(
            "{}/v1",
            crate::services::test_support::spawn_server(app).await
        );
        let tts = TtsClient::new(Some(base), None, "kokoro".to_string(), TtsDialect::OpenAi);
        let module = module_with(tts, "7");
        let db = memory_db().await;

        let reply = module
            .handle_command(&ctx_for("say", "hello"), &db, None)
            .await
            .unwrap();

        assert!(reply.contains("语音发送失败"), "{}", reply);
        assert!(
            !reply.contains("Matrix client is not connected"),
            "leaked internal error detail: {}",
            reply
        );
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
