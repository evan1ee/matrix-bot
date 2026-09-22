//! Text-to-speech: `!say <text>` replies with a spoken voice message.
//!
//! One engine, one code path, no trait — matching `modules/voice` (sherpa
//! only). `modules/ocr` used to be this shape too (PaddleOCR only); it is now
//! an HTTP client to the inference crate's `POST /ocr`, not an engine at all,
//! so the comparison no longer applies to it. This module is sherpa-onnx's
//! Kokoro TTS engine and nothing else; see `engine.rs` for why it differs from
//! `modules/voice/sherpa.rs` in loading its model once instead of per call.

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
use crate::utils::html::html_escape;

pub mod engine;
pub mod models;
pub mod opus;
pub mod text;

pub use models::{Speech, TtsConfig, TtsError};

/// Matches the other modules' 500-character command-input ceiling.
pub const MAX_SPEAK_CHARS: usize = 500;

/// How many buckets Matrix's `waveform` field expects for a voice message.
const WAVEFORM_BUCKETS: usize = 100;

/// The largest value a waveform bucket may hold, per the Matrix spec.
const WAVEFORM_MAX: f32 = 1024.0;

/// Resolves which speaker id `!say` should use: the sender's own preference,
/// or `env_fallback` (`TTS_SPEAKER_ID`) when they have never customised it.
fn resolve_speaker_id(env_fallback: i32, saved: Option<i64>) -> i64 {
    saved.unwrap_or(i64::from(env_fallback))
}

pub struct TtsModule {
    /// The shared handle to the bot's own Matrix client. `AiModule` and
    /// `AccountModule` reach Matrix the same way; see `modules/ai/mod.rs`.
    client: Arc<RwLock<Option<Client>>>,
    config: TtsConfig,
}

impl TtsModule {
    pub fn new(client: Arc<RwLock<Option<Client>>>, config: TtsConfig) -> Self {
        Self { client, config }
    }

    async fn handle_say(&self, ctx: &CommandContext, db: &Database) -> String {
        if !self.config.enabled {
            return "⚠️ <b>语音合成未启用</b><br/>\
                    • <b>原因:</b> 管理员尚未开启文本转语音功能<br/>\
                    • <b>建议:</b> 设置 <code>TTS_ENABLED=true</code> 并重启机器人。"
                .to_string();
        }

        if !engine::model_present(&self.config.model_dir) {
            return format!(
                "⚠️ <b>语音合成模型缺失</b><br/>\
                 • <b>原因:</b> 未在 <code>{}</code> 找到 Kokoro TTS 模型文件<br/>\
                 • <b>建议:</b> 请下载模型后重启机器人。",
                html_escape(&self.config.model_dir.display().to_string())
            );
        }

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

        // TTS_SPEAKER_ID (`self.config.speaker_id`) is the operator's house
        // default — a sensible voice (e.g. Mandarin) for a room full of people
        // who have never picked their own. `resolve_speaker_id` decides
        // between it and the sender's saved `tts_speaker_id`.
        let settings = SettingsRepository::get_or_default(&db.pool, &ctx.owner_id).await;
        let speaker_id = resolve_speaker_id(self.config.speaker_id, settings.tts_speaker_id);

        let speech = match engine::synthesize(&self.config, &text, speaker_id).await {
            Ok(speech) => speech,
            Err(err) => {
                warn!("TTS synthesis failed: {:?}", err);
                return format!(
                    "⚠️ <b>语音合成失败</b><br/>\
                     • <b>原因:</b> {}",
                    html_escape(&err.to_string())
                );
            }
        };

        match self.send_voice_message(&ctx.room_id, &speech).await {
            Ok(()) => format!("🔊 <b>已朗读:</b> “{}”", html_escape(&text)),
            Err(err) => {
                error!("Failed to send TTS voice message: {}", err);
                format!(
                    "⚠️ <b>语音发送失败</b><br/>\
                     • <b>原因:</b> {}",
                    html_escape(&err)
                )
            }
        }
    }

