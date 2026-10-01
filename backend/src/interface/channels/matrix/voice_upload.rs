//! Delivering a reply's audio attachment as a Matrix voice message (MSC3245).
//!
//! `TtsModule` hands back the Ogg Opus clip as a neutral [`Attachment`]; how a
//! platform shows it is the adapter's business. Matrix uploads it with
//! `AttachmentInfo::Voice`, which is what makes a client draw a voice bubble
//! rather than a file card.

use std::time::Duration;

use matrix_sdk::attachment::{AttachmentConfig, AttachmentInfo, BaseAudioInfo};
use matrix_sdk::room::Room;
use matrix_sdk::ruma::UInt;
use tracing::error;

use crate::infrastructure::inference::tts::WAVEFORM_MAX;
use crate::interface::channels::message::{Attachment, AttachmentKind, Reply};

/// What the sender is told when the clip was synthesized but not delivered.
/// Fixed text: the transport error is logged, never shown.
pub const SEND_FAILED_HTML: &str = "⚠️ <b>语音发送失败</b><br/>\
     • <b>原因:</b> 语音已生成，但发送到房间时出错<br/>\
     • <b>建议:</b> 请稍后重试。";

/// Sends every audio attachment of `reply` into `room`, before the reply text.
///
/// Returns the reply to send afterwards: `reply` itself, or the fixed failure
/// notice when an upload failed (the caption would claim a clip nobody got).
pub async fn deliver_audio(room: &Room, reply: Reply) -> Reply {
    for attachment in reply
        .content
        .attachments
        .iter()
        .filter(|a| a.kind == AttachmentKind::Audio)
    {
        if let Err(err) = send_voice_message(room, attachment).await {
            error!("Failed to send TTS voice message: {}", err);
            return Reply::html(SEND_FAILED_HTML);
        }
    }
    reply
}

async fn send_voice_message(room: &Room, attachment: &Attachment) -> Result<(), String> {
    let content_type = attachment
        .mime
        .parse::<mime_guess::mime::Mime>()
        .map_err(|e| format!("Invalid MIME type: {}", e))?;

    let info = AttachmentInfo::Voice(BaseAudioInfo {
        duration: attachment.duration_ms.map(Duration::from_millis),
        size: UInt::new(attachment.bytes.len() as u64),
        waveform: attachment.waveform.clone().map(normalise_waveform),
    });

    room.send_attachment(
        &attachment.filename,
        &content_type,
        attachment.bytes.clone(),
        AttachmentConfig::new().info(info),
    )
    .await
    .map_err(|e| format!("Matrix upload/send failed: {:?}", e))?;

    Ok(())
}

/// matrix-sdk takes the waveform as fractions of full scale and rescales it to
/// MSC3246's 0..=1024 buckets itself. Dividing by a power of two is exact in
/// `f32`, so every bucket reaches the event unchanged.
fn normalise_waveform(buckets: Vec<u16>) -> Vec<f32> {
    buckets
        .into_iter()
        .map(|b| f32::from(b) / f32::from(WAVEFORM_MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// matrix-sdk turns each fraction back into a bucket with
    /// `(v.clamp(0.0, 1.0) * 1024.0) as u16`; every bucket must survive that.
    #[test]
    fn test_normalised_waveform_round_trips_through_the_sdk_rescale() {
        let buckets: Vec<u16> = (0..=WAVEFORM_MAX).collect();
        let rescaled: Vec<u16> = normalise_waveform(buckets.clone())
            .into_iter()
            .map(|v| (v.clamp(0.0, 1.0) * f32::from(WAVEFORM_MAX)) as u16)
            .collect();
        assert_eq!(rescaled, buckets);
    }
}
