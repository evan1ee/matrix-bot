//! Per-user display preferences: timezone, default currency, and a handful
//! of related toggles.
//!
//! Unusually for a feature, its data is read by nearly every other one: the
//! todo and reminder handlers format timestamps against the sender's zone,
//! the finance and portfolio handlers default an amount's currency to it,
//! and the web UI reads the same row through the HTTP API. A preference
//! therefore reads the same whichever front door produced it.
//!
//! `UserSettings` keeps plain `pub` fields rather than a normalising
//! constructor: unlike `JoinedRoom`/`CachedRoom`, it has no cross-field
//! invariant to protect — every field is independently validated by its own
//! `parse_*` function before a `UserSettings` is ever built, and a value read
//! back from storage is accepted as-is (an unparseable stored timezone
//! degrades to UTC in `tz()`, rather than being rejected). AI tools also read
//! the caller's settings and mutate individual fields directly
//! (`modules/ai/tools/settings.rs`).

use chrono_tz::Tz;
use serde::Serialize;

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

/// Why a raw value could not become a setting. Each variant's message is the
/// exact text an adapter has always shown for that failure, so converting one
/// into a reply (chat) or a 400 body (`AppError::Validation` via
/// `application::settings`) changes nothing a caller can observe.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("Unknown timezone. Use an IANA name such as Asia/Shanghai")]
    InvalidTimezone,
    #[error("Currency must be a 3-letter code such as EUR or JPY")]
    InvalidCurrency,
    #[error("max_history_turns must be between {min} and {max}")]
    HistoryTurnsOutOfRange { min: i64, max: i64 },
    #[error("{0}")]
    InvalidBriefingTime(String),
    #[error("{0}")]
    InvalidVoice(String),
}

/// One user's display preferences, keyed by the same `owner_id` every other
/// ownership check uses (`AuthUser::owner_id`, the `owner_id` column).
///
/// Deliberately shared by both front doors: the SPA renders timestamps in
/// `timezone` and pre-selects `currency`, and the Matrix command handlers
/// format their HTML replies with the same values, so a user sees one
/// consistent answer no matter which door they came through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserSettings {
    /// IANA zone name, e.g. `Asia/Shanghai`. Validated on write.
    pub timezone: String,
    /// ISO 4217 code, e.g. `CNY`. Validated on write.
    pub currency: String,
    /// Matrix room ID where development errors and alerts are sent.
    pub dev_room_id: Option<String>,
    /// Number of recent conversation messages included as context for AI turns.
    pub max_history_turns: i64,
    /// Whether automated morning briefing is enabled.
    pub briefing_enabled: bool,
    /// Scheduled delivery time in 24-hour HH:MM format (e.g. 07:00).
    pub briefing_time: String,
    /// The voice `!say` speaks in, in whatever the configured provider calls
    /// it. `None` means no preference, and `modules::tts` falls back to the
    /// operator's `TTS_VOICE`: the same "no row until you change something"
    /// convention `dev_room_id` uses. `Some("0")` is a real Kokoro voice, not
    /// a stand-in for "unset". Shape-checked by `parse_tts_voice` on write.
    pub tts_voice: Option<String>,
}

impl Default for UserSettings {
    /// The domain rule AGENTS.md documents: a missing `user_settings` row
    /// means these defaults, never an error. `application::settings`'s
    /// `get`/`get_or_default` return this when the repository finds no row.
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

/// A whole-row settings replacement, as raw unvalidated input. Mirrors the
/// HTTP PUT body one field at a time; only `UserSettings::apply_update` and
/// its HTTP adapter (`interface/http/settings.rs`) build one of these.
pub struct RawSettingsUpdate {
    pub timezone: String,
    pub currency: String,
    pub dev_room_id: Option<String>,
    pub max_history_turns: Option<i64>,
    pub briefing_enabled: Option<bool>,
    pub briefing_time: Option<String>,
    pub tts_voice: Option<String>,
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

