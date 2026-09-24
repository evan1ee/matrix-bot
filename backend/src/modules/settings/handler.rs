use crate::db::DbPool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::normalise_currency;
use crate::modules::settings::models::{
    parse_timezone, parse_tts_voice, UserSettings, MAX_MAX_HISTORY_TURNS, MIN_MAX_HISTORY_TURNS,
};
use crate::modules::settings::repository::SettingsRepository;
use crate::utils::{format_datetime, html_escape};

/// A few well-known zones, so `!settings timezone` is answerable without leaving
/// the chat to look up an IANA name. Any of the ~600 real names still works.
const SUGGESTED_ZONES: &[&str] = &[
    "UTC",
    "Asia/Shanghai",
    "Asia/Tokyo",
    "Asia/Singapore",
    "Europe/London",
    "Europe/Berlin",
    "America/New_York",
    "America/Los_Angeles",
];

pub async fn handle_settings_command(
    pool: &DbPool,
    owner_id: &str,
    clean_body: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    let body = clean_body.trim();
    let lower = body.to_lowercase();

    let rest = strip_command(&lower, body)?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let field = parts.next().unwrap_or("").to_lowercase();
    let value = parts.next().unwrap_or("").trim();

    match field.as_str() {
        "" => Some(show(pool, owner_id).await),
        "timezone" | "tz" => Some(set_timezone(pool, owner_id, value, broadcaster).await),
        "currency" | "cur" => Some(set_currency(pool, owner_id, value, broadcaster).await),
        "dev_room" | "dev" | "devroom" | "error_room" => {
            Some(set_dev_room(pool, owner_id, value, broadcaster).await)
        }
        "history" | "turns" | "max_history" => {
            Some(set_history(pool, owner_id, value, broadcaster).await)
        }
        "voice" | "speaker" => Some(set_voice(pool, owner_id, value, broadcaster).await),
        other => Some(format!(
            "⚠️ Unknown setting <code>{}</code>.<br/>{}",
            html_escape(other),
            usage()
        )),
    }
}

/// Matches `!settings` or a bare `settings`, returning the remainder.
fn strip_command<'a>(lower: &str, body: &'a str) -> Option<&'a str> {
    for prefix in ["!settings", "settings"] {
        if lower == prefix {
            return Some("");
        }
        if let Some(stripped) = lower.strip_prefix(&format!("{} ", prefix)) {
            let offset = body.len() - stripped.len();
            return Some(body[offset..].trim());
        }
    }
    None
}

fn usage() -> String {
    format!(
        "Usage: <code>!settings</code>, \
         <code>!settings timezone &lt;IANA zone&gt;</code>, \
         <code>!settings currency &lt;CODE&gt;</code>, \
         <code>!settings dev_room &lt;room_id|none&gt;</code>, \
         <code>!settings history &lt;1-100&gt;</code>, \
         <code>!settings voice &lt;voice|none&gt;</code><br/>\
         <i>Zones: {}</i>",
        SUGGESTED_ZONES.join(", ")
    )
}

async fn show(pool: &DbPool, owner_id: &str) -> String {
    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
    let now = format_datetime(&chrono::Utc::now(), &settings.tz());
    let dev_room_display = settings
        .dev_room_id
        .as_deref()
        .map(|r| format!("<code>{}</code>", html_escape(r)))
        .unwrap_or_else(|| "<i>Not configured</i>".to_string());
    let briefing_display = if settings.briefing_enabled {
        format!("Enabled at <b>{}</b>", html_escape(&settings.briefing_time))
    } else {
        "Disabled".to_string()
    };

    format!(
        "⚙️ <b>Your settings</b><br/>\
         • Timezone: <b>{}</b> — it is currently {}<br/>\
         • Default currency: <b>{}</b><br/>\
         • Dev room: {}<br/>\
         • Max AI history: <b>{} messages</b><br/>\
         • Morning briefing: {}<br/><br/>{}",
        html_escape(&settings.timezone),
        html_escape(&now),
        html_escape(&settings.currency),
        dev_room_display,
        settings.max_history_turns,
        briefing_display,
        usage()
    )
}

async fn set_history(
    pool: &DbPool,
    owner_id: &str,
    value: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    if value.is_empty() {
        return format!("⚠️ How many history messages?<br/>{}", usage());
    }

    let turns: i64 = match value.parse() {
        Ok(n) if (MIN_MAX_HISTORY_TURNS..=MAX_MAX_HISTORY_TURNS).contains(&n) => n,
        _ => {
            return format!(
                "⚠️ Max history messages must be an integer between {} and {}.<br/>Example: <code>!settings history 10</code>",
                MIN_MAX_HISTORY_TURNS, MAX_MAX_HISTORY_TURNS
            );
        }
    };

    let mut updated = SettingsRepository::get_or_default(pool, owner_id).await;
    updated.max_history_turns = turns;

    match save(pool, owner_id, updated, broadcaster).await {
        Ok(saved) => format!(
            "✅ Max AI context history set to <b>{} messages</b>.",
            saved.max_history_turns
        ),
        Err(()) => "❌ Database error saving your setting.".to_string(),
    }
}

