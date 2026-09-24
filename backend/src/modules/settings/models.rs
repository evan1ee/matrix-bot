use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

/// Assumed when a user has never saved a preference.
///
/// UTC is the only defensible default: every timestamp is stored in UTC, so a
/// user who has expressed no preference sees the value exactly as recorded
/// rather than shifted into a zone that was guessed for them.
pub const DEFAULT_TIMEZONE: &str = "UTC";
/// Assumed when a user has never saved a preference, and when an `!expense add`
/// names no currency.
pub const DEFAULT_CURRENCY: &str = "USD";
/// Default number of recent messages to retain in AI context (turns).
pub const DEFAULT_MAX_HISTORY_TURNS: i64 = 10;
pub const MIN_MAX_HISTORY_TURNS: i64 = 1;
pub const MAX_MAX_HISTORY_TURNS: i64 = 100;
pub const DEFAULT_BRIEFING_ENABLED: bool = true;
pub const DEFAULT_BRIEFING_TIME: &str = "07:00";
/// The longest voice name or id stored. ElevenLabs ids are 20 characters and
/// OpenAI's names are shorter; this only refuses the absurd.
pub const MAX_TTS_VOICE_LEN: usize = 64;

fn default_max_history_turns() -> i64 {
    DEFAULT_MAX_HISTORY_TURNS
}

fn default_briefing_enabled() -> bool {
    DEFAULT_BRIEFING_ENABLED
}

fn default_briefing_time() -> String {
    DEFAULT_BRIEFING_TIME.to_string()
}

/// One user's display preferences, keyed by the same `owner_id` every other
/// ownership check uses (`AuthUser::owner_id`, the `owner_id` column).
///
/// Deliberately shared by both front doors: the SPA renders timestamps in
/// `timezone` and pre-selects `currency`, and the Matrix command handlers format
/// their HTML replies with the same two values, so a user sees one consistent
/// answer no matter which door they came through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow, ToSchema)]
pub struct UserSettings {
    /// IANA zone name, e.g. `Asia/Shanghai`. Validated on write.
    pub timezone: String,
    /// ISO 4217 code, e.g. `CNY`. Validated on write.
    pub currency: String,
    /// Matrix room ID where development errors and alerts are sent.
    #[serde(default)]
    pub dev_room_id: Option<String>,
    /// Number of recent conversation messages included as context for AI turns.
    #[serde(default = "default_max_history_turns")]
    pub max_history_turns: i64,
    /// Whether automated morning briefing is enabled.
    #[serde(default = "default_briefing_enabled")]
    pub briefing_enabled: bool,
    /// Scheduled delivery time in 24-hour HH:MM format (e.g. 07:00).
    #[serde(default = "default_briefing_time")]
    pub briefing_time: String,
    /// The voice `!say` speaks in, in whatever the configured provider calls
    /// it. `None` means no preference, and `modules::tts` falls back to the
    /// operator's `TTS_VOICE`: the same "no row until you change something"
    /// convention `dev_room_id` uses. `Some("0")` is a real Kokoro voice, not
    /// a stand-in for "unset". Shape-checked by `parse_tts_voice` on write.
    #[serde(default)]
    pub tts_voice: Option<String>,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            timezone: DEFAULT_TIMEZONE.to_string(),
            currency: DEFAULT_CURRENCY.to_string(),
            dev_room_id: None,
            max_history_turns: DEFAULT_MAX_HISTORY_TURNS,
            briefing_enabled: DEFAULT_BRIEFING_ENABLED,
            briefing_time: DEFAULT_BRIEFING_TIME.to_string(),
            tts_voice: None,
        }
    }
}

impl UserSettings {
    /// The stored zone as a `Tz`, falling back to UTC.
    ///
    /// A stored value can only be unparseable if it predates validation or was
    /// written straight to the database, and neither is worth failing a command
    /// over — rendering in UTC degrades the display, whereas returning an error
    /// would swallow the reply entirely.
    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(Tz::UTC)
    }
}

/// Parses an IANA zone name, e.g. `Europe/Berlin`.
///
/// `chrono-tz` carries the full tzdata table, so this accepts exactly the names
/// the browser's `Intl.supportedValuesOf('timeZone')` offers in the settings
/// picker, and rejects anything else.
pub fn parse_timezone(raw: &str) -> Option<Tz> {
    raw.trim().parse().ok()
}

