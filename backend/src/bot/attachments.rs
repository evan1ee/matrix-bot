//! Handling an image or a voice note sent to the bot.
//!
//! The two were written out twice, identically apart from the module they call,
//! two stage labels and the wording of three error messages. Any change to the
//! flow — a new failure mode, a new stage, a third media kind — had to be made
//! in both places, and the branches had already drifted in how they built their
//! configuration.

use std::path::Path;

use futures_util::FutureExt;
use matrix_sdk::{
    room::Room,
    ruma::events::room::message::{
        AudioMessageEventContent, ImageMessageEventContent, MessageType,
        OriginalSyncRoomMessageEvent,
    },
    ruma::events::room::MediaSource,
    ruma::UserId,
};
use tracing::{error, warn};

use crate::bot::stage::{StageIndicator, StageReporter, TypingGuard};
use crate::config::AiEnvConfig;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::voice::VoiceConfig;

/// Everything that differs between handling an image and handling a voice
/// note: which content carries the media, what to call the two processing
/// stages, the wording of three error messages, the two panic messages, and
/// which module function actually does the work.
enum Attachment<'a> {
    Image {
        content: &'a ImageMessageEventContent,
        model_dir: &'a Path,
    },
    Audio {
        content: &'a AudioMessageEventContent,
        voice_config: &'a VoiceConfig,
    },
}

impl<'a> Attachment<'a> {
    fn from_msgtype(
        msgtype: &'a MessageType,
        ocr_model_dir: &'a Path,
        voice_config: &'a VoiceConfig,
    ) -> Option<Self> {
        match msgtype {
            MessageType::Image(content) => Some(Self::Image {
                content,
                model_dir: ocr_model_dir,
            }),
            MessageType::Audio(content) => Some(Self::Audio {
                content,
                voice_config,
            }),
            _ => None,
        }
    }

    fn body(&self) -> &str {
        match self {
            Self::Image { content, .. } => &content.body,
            Self::Audio { content, .. } => &content.body,
        }
    }

    fn source(&self) -> &MediaSource {
        match self {
            Self::Image { content, .. } => &content.source,
            Self::Audio { content, .. } => &content.source,
        }
    }

    fn stage_labels(&self) -> (&'static str, &'static str) {
        match self {
            Self::Image { .. } => (
                "🔍 Scanning image with OCR...",
                "🔍 <i>正在识别图片文字 (OCR)...</i>",
            ),
            Self::Audio { .. } => ("🎧 Transcribing voice...", "🎧 <i>正在转写语音...</i>"),
        }
    }

    fn account_error(&self) -> (&'static str, &'static str) {
        (
            "⚠️ 账号验证失败：无法解析 Matrix 用户身份。请稍后重试或重新加入房间。",
            "⚠️ <b>账号验证失败</b><br/>• <b>原因:</b> 无法解析 Matrix 用户身份<br/>• <b>建议:</b> 请稍后重试或重新加入房间。",
        )
    }

    fn empty_error(&self) -> (&'static str, &'static str) {
        match self {
            Self::Image { .. } => (
                "⚠️ 图片获取失败：图片附件数据为空。请重新上传发送。",
                "⚠️ <b>图片处理失败</b><br/>• <b>原因:</b> 图片附件数据为空<br/>• <b>建议:</b> 请重新上传发送，或直接输入文字。",
            ),
            Self::Audio { .. } => (
                "⚠️ 语音获取失败：音频附件数据为空。请重新录制发送。",
                "⚠️ <b>语音处理失败</b><br/>• <b>原因:</b> 音频附件数据为空<br/>• <b>建议:</b> 请重新录制并发送语音，或直接输入文字。",
            ),
        }
    }

    fn download_error(&self) -> (&'static str, &'static str) {
        match self {
            Self::Image { .. } => (
                "⚠️ 图片下载失败：无法下载或解密图片附件。请检查网络后重新发送。",
                "⚠️ <b>图片下载失败</b><br/>• <b>原因:</b> 无法下载或解密图片附件<br/>• <b>建议:</b> 请检查网络连接后重新发送，或直接输入文字。",
            ),
            Self::Audio { .. } => (
                "⚠️ 语音下载失败：无法下载或解密音频附件。请检查网络后重新发送。",
                "⚠️ <b>语音下载失败</b><br/>• <b>原因:</b> 无法下载或解密音频附件<br/>• <b>建议:</b> 请检查网络连接后重新发送，或直接输入文字。",
            ),
        }
    }

