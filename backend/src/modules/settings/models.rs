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

fn default_max_history_turns() -> i64 {
    DEFAULT_MAX_HISTORY_TURNS
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
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            timezone: DEFAULT_TIMEZONE.to_string(),
            currency: DEFAULT_CURRENCY.to_string(),
            dev_room_id: None,
            max_history_turns: DEFAULT_MAX_HISTORY_TURNS,
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
        assert_eq!(settings.tz(), Tz::UTC);
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
        };
        assert_eq!(settings.tz(), Tz::UTC);
    }
}
