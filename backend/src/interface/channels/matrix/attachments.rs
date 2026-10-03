//! Handling an image or a voice note sent to the bot.
//!
//! The two were written out twice, identically apart from the module they call,
//! two stage labels and the wording of three error messages. Any change to the
//! flow — a new failure mode, a new stage, a third media kind — had to be made
//! in both places, and the branches had already drifted in how they built their
//! configuration.

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

use bytes::Bytes;
use crate::config::AiEnvConfig;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{ConversationAddress, PlatformIdentity};
use crate::domain::jobs::NewJob;
use crate::domain::storage::{StorageCategory, StorageKey};
use crate::infrastructure::postgres::repositories::{identities, jobs};
use crate::interface::channels::matrix::stage::{StageIndicator, StageReporter, TypingGuard};
use crate::interface::chat::voice::VoiceConfig;
use crate::llm::progress::ProgressSink;

/// Everything that differs between handling an image and handling a voice
/// note: which content carries the media, what to call the two processing
/// stages, the wording of three error messages, the two panic messages, and
/// which module function actually does the work.
enum Attachment<'a> {
    Image {
        content: &'a ImageMessageEventContent,
    },
    Audio {
        content: &'a AudioMessageEventContent,
        voice_config: &'a VoiceConfig,
        stt: &'a crate::infrastructure::inference::stt::SttClient,
    },
}