    /// Resolves the room the command arrived in from the bot's own client
    /// handle, exactly as `bot/notify.rs` and `modules/webhook/scheduler.rs`
    /// do, then uploads and sends the Ogg Opus clip as a voice message.
    ///
    /// Ogg Opus, per RFC 7845: combined with `AttachmentInfo::Voice` carrying
    /// a duration and waveform, this is what makes Matrix clients render the
    /// message as a voice bubble rather than a plain audio attachment.
    async fn send_voice_message(&self, room_id: &str, speech: &Speech) -> Result<(), String> {
        let client = self.client.read().await.clone();
        let client = client.ok_or_else(|| "Matrix client is not connected".to_string())?;

        let parsed_room_id = <&RoomId>::try_from(room_id)
            .map_err(|_| format!("Invalid Matrix room ID: {}", room_id))?;
        let room = client
            .get_room(parsed_room_id)
            .ok_or_else(|| format!("Bot is not joined to room {}", room_id))?;

        let ogg_bytes = opus::encode_ogg_opus(&speech.samples, speech.sample_rate)
            .map_err(|e| format!("Ogg Opus encoding failed: {}", e))?;

        let content_type = "audio/ogg"
            .parse::<mime_guess::mime::Mime>()
            .map_err(|e| format!("Invalid MIME type: {}", e))?;

        let info = AttachmentInfo::Voice {
            audio_info: BaseAudioInfo {
                duration: Some(speech.duration()),
                size: UInt::new(ogg_bytes.len() as u64),
            },
            waveform: Some(compute_waveform(&speech.samples)),
        };

        room.send_attachment(
            "voice-message.ogg",
            &content_type,
            ogg_bytes,
            AttachmentConfig::new().info(info),
        )
        .await
        .map_err(|e| format!("Matrix upload/send failed: {:?}", e))?;

        Ok(())
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

    fn module_with(config: TtsConfig) -> TtsModule {
        TtsModule::new(Arc::new(RwLock::new(None)), config)
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
        let module = module_with(TtsConfig::default());
        assert_eq!(module.name(), "Text-to-Speech");
        let commands = module.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "say");
    }

    #[tokio::test]
    async fn test_ignores_commands_it_does_not_claim() {
        let module = module_with(TtsConfig::default());
        let db = memory_db().await;
        let ctx = ctx_for("todo", "list");
        assert!(module.handle_command(&ctx, &db, None).await.is_none());
    }

    #[tokio::test]
    async fn test_refuses_without_panicking_or_loading_when_disabled() {
        let module = module_with(TtsConfig {
            enabled: false,
            ..TtsConfig::default()
        });
        let db = memory_db().await;
        let ctx = ctx_for("say", "hello there");

        let reply = module.handle_command(&ctx, &db, None).await.unwrap();
        assert!(reply.contains("未启用"));
    }

    #[tokio::test]
    async fn test_refuses_without_panicking_when_model_dir_is_absent() {
        let module = module_with(TtsConfig {
            enabled: true,
            model_dir: std::path::PathBuf::from("data/tts_models/definitely-not-here"),
            speaker_id: 0,
        });
        let db = memory_db().await;
        let ctx = ctx_for("say", "hello there");

        let reply = module.handle_command(&ctx, &db, None).await.unwrap();
        assert!(reply.contains("模型缺失"));
    }

    #[tokio::test]
    async fn test_rejects_empty_text_after_stripping_html_and_emoji() {
        let module = module_with(TtsConfig::default());
        let db = memory_db().await;
        // Disabled, so this never reaches the engine, but the empty-text
        // check on an enabled+present config is what production hits; that
        // path requires a real model directory and is covered by the
        // `#[ignore]`d test in `engine.rs`. Here we confirm the guard order:
        // a disabled config is refused before empty text is ever considered.
        let ctx = ctx_for("say", "✅🎉");
        let reply = module.handle_command(&ctx, &db, None).await.unwrap();
        assert!(reply.contains("未启用"));
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
    fn test_resolve_speaker_id_falls_back_to_env_when_unset() {
        assert_eq!(resolve_speaker_id(5, None), 5);
    }

    #[test]
    fn test_resolve_speaker_id_prefers_the_saved_preference() {
        assert_eq!(resolve_speaker_id(5, Some(77)), 77);
    }

    /// The bug this type change fixes: speaker 0 is a real, selectable voice.
    /// An explicit `Some(0)` must win over a non-zero environment fallback —
    /// the earlier `i64`-with-a-sentinel design collapsed `Some(0)` into
    /// "unset" and silently overrode it with `TTS_SPEAKER_ID` instead.
    #[test]
    fn test_resolve_speaker_id_honours_an_explicit_zero_over_a_nonzero_fallback() {
        assert_eq!(resolve_speaker_id(44, Some(0)), 0);
    }

    /// The companion case: with no saved preference at all, the non-zero
    /// fallback must still apply.
    #[test]
    fn test_resolve_speaker_id_falls_back_to_a_nonzero_env_default_when_unset() {
        assert_eq!(resolve_speaker_id(44, None), 44);
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