/// Validates and parses a 24-hour time string "HH:MM" -> (hour, minute).
pub fn parse_briefing_time(raw: &str) -> Result<(u32, u32), String> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() != 2 {
        return Err("Time must be in HH:MM format (e.g. 07:00)".to_string());
    }
    let hour = parts[0]
        .parse::<u32>()
        .map_err(|_| "Hour must be a valid number between 00 and 23".to_string())?;
    let minute = parts[1]
        .parse::<u32>()
        .map_err(|_| "Minute must be a valid number between 00 and 59".to_string())?;

    if hour > 23 || minute > 59 {
        return Err("Time out of 24-hour range (00:00 - 23:59)".to_string());
    }

    Ok((hour, minute))
}

/// Checks that `raw` could be a voice. What voices exist depends on the
/// configured provider, which the bot does not know. A Kokoro speaker id, an
/// OpenAI name and an ElevenLabs id all fit this shape, and a voice the
/// provider does not have fails at `!say` time with the provider's own error.
pub fn parse_tts_voice(raw: &str) -> Result<String, String> {
    let voice = raw.trim();
    let valid = !voice.is_empty()
        && voice.len() <= MAX_TTS_VOICE_LEN
        && voice
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(voice.to_string())
    } else {
        Err(format!(
            "A voice is 1-{} letters, digits, '_' or '-' (e.g. 0, alloy, 21m00Tcm4TlvDq8ikWAM)",
            MAX_TTS_VOICE_LEN
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_utc_and_usd() {
        let settings = UserSettings::default();
        assert_eq!(settings.timezone, "UTC");
        assert_eq!(settings.currency, "USD");
        assert_eq!(settings.dev_room_id, None);
        assert_eq!(settings.max_history_turns, DEFAULT_MAX_HISTORY_TURNS);
        assert!(settings.briefing_enabled);
        assert_eq!(settings.briefing_time, "07:00");
        assert_eq!(settings.tts_voice, None);
        assert_eq!(settings.tz(), Tz::UTC);
    }

    #[test]
    fn test_parses_briefing_time() {
        assert_eq!(parse_briefing_time("07:00"), Ok((7, 0)));
        assert_eq!(parse_briefing_time("7:00"), Ok((7, 0)));
        assert_eq!(parse_briefing_time("23:59"), Ok((23, 59)));
        assert_eq!(parse_briefing_time("00:00"), Ok((0, 0)));
        assert!(parse_briefing_time("24:00").is_err());
        assert!(parse_briefing_time("07:60").is_err());
        assert!(parse_briefing_time("invalid").is_err());
    }

    #[test]
    fn test_parses_iana_zone_names() {
        assert_eq!(parse_timezone("Asia/Shanghai"), Some(Tz::Asia__Shanghai));
        assert_eq!(parse_timezone(" Europe/Berlin "), Some(Tz::Europe__Berlin));
        assert_eq!(parse_timezone("UTC"), Some(Tz::UTC));
    }

    /// The picker sends IANA names; anything else is a client bug or an attack,
    /// and storing it would make every later render fall back to UTC silently.
    #[test]
    fn test_rejects_anything_that_is_not_a_zone() {
        assert_eq!(parse_timezone("Mars/Olympus"), None);
        assert_eq!(parse_timezone("GMT+8"), None);
        assert_eq!(parse_timezone(""), None);
        assert_eq!(parse_timezone("+08:00"), None);
    }

    /// An unparseable stored value must degrade to UTC, never panic — the value
    /// is read on the command path, where an error would eat the whole reply.
    #[test]
    fn test_unparseable_stored_zone_degrades_to_utc() {
        let settings = UserSettings {
            timezone: "Nowhere/Nothing".to_string(),
            currency: "USD".to_string(),
            dev_room_id: None,
            max_history_turns: DEFAULT_MAX_HISTORY_TURNS,
            briefing_enabled: true,
            briefing_time: "07:00".to_string(),
            tts_voice: None,
        };
        assert_eq!(settings.tz(), Tz::UTC);
    }

    #[test]
    fn test_parse_tts_voice_accepts_every_provider_shape() {
        assert_eq!(parse_tts_voice("0"), Ok("0".to_string()));
        assert_eq!(parse_tts_voice(" alloy "), Ok("alloy".to_string()));
        assert_eq!(
            parse_tts_voice("21m00Tcm4TlvDq8ikWAM"),
            Ok("21m00Tcm4TlvDq8ikWAM".to_string())
        );
        assert_eq!(
            parse_tts_voice("zf_xiaobei-2"),
            Ok("zf_xiaobei-2".to_string())
        );
    }

    #[test]
    fn test_parse_tts_voice_rejects_anything_that_is_not_a_voice_name() {
        let too_long = "a".repeat(MAX_TTS_VOICE_LEN + 1);
        for bad in [
            "",
            "   ",
            "a b",
            "../x",
            "a/b",
            "é",
            "a?b",
            too_long.as_str(),
        ] {
            assert!(parse_tts_voice(bad).is_err(), "accepted {:?}", bad);
        }
    }
}
