//! Voice messages and audio files sent to the bot in a direct message.
//!
//! Detection is pure; [`run`] downloads within bounds and hands the bytes to
//! the shared flow in `channels::audio_ingress`.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{Attachment, ChannelId, Http, MessageId, User};
use tracing::warn;

use super::inbound::identity;
use super::progress::TypingProgress;
use super::worker::reply_dm;
use crate::domain::channels::audio::{check_declared_duration, check_declared_size, AudioPayload};
use crate::domain::channels::ConversationAddress;
use crate::interface::channels::audio_ingress::ingest_audio;
use crate::interface::channels::inbound::Inbound;
use crate::interface::channels::message::Reply;
use crate::llm::progress::ProgressSink;

/// Bounds the download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);

const DOWNLOAD_FAILED: &str = "⚠️ Could not download the voice message. Please try again.";
const DOWNLOAD_TIMED_OUT: &str = "⚠️ Downloading the voice message timed out. Please try again.";
const NO_IDENTITY: &str = "⚠️ Could not verify your account. Please try again later.";

const AUDIO_EXTENSIONS: [&str; 5] = [".ogg", ".opus", ".mp3", ".wav", ".m4a"];

/// True when an attachment looks like audio by its MIME type or extension.
pub fn is_audio_attachment(attachment: &Attachment) -> bool {
    if attachment
        .content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("audio/"))
    {
        return true;
    }
    let name = attachment.filename.to_ascii_lowercase();
    AUDIO_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
}

/// Transcribes one audio attachment and answers it in the same DM. Every
/// failure is a reply; nothing here can end the worker.
pub(super) async fn run(
    inbound: &Inbound,
    http: &Arc<Http>,
    channel: ChannelId,
    message: MessageId,
    author: &User,
    attachment: &Attachment,
    caption: Option<&str>,
) {
    let reply = reply_for(inbound, http, channel, author, attachment, caption).await;
    reply_dm(http, channel, message, reply).await;
}

async fn reply_for(
    inbound: &Inbound,
    http: &Arc<Http>,
    channel: ChannelId,
    author: &User,
    attachment: &Attachment,
    caption: Option<&str>,
) -> Reply {
    // A voice message reports its length; a plain file may not.
    let duration_secs = attachment.duration_secs.map(|secs| secs.ceil() as u32);
    let declared = check_declared_size(attachment.size.into())
        .and_then(|()| duration_secs.map_or(Ok(()), check_declared_duration));
    if let Err(err) = declared {
        return Reply::html(err.reply_text());
    }
    let Some(sender) = identity(author, None) else {
        warn!(user_id = %author.id, "could not construct Discord platform identity");
        return Reply::html(NO_IDENTITY);
    };
    let bytes = match tokio::time::timeout(DOWNLOAD_TIMEOUT, attachment.download()).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(err)) => {
            warn!(error = %err, "could not download a Discord voice attachment");
            return Reply::html(DOWNLOAD_FAILED);
        }
        Err(_) => {
            warn!("timed out downloading a Discord voice attachment");
            return Reply::html(DOWNLOAD_TIMED_OUT);
        }
    };
    let payload = match AudioPayload::new(
        bytes,
        attachment.content_type.as_deref(),
        Some(&attachment.filename),
        duration_secs,
    ) {
        Ok(payload) => payload,
        Err(err) => return Reply::html(err.reply_text()),
    };
    let progress = TypingProgress::new(Arc::clone(http), channel);
    ingest_audio(
        inbound,
        &sender,
        &ConversationAddress::Discord(channel.get()),
        payload,
        caption,
        Some(&progress as &dyn ProgressSink),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::discord::inbound::is_image_attachment;

    fn attachment(filename: &str, content_type: Option<&str>) -> Attachment {
        serde_json::from_value(serde_json::json!({
            "id": "1100000000000000021",
            "filename": filename,
            "size": 2048,
            "url": "https://cdn.discordapp.com/attachments/1/2/voice-message.ogg",
            "proxy_url": "https://media.discordapp.net/attachments/1/2/voice-message.ogg",
            "content_type": content_type,
            "duration_secs": 3.2,
        }))
        .unwrap()
    }

    #[test]
    fn detects_audio_by_mime_and_extension() {
        assert!(is_audio_attachment(&attachment(
            "voice-message.ogg",
            Some("audio/ogg")
        )));
        assert!(is_audio_attachment(&attachment(
            "clip.bin",
            Some("audio/mpeg")
        )));
        assert!(is_audio_attachment(&attachment("Memo.M4A", None)));
        assert!(is_audio_attachment(&attachment(
            "take.wav",
            Some("application/octet-stream")
        )));
    }

    #[test]
    fn images_pdfs_and_text_are_not_audio() {
        for (name, mime) in [
            ("receipt.png", Some("image/png")),
            ("invoice.pdf", Some("application/pdf")),
            ("notes.txt", None),
            ("movie.mp4", Some("video/mp4")),
        ] {
            assert!(!is_audio_attachment(&attachment(name, mime)), "{name}");
        }
    }

    #[test]
    fn an_audio_attachment_is_never_an_image_so_it_cannot_become_a_receipt() {
        let voice = attachment("voice-message.ogg", Some("audio/ogg"));
        assert!(!is_image_attachment(&voice));
    }

    #[test]
    fn the_reported_duration_is_read() {
        assert_eq!(attachment("v.ogg", None).duration_secs, Some(3.2));
    }
}
