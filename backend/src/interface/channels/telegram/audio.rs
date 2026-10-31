//! Voice notes and audio files sent to the bot in a private chat.
//!
//! [`audio_of`] is pure (no network, no services), so it is tested against
//! fixtures alone; [`run`] downloads within bounds and hands the bytes to the
//! shared flow in `channels::audio_ingress`.

use std::time::Duration;

use teloxide_core::net::Download;
use teloxide_core::prelude::*;
use teloxide_core::types::{Update, UpdateKind as TgUpdateKind};
use tracing::warn;

use super::inbound::{identity, UpdateKind};
use super::progress::TelegramProgress;
use super::render::Plan;
use super::worker::{plan_reply, post, WorkerCtx};
use crate::domain::channels::audio::{check_declared_duration, check_declared_size, AudioPayload};
use crate::domain::channels::{ConversationAddress, PlatformIdentity};
use crate::interface::channels::audio_ingress::ingest_audio;
use crate::interface::channels::inbound::InboundOutcome;
use crate::interface::channels::message::Reply;
use crate::llm::progress::ProgressSink;

/// Bounds the `getFile` call and the download together.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

/// A voice note's MIME type when Telegram sends none: voice notes are Opus in Ogg.
const VOICE_DEFAULT_MIME: &str = "audio/ogg";

const DOWNLOAD_FAILED: &str = "⚠️ Could not download the voice message. Please try again.";
const DOWNLOAD_TIMED_OUT: &str = "⚠️ Downloading the voice message timed out. Please try again.";

#[derive(Debug, PartialEq, Eq)]
pub struct TelegramAudio {
    pub chat_id: i64,
    pub message_id: i32,
    pub sender: PlatformIdentity,
    pub file_id: String,
    /// `None` when Telegram did not report a size.
    pub file_size: Option<u32>,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub duration_secs: u32,
    pub caption: Option<String>,
}

/// The voice note or audio file in `update`, or `None`: not a message, a bot
/// author, or not a private chat (a voice note cannot mention the bot, so a
/// group never addresses it).
pub fn audio_of(update: &Update) -> Option<TelegramAudio> {
    let TgUpdateKind::Message(message) = &update.kind else {
        return None;
    };
    let from = message.from.as_ref()?;
    if from.is_bot || !message.chat.is_private() {
        return None;
    }
    let (file, duration, mime, filename) = match (message.voice(), message.audio()) {
        (Some(voice), _) => (
            &voice.file,
            voice.duration,
            Some(
                voice
                    .mime_type
                    .as_ref()
                    .map_or_else(|| VOICE_DEFAULT_MIME.to_owned(), ToString::to_string),
            ),
            None,
        ),
        (None, Some(audio)) => (
            &audio.file,
            audio.duration,
            audio.mime_type.as_ref().map(ToString::to_string),
            audio.file_name.clone(),
        ),
        (None, None) => return None,
    };
    Some(TelegramAudio {
        chat_id: message.chat.id.0,
        message_id: message.id.0,
        sender: identity(from)?,
        file_id: file.id.0.clone(),
        file_size: known_size(file.size),
        mime,
        filename,
        duration_secs: duration.seconds(),
        caption: message
            .caption()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned),
    })
}

/// Telegram's `FileMeta` uses `u32::MAX` for "not reported".
fn known_size(size: u32) -> Option<u32> {
    (size != u32::MAX).then_some(size)
}

/// Transcribes one voice note and answers it where it came from. Every failure
/// is a reply; nothing here can end the worker.
pub(super) async fn run(ctx: &WorkerCtx, audio: TelegramAudio) {
    let kind = UpdateKind::Message {
        chat_id: audio.chat_id,
        message_id: audio.message_id,
    };
    let progress = TelegramProgress::new(ctx.bot.clone(), ChatId(audio.chat_id));
    let reply = reply_for(ctx, &audio, &progress).await;
    match plan_reply(InboundOutcome::Replied(reply), false) {
        Ok(Some(plan)) => {
            post(&ctx.bot, &kind, plan).await;
        }
        Ok(None) => {}
        Err(err) => {
            warn!(error = %err, "A Telegram voice reply could not be planned");
            post(
                &ctx.bot,
                &kind,
                Plan::text("That reply could not be sent here."),
            )
            .await;
        }
    }
    progress.finish().await;
}