async fn set_voice(
    pool: &DbPool,
    owner_id: &str,
    value: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    if value.is_empty() {
        return format!(
            "⚠️ Which voice? A voice name or id (or <code>none</code> to clear).<br/>{}",
            usage()
        );
    }

    // "none"/"clear" resets to no preference, the same clearing keywords
    // `set_dev_room` accepts — a voice name alone has no way to say
    // "forget my choice" the way an empty value would elsewhere, since an
    // empty value here is instead "you gave no argument at all" (above).
    let voice: Option<String> = if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("clear")
    {
        None
    } else {
        match parse_tts_voice(value) {
            Ok(voice) => Some(voice),
            Err(reason) => {
                return format!(
                    "⚠️ {} (or <code>none</code> to clear).<br/>Example: <code>!settings voice alloy</code>",
                    html_escape(&reason)
                );
            }
        }
    };

    let mut updated = SettingsRepository::get_or_default(pool, owner_id).await;
    updated.tts_voice = voice;

    match save(pool, owner_id, updated, broadcaster).await {
        Ok(saved) => match saved.tts_voice {
            Some(voice) => format!("✅ Voice set to <b>{}</b>.", html_escape(&voice)),
            None => "✅ Voice cleared — using the server default.".to_string(),
        },
        Err(()) => "❌ Database error saving your voice.".to_string(),
    }
}

async fn set_timezone(
    pool: &DbPool,
    owner_id: &str,
    value: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    if value.is_empty() {
        return format!("⚠️ Which timezone?<br/>{}", usage());
    }

    let Some(tz) = parse_timezone(value) else {
        return format!(
            "⚠️ <b>{}</b> is not an IANA timezone name.<br/>Try one of: {}",
            html_escape(value),
            SUGGESTED_ZONES.join(", ")
        );
    };

    let mut updated = SettingsRepository::get_or_default(pool, owner_id).await;
    updated.timezone = tz.name().to_string();

    match save(pool, owner_id, updated, broadcaster).await {
        Ok(saved) => format!(
            "✅ Timezone set to <b>{}</b>. It is currently <b>{}</b>.",
            html_escape(&saved.timezone),
            html_escape(&format_datetime(&chrono::Utc::now(), &saved.tz()))
        ),
        Err(()) => "❌ Database error saving your timezone.".to_string(),
    }
}

async fn set_currency(
    pool: &DbPool,
    owner_id: &str,
    value: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    if value.is_empty() {
        return format!("⚠️ Which currency?<br/>{}", usage());
    }

    let currency = match normalise_currency(value) {
        Ok(code) => code,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Example: <code>!settings currency CNY</code>",
                html_escape(&err.to_string())
            )
        }
    };

    let mut updated = SettingsRepository::get_or_default(pool, owner_id).await;
    updated.currency = currency;

    match save(pool, owner_id, updated, broadcaster).await {
        Ok(saved) => format!(
            "✅ Default currency set to <b>{}</b>. \
             <code>!expense add 9.99 coffee</code> now books in {}.",
            html_escape(&saved.currency),
            html_escape(&saved.currency)
        ),
        Err(()) => "❌ Database error saving your currency.".to_string(),
    }
}

async fn set_dev_room(
    pool: &DbPool,
    owner_id: &str,
    value: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let current = SettingsRepository::get_or_default(pool, owner_id).await;
    let dev_room_id = if value.is_empty()
        || value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("clear")
    {
        None
    } else {
        Some(value.to_string())
    };

    let mut updated = current;
    updated.dev_room_id = dev_room_id;

    match save(pool, owner_id, updated, broadcaster).await {
        Ok(saved) => match saved.dev_room_id {
            Some(ref room) => format!(
                "✅ Dev / Error alert room set to <code>{}</code>.",
                html_escape(room)
            ),
            None => "✅ Dev / Error alert room cleared (none).".to_string(),
        },
        Err(()) => "❌ Database error saving your dev room.".to_string(),
    }
}

