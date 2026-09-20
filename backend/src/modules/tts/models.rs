use std::path::PathBuf;
use std::time::Duration;

/// Configuration for the text-to-speech module.
///
/// `enabled` defaults to `false` (see `Config::from_env`'s `TTS_ENABLED`),
/// unlike OCR and voice which default on — the Kokoro model this module loads
/// is 311 MB, and that should not start loading just because someone pulled
/// this branch.
#[derive(Debug, Clone)]
pub struct TtsConfig {
    pub enabled: bool,
    pub model_dir: PathBuf,
    pub speaker_id: i32,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_dir: PathBuf::from("data/tts_models/kokoro-multi-lang-v1_1"),
            speaker_id: 0,
        }
    }
}

/// Errors that can occur while preparing or running text-to-speech.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("Text-to-speech feature is disabled")]
    Disabled,
    #[error("Missing TTS model files in {0:?}")]
    ModelMissing(PathBuf),
    #[error("No speakable text remained after preparing it for synthesis")]
    EmptyText,
    #[error("Text is too long to speak (max {max} characters)")]
    TextTooLong { max: usize },
    #[error("TTS engine error: {0}")]
    Engine(String),
}

/// Synthesised audio: raw samples plus the sample rate they were generated at.
#[derive(Debug, Clone)]
pub struct Speech {
    pub samples: Vec<f32>,
    pub sample_rate: i32,
}

impl Speech {
    /// The clip's length, derived from sample count and rate rather than
    /// stored separately, so the two can never disagree.
    pub fn duration(&self) -> Duration {
        if self.sample_rate <= 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_disabled_and_points_at_the_kokoro_model() {
        let config = TtsConfig::default();
        assert!(!config.enabled);
        assert_eq!(
            config.model_dir,
            PathBuf::from("data/tts_models/kokoro-multi-lang-v1_1")
        );
        assert_eq!(config.speaker_id, 0);
    }

    #[test]
    fn test_duration_matches_sample_count_and_rate() {
        let speech = Speech {
            samples: vec![0.0; 24000],
            sample_rate: 24000,
        };
        assert_eq!(speech.duration(), Duration::from_secs(1));
    }

    #[test]
    fn test_duration_is_zero_for_an_invalid_sample_rate() {
        let speech = Speech {
            samples: vec![0.0; 100],
            sample_rate: 0,
        };
        assert_eq!(speech.duration(), Duration::ZERO);
    }
}
