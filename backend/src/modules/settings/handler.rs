use sqlx::SqlitePool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::normalise_currency;
use crate::modules::settings::models::{parse_timezone, UserSettings};
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
    pool: &SqlitePool,
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
         <code>!settings currency &lt;CODE&gt;</code><br/>\
         <i>Zones: {}</i>",
        SUGGESTED_ZONES.join(", ")
    )
}

async fn show(pool: &SqlitePool, owner_id: &str) -> String {
    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
    let now = format_datetime(&chrono::Utc::now(), &settings.tz());

    format!(
        "⚙️ <b>Your settings</b><br/>\
         • Timezone: <b>{}</b> — it is currently {}<br/>\
         • Default currency: <b>{}</b><br/><br/>{}",
        html_escape(&settings.timezone),
        html_escape(&now),
        html_escape(&settings.currency),
        usage()
    )
}

async fn set_timezone(
    pool: &SqlitePool,
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

    let current = SettingsRepository::get_or_default(pool, owner_id).await;
    let updated = UserSettings {
        timezone: tz.name().to_string(),
        currency: current.currency,
    };

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
    pool: &SqlitePool,
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

    let current = SettingsRepository::get_or_default(pool, owner_id).await;
    let updated = UserSettings { timezone: current.timezone, currency };

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

/// Persists and announces a change.
///
/// The broadcast is what keeps an open web tab honest: every timestamp it shows
/// is formatted against these settings, so a change made over Matrix has to
/// reach the SSE stream or the browser silently keeps rendering the old zone.
async fn save(
    pool: &SqlitePool,
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
    use crate::db::schema::run_migrations;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn run(pool: &SqlitePool, body: &str) -> String {
        handle_settings_command(pool, "@alice:a", body, None)
            .await
            .expect("the handler must claim its own command")
    }

    #[tokio::test]
    async fn test_bare_command_shows_the_current_values() {
        let pool = pool().await;
        let reply = run(&pool, "!settings").await;

        assert!(reply.contains("UTC"), "should show the default zone: {reply}");
        assert!(reply.contains("USD"), "should show the default currency: {reply}");
    }

    #[tokio::test]
    async fn test_sets_timezone_and_currency_independently() {
        let pool = pool().await;

        let tz_reply = run(&pool, "!settings timezone Asia/Shanghai").await;
        assert!(tz_reply.contains("Asia/Shanghai"), "{tz_reply}");

        // Setting one must not reset the other.
        let cur_reply = run(&pool, "!settings currency cny").await;
        assert!(cur_reply.contains("CNY"), "should normalise to upper case: {cur_reply}");

        let shown = run(&pool, "!settings").await;
        assert!(shown.contains("Asia/Shanghai"), "the zone must survive: {shown}");
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

        for bad in ["!settings timezone GMT+8", "!settings timezone Mars/Olympus"] {
            let reply = run(&pool, bad).await;
            assert!(reply.contains("not an IANA"), "{bad} -> {reply}");
        }
        let reply = run(&pool, "!settings currency EURO").await;
        assert!(reply.contains("3-letter"), "{reply}");

        let shown = run(&pool, "!settings").await;
        assert!(shown.contains("Asia/Shanghai"), "a rejected value must not clobber: {shown}");
        assert!(shown.contains("USD"), "a rejected currency must not clobber: {shown}");
    }

    #[tokio::test]
    async fn test_names_a_missing_value_rather_than_saving_an_empty_one() {
        let pool = pool().await;

        assert!(run(&pool, "!settings timezone").await.contains("Which timezone"));
        assert!(run(&pool, "!settings currency").await.contains("Which currency"));
        assert!(run(&pool, "!settings colour blue").await.contains("Unknown setting"));
    }

    /// User text reaches an HTML message, so it has to be escaped.
    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let pool = pool().await;
        let reply = run(&pool, "!settings timezone <script>alert(1)</script>").await;

        assert!(!reply.contains("<script>"), "raw markup must not survive: {reply}");
        assert!(reply.contains("&lt;script&gt;"));
    }

    /// An open web client formats every timestamp against these settings, so a
    /// change made over Matrix has to reach the SSE stream.
    #[tokio::test]
    async fn test_broadcasts_the_change_to_the_owner() {
        let pool = pool().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        handle_settings_command(&pool, "@alice:a", "!settings tz Asia/Tokyo", Some(&broadcaster))
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

        handle_settings_command(&pool, "@alice:a", "!settings tz Mars/Olympus", Some(&broadcaster))
            .await
            .unwrap();
        handle_settings_command(&pool, "@alice:a", "!settings", Some(&broadcaster))
            .await
            .unwrap();

        assert!(rx.try_recv().is_err(), "a read or a rejection must not broadcast");
    }
}