    fn panic_message(&self) -> &'static str {
        match self {
            Self::Image { .. } => {
                "⚠️ <b>图片处理异常</b><br/>• <b>原因:</b> 图片识别过程中发生系统异常<br/>• <b>建议:</b> 请尝试重新拍摄发送，或直接输入文字。"
            }
            Self::Audio { .. } => {
                "⚠️ <b>语音处理异常</b><br/>• <b>原因:</b> 音频解码或识别过程中发生系统异常<br/>• <b>建议:</b> 请尝试重新发送语音，或直接输入文字。"
            }
        }
    }

    fn warn_empty(&self, room_id: &str) {
        match self {
            Self::Image { .. } => {
                warn!("Media attachment returned empty body in room {}", room_id);
            }
            Self::Audio { .. } => {
                warn!(
                    "Audio media attachment returned empty body in room {}",
                    room_id
                );
            }
        }
    }

    fn warn_download_failed(&self, room_id: &str, err: &anyhow::Error) {
        match self {
            Self::Image { .. } => {
                warn!("Failed to fetch media file in room {}: {:?}", room_id, err);
            }
            Self::Audio { .. } => {
                warn!("Failed to fetch audio file in room {}: {:?}", room_id, err);
            }
        }
    }

    fn log_panic(&self, panic_err: &(dyn std::any::Any + Send)) {
        match self {
            Self::Image { .. } => error!("OCR image processing panicked: {:?}", panic_err),
            Self::Audio { .. } => error!("Voice audio processing panicked: {:?}", panic_err),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_module(
        &self,
        db: &Database,
        ai: &AiEnvConfig,
        broadcaster: Option<&EventBroadcaster>,
        room: &Room,
        owner_id: &str,
        media_bytes: &[u8],
        reporter: &StageReporter,
    ) -> String {
        match self {
            Self::Image { content, model_dir } => {
                crate::modules::ocr::process_image_event(
                    db,
                    ai,
                    broadcaster,
                    room,
                    owner_id,
                    Some(&content.body),
                    media_bytes,
                    model_dir,
                    Some(reporter),
                )
                .await
            }
            Self::Audio { voice_config, .. } => {
                crate::modules::voice::process_audio_event(
                    db,
                    ai,
                    broadcaster,
                    room,
                    owner_id,
                    media_bytes,
                    voice_config,
                    Some(reporter),
                )
                .await
            }
        }
    }
}

/// Handles an incoming image or audio attachment: OCR / transcription, an AI
/// turn over the result, and a reply posted in-place over a stage indicator.
///
/// A no-op for any other message type.
#[allow(clippy::too_many_arguments)]
pub async fn handle_attachment(
    ev: &OriginalSyncRoomMessageEvent,
    room: &Room,
    own_id: &UserId,
    is_direct: bool,
    db: &Database,
    ai_config: &AiEnvConfig,
    broadcaster: Option<&EventBroadcaster>,
    ocr_model_dir: &Path,
    voice_config: &VoiceConfig,
) {
    let Some(attachment) =
        Attachment::from_msgtype(&ev.content.msgtype, ocr_model_dir, voice_config)
    else {
        return;
    };

    let is_mentioned = attachment.body().contains(own_id.as_str())
        || attachment.body().contains(own_id.localpart());

    if !is_direct && !is_mentioned {
        return;
    }

    let mut typing_guard = TypingGuard::start(room.clone());
    let (plain_stage, html_stage) = attachment.stage_labels();
    let stage_indicator = StageIndicator::start(room, plain_stage, html_stage).await;

    let owner_id = match crate::accounts::AccountRepository::resolve_or_provision(
        &db.pool,
        ev.sender.as_str(),
    )
    .await
    {
        Ok(owner_id) => owner_id,
        Err(err) => {
            error!("Could not resolve account for {}: {:?}", ev.sender, err);
            typing_guard.stop().await;
            let (plain, html) = attachment.account_error();
            stage_indicator.cancel_with_error(plain, html).await;
            return;
        }
    };

    let media_bytes =
        match crate::bot::media::download_media_file(&room.client(), attachment.source()).await {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => {
                attachment.warn_empty(room.room_id().as_str());
                typing_guard.stop().await;
                let (plain, html) = attachment.empty_error();
                stage_indicator.cancel_with_error(plain, html).await;
                return;
            }
            Err(err) => {
                attachment.warn_download_failed(room.room_id().as_str(), &err);
                typing_guard.stop().await;
                let (plain, html) = attachment.download_error();
                stage_indicator.cancel_with_error(plain, html).await;
                return;
            }
        };

    let reporter = stage_indicator.reporter();
    let result = std::panic::AssertUnwindSafe(attachment.run_module(
        db,
        ai_config,
        broadcaster,
        room,
        &owner_id,
        &media_bytes,
        &reporter,
    ))
    .catch_unwind()
    .await;

    typing_guard.stop().await;

    let reply_html = match result {
        Ok(html) => html,
        Err(panic_err) => {
            attachment.log_panic(&*panic_err);
            attachment.panic_message().to_string()
        }
    };

    let plain_text = crate::utils::strip_html(&reply_html);
    let sent_eid = stage_indicator
        .finish_with_reply(&plain_text, &reply_html)
        .await;
    crate::bot::reaction_targets::link_turn_targets(
        &db.pool,
        stage_indicator.event_id().map(|e| e.as_str()),
        sent_eid.as_deref().map(|e| e.as_str()),
        ev.event_id.as_str(),
    )
    .await;
}