/// Persists and announces a change.
///
/// The broadcast is what keeps an open web tab honest: every timestamp it shows
/// is formatted against these settings, so a change made over Matrix has to
/// reach the SSE stream or the browser silently keeps rendering the old zone.
async fn save(
    pool: &DbPool,
    owner_id: &str,
    settings: UserSettings,
    broadcaster: Option<&EventBroadcaster>,
) -> Result<UserSettings, ()> {
    let saved = SettingsRepository::upsert(pool, owner_id, &settings)
        .await
        .map_err(|err| error!("Failed to save settings for {}: {:?}", owner_id, err))?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::SettingsUpdated {
            owner_id: owner_id.to_string(),
            settings: saved.clone(),
        });
    }

    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    async fn run(pool: &DbPool, body: &str) -> String {
        handle_settings_command(pool, "@alice:a", body, None)
            .await
            .expect("the handler must claim its own command")
    }

    #[tokio::test]
    async fn test_bare_command_shows_the_current_values() {
        let pool = pool().await;
        let reply = run(&pool, "!settings").await;

        assert!(
            reply.contains("UTC"),
            "should show the default zone: {reply}"
        );
        assert!(
            reply.contains("USD"),
            "should show the default currency: {reply}"
        );
    }

    #[tokio::test]
    async fn test_sets_timezone_and_currency_independently() {
        let pool = pool().await;

        let tz_reply = run(&pool, "!settings timezone Asia/Shanghai").await;
        assert!(tz_reply.contains("Asia/Shanghai"), "{tz_reply}");

        // Setting one must not reset the other.
        let cur_reply = run(&pool, "!settings currency cny").await;
        assert!(
            cur_reply.contains("CNY"),
            "should normalise to upper case: {cur_reply}"
        );

        let shown = run(&pool, "!settings").await;
        assert!(
            shown.contains("Asia/Shanghai"),
            "the zone must survive: {shown}"
        );
        assert!(shown.contains("CNY"), "the currency must survive: {shown}");
    }

    #[tokio::test]
    async fn test_tz_alias_works() {
        let pool = pool().await;
        let reply = run(&pool, "!settings tz Europe/Berlin").await;
        assert!(reply.contains("Europe/Berlin"), "{reply}");
    }

    /// A rejected value must leave the stored settings untouched, and say why.
    #[tokio::test]
    async fn test_rejects_bad_values_without_changing_anything() {
        let pool = pool().await;
        run(&pool, "!settings timezone Asia/Shanghai").await;

        for bad in [
            "!settings timezone GMT+8",
            "!settings timezone Mars/Olympus",
        ] {
            let reply = run(&pool, bad).await;
            assert!(reply.contains("not an IANA"), "{bad} -> {reply}");
        }
        let reply = run(&pool, "!settings currency EURO").await;
        assert!(reply.contains("3-letter"), "{reply}");

        let shown = run(&pool, "!settings").await;
        assert!(
            shown.contains("Asia/Shanghai"),
            "a rejected value must not clobber: {shown}"
        );
        assert!(
            shown.contains("USD"),
            "a rejected currency must not clobber: {shown}"
        );
    }

    #[tokio::test]
    async fn test_names_a_missing_value_rather_than_saving_an_empty_one() {
        let pool = pool().await;

        assert!(run(&pool, "!settings timezone")
            .await
            .contains("Which timezone"));
        assert!(run(&pool, "!settings currency")
            .await
            .contains("Which currency"));
        assert!(run(&pool, "!settings colour blue")
            .await
            .contains("Unknown setting"));
    }

    /// User text reaches an HTML message, so it has to be escaped.
    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let pool = pool().await;
        let reply = run(&pool, "!settings timezone <script>alert(1)</script>").await;

        assert!(
            !reply.contains("<script>"),
            "raw markup must not survive: {reply}"
        );
        assert!(reply.contains("&lt;script&gt;"));
    }

    /// An open web client formats every timestamp against these settings, so a
    /// change made over Matrix has to reach the SSE stream.
    #[tokio::test]
    async fn test_broadcasts_the_change_to_the_owner() {
        let pool = pool().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        handle_settings_command(
            &pool,
            "@alice:a",
            "!settings tz Asia/Tokyo",
            Some(&broadcaster),
        )
        .await
        .unwrap();

        let event = rx.try_recv().expect("a settings change must be broadcast");
        assert_eq!(event.owner_id(), "@alice:a");
        assert!(matches!(event, SystemEvent::SettingsUpdated { .. }));
    }

    /// A rejected value changed nothing, so it must not announce a change either.
    #[tokio::test]
    async fn test_does_not_broadcast_when_nothing_was_saved() {
        let pool = pool().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        handle_settings_command(
            &pool,
            "@alice:a",
            "!settings tz Mars/Olympus",
            Some(&broadcaster),
        )
        .await
        .unwrap();
        handle_settings_command(&pool, "@alice:a", "!settings", Some(&broadcaster))
            .await
            .unwrap();

        assert!(
            rx.try_recv().is_err(),
            "a read or a rejection must not broadcast"
        );
    }

    #[tokio::test]
    async fn test_sets_dev_room_and_clears_it() {
        let pool = pool().await;

        let reply = run(&pool, "!settings dev_room !room123:mbm.one").await;
        assert!(reply.contains("!room123:mbm.one"), "{reply}");

        let shown = run(&pool, "!settings").await;
        assert!(shown.contains("!room123:mbm.one"), "{shown}");

        let clear_reply = run(&pool, "!settings dev none").await;
        assert!(clear_reply.contains("cleared"), "{clear_reply}");

        let shown2 = run(&pool, "!settings").await;
        assert!(shown2.contains("Not configured"), "{shown2}");
    }

    #[tokio::test]
    async fn test_sets_max_history_turns() {
        let pool = pool().await;

        let reply = run(&pool, "!settings history 25").await;
        assert!(reply.contains("25 messages"), "{reply}");

        let shown = run(&pool, "!settings").await;
        assert!(shown.contains("25 messages"), "{shown}");

        // Rejects invalid values
        let bad = run(&pool, "!settings history 0").await;
        assert!(bad.contains("between 1 and 100"), "{bad}");

        let bad2 = run(&pool, "!settings history 150").await;
        assert!(bad2.contains("between 1 and 100"), "{bad2}");

        let bad3 = run(&pool, "!settings history abc").await;
        assert!(bad3.contains("between 1 and 100"), "{bad3}");

        // Previous setting remains intact
        let shown2 = run(&pool, "!settings").await;
        assert!(shown2.contains("25 messages"), "{shown2}");
    }

    #[tokio::test]
    async fn test_sets_voice_and_the_speaker_alias_works() {
        let pool = pool().await;

        let reply = run(&pool, "!settings voice alloy").await;
        assert!(reply.contains("alloy"), "{reply}");
        assert_eq!(
            SettingsRepository::get(&pool, "@alice:a")
                .await
                .unwrap()
                .tts_voice
                .as_deref(),
            Some("alloy")
        );

        let reply2 = run(&pool, "!settings speaker 21m00Tcm4TlvDq8ikWAM").await;
        assert!(reply2.contains("21m00Tcm4TlvDq8ikWAM"), "{reply2}");
        assert_eq!(
            SettingsRepository::get(&pool, "@alice:a")
                .await
                .unwrap()
                .tts_voice
                .as_deref(),
            Some("21m00Tcm4TlvDq8ikWAM")
        );
    }

    /// Speaker 0 is a real, selectable voice — saving it must not be
    /// confused with clearing the preference.
    #[tokio::test]
    async fn test_sets_voice_to_zero_and_zero_survives() {
        let pool = pool().await;

        let reply = run(&pool, "!settings voice 0").await;
        assert!(reply.contains('0'), "{reply}");

        let settings = SettingsRepository::get(&pool, "@alice:a").await.unwrap();
        assert_eq!(settings.tts_voice.as_deref(), Some("0"));
    }

    #[tokio::test]
    async fn test_clearing_the_voice_resets_to_no_preference() {
        let pool = pool().await;
        run(&pool, "!settings voice alloy").await;

        let reply = run(&pool, "!settings voice none").await;
        assert!(reply.contains("cleared"), "{reply}");

        let settings = SettingsRepository::get(&pool, "@alice:a").await.unwrap();
        assert_eq!(settings.tts_voice, None);
    }

    #[tokio::test]
    async fn test_rejects_a_malformed_voice_without_changing_anything() {
        let pool = pool().await;
        run(&pool, "!settings voice alloy").await;

        let too_long = "a".repeat(65);
        for bad in [
            "!settings voice a/b".to_string(),
            "!settings voice ../x".to_string(),
            format!("!settings voice {}", too_long),
        ] {
            let reply = run(&pool, &bad).await;
            assert!(reply.contains("⚠️"), "{bad} -> {reply}");
        }

        let settings = SettingsRepository::get(&pool, "@alice:a").await.unwrap();
        assert_eq!(
            settings.tts_voice.as_deref(),
            Some("alloy"),
            "a rejected value must not clobber the saved voice"
        );
    }

    #[tokio::test]
    async fn test_voice_broadcasts_on_success_and_not_on_rejection() {
        let pool = pool().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        handle_settings_command(
            &pool,
            "@alice:a",
            "!settings voice alloy",
            Some(&broadcaster),
        )
        .await
        .unwrap();
        let event = rx.try_recv().expect("a valid voice change must broadcast");
        assert!(matches!(event, SystemEvent::SettingsUpdated { .. }));

        handle_settings_command(&pool, "@alice:a", "!settings voice a/b", Some(&broadcaster))
            .await
            .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "a rejected voice must not broadcast"
        );
    }
}