    /// Validates and merges a whole-row replacement against the caller's
    /// current settings ("settings are saved as a whole row" — there is no
    /// partial-update path). `dev_room_id` and `tts_voice` clear to `None`
    /// when `raw` omits them; every other field keeps `current`'s value.
    pub fn apply_update(
        current: &UserSettings,
        raw: RawSettingsUpdate,
    ) -> Result<UserSettings, SettingsError> {
        let timezone = parse_timezone(&raw.timezone).ok_or(SettingsError::InvalidTimezone)?;
        let currency = parse_currency(&raw.currency)?;

        let dev_room_id = raw.dev_room_id.and_then(|r| {
            let trimmed = r.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let max_history_turns = match raw.max_history_turns {
            Some(turns) => parse_max_history_turns(turns)?,
            None => current.max_history_turns,
        };

        let briefing_enabled = raw.briefing_enabled.unwrap_or(current.briefing_enabled);

        let briefing_time = match raw.briefing_time {
            Some(ref bt) => {
                parse_briefing_time(bt)?;
                bt.trim().to_string()
            }
            None => current.briefing_time.clone(),
        };

        // Unlike `max_history_turns` or `briefing_time`, a missing/null value
        // here is not "leave the current value alone" — it directly clears
        // the preference, the same as `dev_room_id` above.
        let tts_voice = match raw.tts_voice.as_deref() {
            Some(v) => Some(parse_tts_voice(v)?),
            None => None,
        };

        Ok(UserSettings {
            timezone: timezone.name().to_string(),
            currency,
            dev_room_id,
            max_history_turns,
            briefing_enabled,
            briefing_time,
            tts_voice,
        })
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

/// Normalises a currency code, rejecting anything that is not three letters.
///
/// Deliberately not a call into `modules::finance::money`: that module's
/// `normalise_currency` shares this exact rule, but reaching into finance for
/// three lines of validation would make settings depend on a feature that
/// depends on settings for nothing in return — see the Task 2 report for the
/// coupling this replaced.
pub fn parse_currency(raw: &str) -> Result<String, SettingsError> {
    let code = raw.trim().to_uppercase();
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
        Ok(code)
    } else {
        Err(SettingsError::InvalidCurrency)
    }
}

/// Validates a max-history-turns value against the documented bounds.
pub fn parse_max_history_turns(turns: i64) -> Result<i64, SettingsError> {
    if (MIN_MAX_HISTORY_TURNS..=MAX_MAX_HISTORY_TURNS).contains(&turns) {
        Ok(turns)
    } else {
        Err(SettingsError::HistoryTurnsOutOfRange {
            min: MIN_MAX_HISTORY_TURNS,
            max: MAX_MAX_HISTORY_TURNS,
        })
    }
}

/// Validates and parses a 24-hour time string "HH:MM" -> (hour, minute).
pub fn parse_briefing_time(raw: &str) -> Result<(u32, u32), SettingsError> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() != 2 {
        return Err(SettingsError::InvalidBriefingTime(
            "Time must be in HH:MM format (e.g. 07:00)".to_string(),
        ));
    }
    let hour = parts[0].parse::<u32>().map_err(|_| {
        SettingsError::InvalidBriefingTime(
            "Hour must be a valid number between 00 and 23".to_string(),
        )
    })?;
    let minute = parts[1].parse::<u32>().map_err(|_| {
        SettingsError::InvalidBriefingTime(
            "Minute must be a valid number between 00 and 59".to_string(),
        )
    })?;

    if hour > 23 || minute > 59 {
        return Err(SettingsError::InvalidBriefingTime(
            "Time out of 24-hour range (00:00 - 23:59)".to_string(),
        ));
    }

    Ok((hour, minute))
}

/// Checks that `raw` could be a voice. What voices exist depends on the
/// configured provider, which the bot does not know. A Kokoro speaker id, an
/// OpenAI name and an ElevenLabs id all fit this shape, and a voice the
/// provider does not have fails at `!say` time with the provider's own error.
pub fn parse_tts_voice(raw: &str) -> Result<String, SettingsError> {
    let voice = raw.trim();
    let valid = !voice.is_empty()
        && voice.len() <= MAX_TTS_VOICE_LEN
        && voice
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(voice.to_string())
    } else {
        Err(SettingsError::InvalidVoice(format!(
            "A voice is 1-{} letters, digits, '_' or '-' (e.g. 0, alloy, 21m00Tcm4TlvDq8ikWAM)",
            MAX_TTS_VOICE_LEN
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(timezone: &str, currency: &str) -> RawSettingsUpdate {
        RawSettingsUpdate {
            timezone: timezone.to_string(),
            currency: currency.to_string(),
            dev_room_id: None,
            max_history_turns: None,
            briefing_enabled: None,
            briefing_time: None,
            tts_voice: None,
        }
    }

    /// AGENTS.md's documented invariant: a missing row means these defaults,
    /// never an error. `application::settings::SettingsService::get`/
    /// `get_or_default` rely on `UserSettings::default()` for exactly this.
    #[test]
    fn a_missing_row_uses_the_documented_defaults() {
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
            ..UserSettings::default()
        };
        assert_eq!(settings.tz(), Tz::UTC);
    }

    #[test]
    fn test_parse_currency_accepts_three_letter_codes() {
        assert_eq!(parse_currency("cny"), Ok("CNY".to_string()));
        assert_eq!(parse_currency(" eur "), Ok("EUR".to_string()));
        assert_eq!(parse_currency("USD"), Ok("USD".to_string()));
    }

    #[test]
    fn test_parse_currency_rejects_anything_that_is_not_three_letters() {
        for bad in ["EURO", "E1R", "", "US"] {
            assert_eq!(parse_currency(bad), Err(SettingsError::InvalidCurrency));
        }
    }

    #[test]
    fn test_parse_max_history_turns_enforces_bounds() {
        assert_eq!(parse_max_history_turns(1), Ok(1));
        assert_eq!(parse_max_history_turns(100), Ok(100));
        assert!(parse_max_history_turns(0).is_err());
        assert!(parse_max_history_turns(101).is_err());
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

    #[test]
    fn apply_update_validates_timezone_before_anything_else() {
        let current = UserSettings::default();
        let err = UserSettings::apply_update(&current, raw("Mars/Olympus", "USD")).unwrap_err();
        assert_eq!(err, SettingsError::InvalidTimezone);
    }

    #[test]
    fn apply_update_validates_currency() {
        let current = UserSettings::default();
        let err = UserSettings::apply_update(&current, raw("UTC", "EURO")).unwrap_err();
        assert_eq!(err, SettingsError::InvalidCurrency);
    }

    #[test]
    fn apply_update_keeps_fields_absent_from_raw() {
        let current = UserSettings {
            max_history_turns: 42,
            briefing_time: "08:15".to_string(),
            briefing_enabled: false,
            ..UserSettings::default()
        };
        let updated = UserSettings::apply_update(&current, raw("Asia/Tokyo", "JPY")).unwrap();
        assert_eq!(updated.max_history_turns, 42);
        assert_eq!(updated.briefing_time, "08:15");
        assert!(!updated.briefing_enabled);
    }

    /// Unlike `max_history_turns`/`briefing_time`, an absent `dev_room_id` or
    /// `tts_voice` clears the preference rather than keeping the old value.
    #[test]
    fn apply_update_clears_dev_room_and_voice_when_absent() {
        let current = UserSettings {
            dev_room_id: Some("!dev:x".to_string()),
            tts_voice: Some("alloy".to_string()),
            ..UserSettings::default()
        };
        let updated = UserSettings::apply_update(&current, raw("UTC", "USD")).unwrap();
        assert_eq!(updated.dev_room_id, None);
        assert_eq!(updated.tts_voice, None);
    }

    #[test]
    fn apply_update_rejects_an_out_of_range_history_value_without_changing_anything() {
        let current = UserSettings {
            max_history_turns: 5,
            ..UserSettings::default()
        };
        let mut r = raw("UTC", "USD");
        r.max_history_turns = Some(0);
        let err = UserSettings::apply_update(&current, r).unwrap_err();
        assert_eq!(
            err,
            SettingsError::HistoryTurnsOutOfRange { min: 1, max: 100 }
        );
    }
}
