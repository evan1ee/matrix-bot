use std::fmt;

/// Voice transcription settings the bot owns. Where the model runs, and which
/// provider serves it, is inference's concern.
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub language: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            language: "auto".to_string(),
        }
    }
}

impl VoiceConfig {
    /// The language to ask the transcription service for, or `None` to let it
    /// detect. `auto` is SenseVoice's own word for detection, and
    /// OpenAI-compatible providers reject it, so it is never sent.
    pub fn language_hint(&self) -> Option<&str> {
        let language = self.language.trim();
        if language.is_empty() || language.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(language)
        }
    }
}

#[derive(Debug, Clone)]
pub struct VoiceTranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub duration_secs: f32,
}

impl VoiceTranscriptionResult {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

impl From<crate::infrastructure::inference::stt::SttTranscription> for VoiceTranscriptionResult {
    fn from(t: crate::infrastructure::inference::stt::SttTranscription) -> Self {
        Self {
            text: t.text,
            language: t.language,
            duration_secs: t.duration.unwrap_or(0.0),
        }
    }
}

impl fmt::Display for VoiceTranscriptionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_hint_never_sends_auto_or_blank() {
        let hint = |language: &str| {
            VoiceConfig {
                enabled: true,
                language: language.to_string(),
            }
            .language_hint()
            .map(str::to_owned)
        };
        assert_eq!(hint("auto"), None);
        assert_eq!(hint("AUTO"), None);
        assert_eq!(hint("  "), None);
        assert_eq!(hint(""), None);
        assert_eq!(hint("zh"), Some("zh".to_string()));
        assert_eq!(hint(" en "), Some("en".to_string()));
    }

    #[test]
    fn test_voice_config_defaults_to_enabled_with_auto_detection() {
        let config = VoiceConfig::default();
        assert!(config.enabled);
        assert_eq!(config.language, "auto");
    }

    #[test]
    fn test_a_transcription_without_a_duration_maps_to_zero() {
        let result: VoiceTranscriptionResult =
            crate::infrastructure::inference::stt::SttTranscription {
                text: "hi".to_string(),
                language: None,
                duration: None,
            }
            .into();
        assert_eq!(result.text, "hi");
        assert_eq!(result.duration_secs, 0.0);
    }
}