impl<'a> Attachment<'a> {
    fn from_msgtype(
        msgtype: &'a MessageType,
        _ocr: &'a crate::infrastructure::inference::ocr::OcrClient,
        voice_config: &'a VoiceConfig,
        stt: &'a crate::infrastructure::inference::stt::SttClient,
    ) -> Option<Self> {
        match msgtype {
            MessageType::Image(content) => Some(Self::Image { content }),
            MessageType::Audio(content) => Some(Self::Audio {
                content,
                voice_config,
                stt,
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
                "🧾 Saving receipt to storage...",
                "🧾 <i>正在保存小票至存储并排队处理...</i>",
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
                "⚠️ <b>图片处理异常</b><br/>• <b>原因:</b> 图片接收过程中发生系统异常<br/>• <b>建议:</b> 请尝试重新发送图片，或直接输入文字。"
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
            Self::Image { .. } => error!("Receipt image ingestion panicked: {:?}", panic_err),
            Self::Audio { .. } => error!("Voice audio processing panicked: {:?}", panic_err),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_module(
        &self,
        db: &Database,
        services: &crate::application::AppServices,
        ai: &AiEnvConfig,
        room: &Room,
        owner_id: &AccountId,
        media_bytes: &[u8],
        reporter: &StageReporter,
    ) -> String {
        let conversation = room.room_id().as_str();
        let progress: Option<&dyn ProgressSink> = Some(reporter);
        let reply = match self {
            Self::Image { content } => {
                let conversation =
                    ConversationAddress::Matrix(room.room_id().as_str().to_string());
                let mimetype = content
                    .info
                    .as_ref()
                    .and_then(|info| info.mimetype.as_deref());
                match ingest_receipt_image(
                    &db.pool,
                    services,
                    conversation,
                    owner_id,
                    &content.body,
                    mimetype,
                    media_bytes,
                )
                .await
                {
                    Ok(html) => return html,
                    Err(err) => return err,
                }
            }
            Self::Audio {
                content,
                voice_config,
                stt,
            } => {
                crate::interface::chat::voice::process_audio(
                    db,
                    services,
                    ai,
                    conversation,
                    owner_id,
                    media_bytes,
                    content
                        .info
                        .as_ref()
                        .and_then(|info| info.mimetype.as_deref()),
                    content.info.as_ref().and_then(|info| info.duration),
                    voice_config,
                    stt,
                    progress,
                )
                .await
            }
        };
        // The turn knows no channel; the tool targets are recorded against the
        // stage event, which is what a reaction on the reply resolves through.
        crate::interface::channels::matrix::reaction_targets::record_ai_tool_targets(
            &db.pool,
            reporter.message_ref().as_deref(),
            conversation,
            owner_id,
            &reply.tool_results,
        )
        .await;
        reply.html
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
    services: &crate::application::AppServices,
    ai_config: &AiEnvConfig,
    ocr: &crate::infrastructure::inference::ocr::OcrClient,
    voice_config: &VoiceConfig,
    stt: &crate::infrastructure::inference::stt::SttClient,
) {
    let Some(attachment) = Attachment::from_msgtype(&ev.content.msgtype, ocr, voice_config, stt)
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

    let resolved = match PlatformIdentity::matrix(ev.sender.as_str()) {
        Ok(identity) => identities::resolve_or_provision(&db.pool, &identity)
            .await
            .map_err(|err| format!("{err:?}")),
        Err(err) => Err(err.to_string()),
    };
    let owner_id = match resolved {
        Ok(owner_id) => owner_id,
        Err(err) => {
            error!("Could not resolve account for {}: {}", ev.sender, err);
            typing_guard.stop().await;
            let (plain, html) = attachment.account_error();
            stage_indicator.cancel_with_error(plain, html).await;
            return;
        }
    };

    let media_bytes = match crate::interface::channels::matrix::media::download_media_file(
        &room.client(),
        attachment.source(),
    )
    .await
    {
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
        services,
        ai_config,
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
    crate::interface::channels::matrix::reaction_targets::link_turn_targets(
        &db.pool,
        stage_indicator.event_id().map(|e| e.as_str()),
        sent_eid.as_deref().map(|e| e.as_str()),
        ev.event_id.as_str(),
    )
    .await;
}

fn image_meta(mimetype: Option<&str>, body: &str, bytes: &[u8]) -> (&'static str, &'static str) {
    let mime = mimetype.unwrap_or_default().to_ascii_lowercase();
    if mime.contains("jpeg") || mime.contains("jpg") {
        ("jpg", "image/jpeg")
    } else if mime.contains("png") {
        ("png", "image/png")
    } else if mime.contains("webp") {
        ("webp", "image/webp")
    } else if mime.contains("gif") {
        ("gif", "image/gif")
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        ("jpg", "image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("png", "image/png")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ("webp", "image/webp")
    } else {
        let lower = body.to_ascii_lowercase();
        if lower.ends_with(".png") {
            ("png", "image/png")
        } else if lower.ends_with(".webp") {
            ("webp", "image/webp")
        } else if lower.ends_with(".gif") {
            ("gif", "image/gif")
        } else {
            ("jpg", "image/jpeg")
        }
    }
}

pub(crate) async fn ingest_receipt_image(
    pool: &sqlx::PgPool,
    services: &crate::application::AppServices,
    conversation: ConversationAddress,
    owner_id: &AccountId,
    body: &str,
    mimetype: Option<&str>,
    media_bytes: &[u8],
) -> Result<String, String> {
    let (ext, content_type) = image_meta(mimetype, body, media_bytes);
    let storage_key = StorageKey::generate(owner_id, StorageCategory::Receipts, ext);

    if let Err(err) = services
        .storage
        .put(
            &storage_key,
            Bytes::copy_from_slice(media_bytes),
            content_type,
        )
        .await
    {
        error!(
            "Failed to save receipt image to storage for owner {}: {:?}",
            owner_id, err
        );
        return Err("⚠️ <b>图片保存失败</b><br/>• <b>原因:</b> 无法保存至存储服务<br/>• <b>建议:</b> 请稍后重试。".to_string());
    }

    let caption = {
        let b = body.trim();
        if b.is_empty() {
            None
        } else {
            Some(b.to_string())
        }
    };

    let job = NewJob::media_process_receipt(
        *owner_id,
        storage_key,
        Some(conversation),
        caption,
    );

    if let Err(err) = jobs::enqueue(pool, &job).await {
        error!(
            "Failed to enqueue media.process_receipt job for owner {}: {:?}",
            owner_id, err
        );
        return Err("⚠️ <b>任务排队失败</b><br/>• <b>原因:</b> 无法提交后台解析任务<br/>• <b>建议:</b> 请稍后重试。".to_string());
    }

    Ok("🧾 <b>已收到小票图片</b><br/>后台正在进行 OCR 识别与自动记账，处理完成后将在此通知您。".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::domain::jobs::JobType;
    use crate::infrastructure::storage::local::LocalStorageService;

    #[test]
    fn detects_image_metadata() {
        assert_eq!(image_meta(Some("image/png"), "", b""), ("png", "image/png"));
        assert_eq!(image_meta(Some("image/jpeg"), "", b""), ("jpg", "image/jpeg"));
        assert_eq!(image_meta(None, "receipt.webp", b""), ("webp", "image/webp"));
        assert_eq!(image_meta(None, "", b"\xFF\xD8\xFF\xE0"), ("jpg", "image/jpeg"));
        assert_eq!(image_meta(None, "", b"\x89PNG\r\n\x1a\n"), ("png", "image/png"));
        assert_eq!(image_meta(None, "unknown", b"data"), ("jpg", "image/jpeg"));
    }

    #[tokio::test]
    async fn ingest_receipt_image_stores_payload_and_enqueues_durable_job() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorageService::new(
            temp_dir.path(),
            None,
            "test-secret",
        )
        .unwrap();

        let services = crate::application::AppServices::new(&pool)
            .with_storage(Arc::new(storage.clone()));

        let owner = AccountId::from_seed("acct_matrix_ingest");
        crate::infrastructure::postgres::repositories::accounts::core::create_with_id(
            &pool,
            &owner,
            "Tester",
            crate::domain::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();

        let conv = ConversationAddress::Matrix("!room123:matrix.org".into());
        let fake_image = b"\x89PNG\r\n\x1a\nfake-receipt-image-content";

        let html = ingest_receipt_image(
            &pool,
            &services,
            conv.clone(),
            &owner,
            "lunch at cafe",
            Some("image/png"),
            fake_image,
        )
        .await
        .expect("ingestion should succeed");

        assert!(html.contains("已收到小票图片"));

        // Claim the enqueued job to verify its type and payload
        let claimed = jobs::claim(&pool, "test-worker", &[JobType::MediaProcessReceipt.as_str()])
            .await
            .expect("claim query should succeed")
            .expect("job should be claimable");

        assert_eq!(claimed.job_type, JobType::MediaProcessReceipt.as_str());
        assert_eq!(claimed.account_id, Some(owner));

        let storage_key_str = claimed.payload["storage_key"].as_str().expect("storage_key in payload");
        let storage_key: StorageKey = storage_key_str.parse().expect("valid storage key");

        assert_eq!(storage_key.account_id(), owner);
        assert_eq!(storage_key.category(), StorageCategory::Receipts);

        // Verify image bytes can be retrieved from object storage
        let retrieved = services.storage.get(&storage_key).await.expect("storage get");
        assert_eq!(retrieved.as_ref(), fake_image);
    }
}
