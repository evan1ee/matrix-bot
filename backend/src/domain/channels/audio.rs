//! A voice note or audio clip a channel hands to the transcriber. Pure: the
//! adapter downloads the bytes, this decides whether they are worth sending.

use thiserror::Error;

pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_AUDIO_SECS: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMime {
    Ogg,
    Opus,
    Mpeg,
    Wav,
    M4a,
}

impl AudioMime {
    /// The canonical MIME type, sent on to the speech-to-text service.
    pub fn as_mime(self) -> &'static str {
        match self {
            Self::Ogg => "audio/ogg",
            Self::Opus => "audio/opus",
            Self::Mpeg => "audio/mpeg",
            Self::Wav => "audio/wav",
            Self::M4a => "audio/m4a",
        }
    }

    fn from_mime(mime: &str) -> Option<Self> {
        match mime {
            "audio/ogg" => Some(Self::Ogg),
            "audio/opus" => Some(Self::Opus),
            "audio/mpeg" | "audio/mp3" => Some(Self::Mpeg),
            "audio/wav" | "audio/x-wav" | "audio/wave" => Some(Self::Wav),
            "audio/m4a" | "audio/x-m4a" | "audio/mp4" => Some(Self::M4a),
            _ => None,
        }
    }

    fn from_filename(name: &str) -> Option<Self> {
        let (_, ext) = name.rsplit_once('.')?;
        match ext.to_ascii_lowercase().as_str() {
            "ogg" | "oga" => Some(Self::Ogg),
            "opus" => Some(Self::Opus),
            "mp3" => Some(Self::Mpeg),
            "wav" => Some(Self::Wav),
            "m4a" => Some(Self::M4a),
            _ => None,
        }
    }

    /// A declared MIME wins. The filename decides only when the platform sent
    /// none, or sent the generic `application/octet-stream`.
    fn classify(mime: Option<&str>, filename: Option<&str>) -> Option<Self> {
        let declared = mime
            .map(|m| {
                m.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .filter(|m| !m.is_empty() && m != "application/octet-stream");
        match declared {
            Some(m) => Self::from_mime(&m),
            None => filename.and_then(Self::from_filename),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AudioIngressError {
    #[error("the audio is empty")]
    Empty,
    #[error("the audio exceeds the size limit")]
    PayloadTooLarge,
    #[error("the audio exceeds the duration limit")]
    TooLong,
    #[error("the audio format is not supported")]
    UnsupportedFormat,
}

impl AudioIngressError {
    /// What the sender is told; never carries platform detail.
    pub fn reply_text(&self) -> &'static str {
        match self {
            Self::Empty => "⚠️ The voice message was empty. Please record it again.",
            Self::PayloadTooLarge => "⚠️ That audio is too large (maximum size is 10 MiB).",
            Self::TooLong => "⚠️ That audio is too long (maximum length is 2 minutes).",
            Self::UnsupportedFormat => {
                "⚠️ Unsupported audio format. Send ogg, opus, mp3, wav or m4a."
            }
        }
    }
}

/// The size a platform reports before the download, so an oversized file is
/// refused without fetching it.
pub fn check_declared_size(bytes: u64) -> Result<(), AudioIngressError> {
    if bytes > MAX_AUDIO_BYTES as u64 {
        Err(AudioIngressError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

/// The length a platform reports before the download, refused for the same
/// reason as [`check_declared_size`].
pub fn check_declared_duration(secs: u32) -> Result<(), AudioIngressError> {
    if secs > MAX_AUDIO_SECS {
        Err(AudioIngressError::TooLong)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AudioPayload {
    bytes: Vec<u8>,
    mime: AudioMime,
    duration_secs: Option<u32>,
}

impl AudioPayload {
    pub fn new(
        bytes: Vec<u8>,
        mime: Option<&str>,
        filename: Option<&str>,
        duration_secs: Option<u32>,
    ) -> Result<Self, AudioIngressError> {
        if bytes.is_empty() {
            return Err(AudioIngressError::Empty);
        }
        if bytes.len() > MAX_AUDIO_BYTES {
            return Err(AudioIngressError::PayloadTooLarge);
        }
        if duration_secs.is_some_and(|d| d > MAX_AUDIO_SECS) {
            return Err(AudioIngressError::TooLong);
        }
        let mime =
            AudioMime::classify(mime, filename).ok_or(AudioIngressError::UnsupportedFormat)?;
        Ok(Self {
            bytes,
            mime,
            duration_secs,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn mime(&self) -> AudioMime {
        self.mime
    }

    pub fn duration_secs(&self) -> Option<u32> {
        self.duration_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(mime: Option<&str>, name: Option<&str>) -> Result<AudioPayload, AudioIngressError> {
        AudioPayload::new(vec![1, 2, 3], mime, name, None)
    }

    #[test]
    fn accepts_ogg_with_codec_parameter() {
        let p = payload(Some("audio/ogg; codecs=opus"), None).unwrap();
        assert_eq!(p.mime(), AudioMime::Ogg);
    }

    #[test]
    fn maps_aliases() {
        for (mime, want) in [
            ("audio/x-m4a", AudioMime::M4a),
            ("audio/mp4", AudioMime::M4a),
            ("audio/x-wav", AudioMime::Wav),
            ("audio/mp3", AudioMime::Mpeg),
            ("AUDIO/MPEG", AudioMime::Mpeg),
        ] {
            assert_eq!(payload(Some(mime), None).unwrap().mime(), want, "{mime}");
        }
    }

    #[test]
    fn falls_back_to_the_filename_extension_when_mime_is_missing_or_generic() {
        let want = AudioMime::Ogg;
        assert_eq!(payload(None, Some("voice.OGG")).unwrap().mime(), want);
        let generic = payload(Some("application/octet-stream"), Some("voice.ogg"));
        assert_eq!(generic.unwrap().mime(), want);
    }

    #[test]
    fn a_declared_mime_is_not_overridden_by_the_filename() {
        assert_eq!(
            payload(Some("audio/flac"), Some("voice.ogg")).unwrap_err(),
            AudioIngressError::UnsupportedFormat
        );
    }

    #[test]
    fn rejects_flac_and_unknown_extensions() {
        for (mime, name) in [
            (Some("audio/flac"), None),
            (None, Some("notes.txt")),
            (None, Some("noextension")),
            (None, None),
            (Some("video/mp4"), None),
        ] {
            assert_eq!(
                payload(mime, name).unwrap_err(),
                AudioIngressError::UnsupportedFormat,
                "{mime:?} {name:?}"
            );
        }
    }

    #[test]
    fn rejects_empty() {
        let err = AudioPayload::new(vec![], Some("audio/ogg"), None, None).unwrap_err();
        assert_eq!(err, AudioIngressError::Empty);
    }

    #[test]
    fn rejects_one_byte_over_ten_mib_and_accepts_exactly_ten() {
        let ok = AudioPayload::new(vec![0; MAX_AUDIO_BYTES], Some("audio/ogg"), None, None);
        assert!(ok.is_ok());
        let over = AudioPayload::new(vec![0; MAX_AUDIO_BYTES + 1], Some("audio/ogg"), None, None);
        assert_eq!(over.unwrap_err(), AudioIngressError::PayloadTooLarge);
    }

    #[test]
    fn rejects_121_seconds_and_accepts_120() {
        let at = |secs| AudioPayload::new(vec![1], Some("audio/ogg"), None, Some(secs));
        assert!(at(120).is_ok());
        assert_eq!(at(121).unwrap_err(), AudioIngressError::TooLong);
    }

    #[test]
    fn check_declared_duration_matches_the_limit() {
        assert!(check_declared_duration(MAX_AUDIO_SECS).is_ok());
        assert_eq!(
            check_declared_duration(MAX_AUDIO_SECS + 1).unwrap_err(),
            AudioIngressError::TooLong
        );
    }

    #[test]
    fn check_declared_size_matches_the_limit() {
        assert!(check_declared_size(MAX_AUDIO_BYTES as u64).is_ok());
        assert_eq!(
            check_declared_size(MAX_AUDIO_BYTES as u64 + 1).unwrap_err(),
            AudioIngressError::PayloadTooLarge
        );
    }
}
