//! Morning-briefing invariants that do not depend on how the briefing is built.

/// Longest spoken script, in characters: bounds synthesis latency.
pub const MAX_AUDIO_SCRIPT_CHARS: usize = 2_000;

/// Days a briefing recording is kept.
pub const AUDIO_RETENTION_DAYS: i64 = 30;

/// Plain text written to be read aloud, never empty and never longer than
/// [`MAX_AUDIO_SCRIPT_CHARS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingAudioScript(String);

impl BriefingAudioScript {
    /// Trims `text`; `None` when nothing is left. An over-long text is cut at
    /// the last sentence end that fits, or at the limit when there is none.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if text.chars().count() <= MAX_AUDIO_SCRIPT_CHARS {
            return Some(Self(text.to_string()));
        }
        let cut: String = text.chars().take(MAX_AUDIO_SCRIPT_CHARS).collect();
        let end = cut
            .char_indices()
            .filter(|(_, c)| matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '\n'))
            .map(|(i, c)| i + c.len_utf8())
            .next_back()
            .unwrap_or(cut.len());
        Some(Self(cut[..end].trim_end().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_text_has_no_script() {
        assert_eq!(BriefingAudioScript::new(" \n\t "), None);
    }

    #[test]
    fn short_text_is_trimmed_not_changed() {
        let script = BriefingAudioScript::new("  Good morning.  ").unwrap();
        assert_eq!(script.as_str(), "Good morning.");
    }

    #[test]
    fn long_text_is_cut_at_the_last_sentence_that_fits() {
        let sentence = "This is one spoken sentence. ";
        let text = sentence.repeat(200);
        let script = BriefingAudioScript::new(&text).unwrap();
        assert!(script.as_str().chars().count() <= MAX_AUDIO_SCRIPT_CHARS);
        assert!(script.as_str().ends_with("sentence."));
    }

    #[test]
    fn long_text_without_punctuation_is_cut_at_the_limit_on_a_char_boundary() {
        let text = "好".repeat(MAX_AUDIO_SCRIPT_CHARS + 50);
        let script = BriefingAudioScript::new(&text).unwrap();
        assert_eq!(script.as_str().chars().count(), MAX_AUDIO_SCRIPT_CHARS);
    }
}