async fn reply_for(ctx: &WorkerCtx, audio: &TelegramAudio, progress: &TelegramProgress) -> Reply {
    let declared = audio
        .file_size
        .map_or(Ok(()), |size| check_declared_size(size.into()))
        .and_then(|()| check_declared_duration(audio.duration_secs));
    if let Err(err) = declared {
        return Reply::html(err.reply_text());
    }
    let bytes = match download(&ctx.bot, &audio.file_id).await {
        Ok(bytes) => bytes,
        Err(notice) => return Reply::html(notice),
    };
    let payload = match AudioPayload::new(
        bytes,
        audio.mime.as_deref(),
        audio.filename.as_deref(),
        Some(audio.duration_secs),
    ) {
        Ok(payload) => payload,
        Err(err) => return Reply::html(err.reply_text()),
    };
    ingest_audio(
        &ctx.inbound,
        &audio.sender,
        &ConversationAddress::Telegram(audio.chat_id),
        payload,
        audio.caption.as_deref(),
        Some(progress as &dyn ProgressSink),
    )
    .await
}

async fn download(bot: &Bot, file_id: &str) -> Result<Vec<u8>, &'static str> {
    let fetch = async {
        let file = bot
            .get_file(file_id.to_owned().into())
            .send()
            .await
            .map_err(|err| warn!(error = %err, "Telegram getFile failed for a voice message"))?;
        if let Some(size) = known_size(file.size) {
            if check_declared_size(size.into()).is_err() {
                return Ok(Vec::new());
            }
        }
        let mut bytes = Vec::new();
        bot.download_file(&file.path, &mut bytes).await.map_err(
            |err| warn!(error = %err, "Telegram file download failed for a voice message"),
        )?;
        Ok(bytes)
    };
    match tokio::time::timeout(DOWNLOAD_TIMEOUT, fetch).await {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(())) => Err(DOWNLOAD_FAILED),
        Err(_) => Err(DOWNLOAD_TIMED_OUT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const VOICE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_private_voice.json"
    ));

    fn update(edit: impl FnOnce(&mut Value)) -> Update {
        let mut v: Value = serde_json::from_str(VOICE).unwrap();
        edit(&mut v);
        // Never `from_value`: it silently yields `UpdateKind::Error`.
        serde_json::from_str(&v.to_string()).unwrap()
    }

    #[test]
    fn a_private_voice_note_is_extracted() {
        let a = audio_of(&update(|_| {})).expect("audio");
        assert_eq!(a.chat_id, 4100000003);
        assert_eq!(a.message_id, 102);
        assert_eq!(a.file_id, "AwACAgIAAxkBAAIBZ2voice");
        assert_eq!(a.file_size, Some(10240));
        assert_eq!(a.mime.as_deref(), Some("audio/ogg"));
        assert_eq!(a.duration_secs, 3);
        assert_eq!(a.caption, None);
        assert_eq!(a.sender.user_id().as_str(), "4100000003");
    }

    #[test]
    fn a_voice_note_with_a_null_mime_type_is_ogg() {
        let a = audio_of(&update(|v| {
            v["message"]["voice"]["mime_type"] = Value::Null;
        }))
        .expect("audio");
        assert_eq!(a.mime.as_deref(), Some("audio/ogg"));
    }

    #[test]
    fn a_missing_size_is_unknown_not_huge() {
        let a = audio_of(&update(|v| {
            v["message"]["voice"]
                .as_object_mut()
                .unwrap()
                .remove("file_size");
        }))
        .expect("audio");
        assert_eq!(a.file_size, None);
    }

    #[test]
    fn an_audio_file_carries_its_name_and_caption() {
        let a = audio_of(&update(|v| {
            let m = v["message"].as_object_mut().unwrap();
            let mut voice = m.remove("voice").unwrap();
            voice["file_name"] = "memo.m4a".into();
            voice["mime_type"] = "audio/mp4".into();
            m.insert("audio".into(), voice);
            m.insert("caption".into(), "  /note groceries ".into());
        }))
        .expect("audio");
        assert_eq!(a.filename.as_deref(), Some("memo.m4a"));
        assert_eq!(a.mime.as_deref(), Some("audio/mp4"));
        assert_eq!(a.caption.as_deref(), Some("/note groceries"));
    }

    #[test]
    fn a_group_voice_note_is_ignored() {
        let none = audio_of(&update(|v| {
            v["message"]["chat"]["id"] = (-100123).into();
            v["message"]["chat"]["type"] = "supergroup".into();
        }));
        assert!(none.is_none());
    }

    #[test]
    fn a_bot_sender_is_ignored() {
        assert!(audio_of(&update(|v| v["message"]["from"]["is_bot"] = true.into())).is_none());
    }

    #[test]
    fn a_text_message_yields_none() {
        let text: Update = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../contracts/channels/telegram_private_text.json"
        )))
        .unwrap();
        assert!(audio_of(&text).is_none());
    }
}
