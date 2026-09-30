//! The Matrix side of settings: `!settings` and its subcommands, calling
//! `SettingsService` for every read and write.

use async_trait::async_trait;

use crate::application::settings::SettingsService;
use crate::application::AppServices;
use crate::db::Database;
use crate::domain::settings::{
    parse_currency, parse_max_history_turns, parse_timezone, parse_tts_voice, UserSettings,
    MAX_MAX_HISTORY_TURNS, MIN_MAX_HISTORY_TURNS,
};
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
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

pub struct SettingsModule;

impl Default for SettingsModule {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for SettingsModule {
    fn name(&self) -> &str {
        "Settings"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "settings".to_string(),
                usage: "!settings".to_string(),
                description: "Show your timezone and default currency".to_string(),
            },
            CommandHelp {
                name: "settings timezone".to_string(),
                usage: "!settings timezone <IANA zone>".to_string(),
                description: "Set the timezone dates are shown in \
                              (e.g. !settings timezone Asia/Shanghai)"
                    .to_string(),
            },
            CommandHelp {
                name: "settings currency".to_string(),
                usage: "!settings currency <CODE>".to_string(),
                description: "Set the currency assumed when an amount names none \
                              (e.g. !settings currency CNY)"
                    .to_string(),
            },
            CommandHelp {
                name: "settings voice".to_string(),
                usage: "!settings voice <voice|none>".to_string(),
                description: "Set the voice !say speaks in: a name or id your TTS provider \
                              knows (e.g. !settings voice 0)"
                    .to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
    ) -> Option<String> {
        handle_settings_command(&services.settings, &ctx.owner_id, &ctx.full_body).await
    }
}

use crate::domain::accounts::AccountId;

async fn handle_settings_command(
    settings: &SettingsService,
    owner_id: &AccountId,
    clean_body: &str,
) -> Option<String> {
    let body = clean_body.trim();
    let lower = body.to_lowercase();

    let rest = strip_command(&lower, body)?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let field = parts.next().unwrap_or("").to_lowercase();
    let value = parts.next().unwrap_or("").trim();

    match field.as_str() {
        "" => Some(show(settings, owner_id).await),
        "timezone" | "tz" => Some(set_timezone(settings, owner_id, value).await),
        "currency" | "cur" => Some(set_currency(settings, owner_id, value).await),
        "dev_room" | "dev" | "devroom" | "error_room" => {
            Some(set_dev_room(settings, owner_id, value).await)
        }
        "history" | "turns" | "max_history" => Some(set_history(settings, owner_id, value).await),
        "voice" | "speaker" => Some(set_voice(settings, owner_id, value).await),
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

async fn show(settings: &SettingsService, owner_id: &AccountId) -> String {
    let settings = settings.get_or_default(owner_id).await;
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

async fn set_history(settings: &SettingsService, owner_id: &AccountId, value: &str) -> String {
    if value.is_empty() {
        return format!("⚠️ How many history messages?<br/>{}", usage());
    }

    let turns: i64 = match value
        .parse()
        .ok()
        .and_then(|n| parse_max_history_turns(n).ok())
    {
        Some(n) => n,
        None => {
            return format!(
                "⚠️ Max history messages must be an integer between {} and {}.<br/>Example: <code>!settings history 10</code>",
                MIN_MAX_HISTORY_TURNS, MAX_MAX_HISTORY_TURNS
            );
        }
    };

    match modify(settings, owner_id, |current| {
        current.max_history_turns = turns
    })
    .await
    {
        Ok(saved) => format!(
            "✅ Max AI context history set to <b>{} messages</b>.",
            saved.max_history_turns
        ),
        Err(()) => "❌ Database error saving your setting.".to_string(),
    }
}

async fn set_voice(settings: &SettingsService, owner_id: &AccountId, value: &str) -> String {
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
                    html_escape(&reason.to_string())
                );
            }
        }
    };

    match modify(settings, owner_id, |current| current.tts_voice = voice).await {
        Ok(saved) => match saved.tts_voice {
            Some(voice) => format!("✅ Voice set to <b>{}</b>.", html_escape(&voice)),
            None => "✅ Voice cleared — using the server default.".to_string(),
        },
        Err(()) => "❌ Database error saving your voice.".to_string(),
    }
}

async fn set_timezone(settings: &SettingsService, owner_id: &AccountId, value: &str) -> String {
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

    let timezone = tz.name().to_string();
    match modify(settings, owner_id, |current| current.timezone = timezone).await {
        Ok(saved) => format!(
            "✅ Timezone set to <b>{}</b>. It is currently <b>{}</b>.",
            html_escape(&saved.timezone),
            html_escape(&format_datetime(&chrono::Utc::now(), &saved.tz()))
        ),
        Err(()) => "❌ Database error saving your timezone.".to_string(),
    }
}

async fn set_currency(settings: &SettingsService, owner_id: &AccountId, value: &str) -> String {
    if value.is_empty() {
        return format!("⚠️ Which currency?<br/>{}", usage());
    }

    let currency = match parse_currency(value) {
        Ok(code) => code,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Example: <code>!settings currency CNY</code>",
                html_escape(&err.to_string())
            )
        }
    };

    match modify(settings, owner_id, |current| current.currency = currency).await {
        Ok(saved) => format!(
            "✅ Default currency set to <b>{}</b>. \
             <code>!expense add 9.99 coffee</code> now books in {}.",
            html_escape(&saved.currency),
            html_escape(&saved.currency)
        ),
        Err(()) => "❌ Database error saving your currency.".to_string(),
    }
}

async fn set_dev_room(settings: &SettingsService, owner_id: &AccountId, value: &str) -> String {
    let dev_room_id = if value.is_empty()
        || value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("clear")
    {
        None
    } else {
        Some(value.to_string())
    };

    match modify(settings, owner_id, |current| {
        current.dev_room_id = dev_room_id
    })
    .await
    {
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

/// Applies one field change through `SettingsService::modify` — a read,
/// the mutation, and the write all in one transaction, so a concurrent
/// `!settings` command for the same owner can never interleave and lose one
/// side's change (see `SettingsService::modify`'s doc comment).
async fn modify(
    settings: &SettingsService,
    owner_id: &AccountId,
    f: impl FnOnce(&mut UserSettings),
) -> Result<UserSettings, ()> {
    settings
        .modify(owner_id, |current| {
            f(current);
            Ok(())
        })
        .await
        .map_err(|err| {
            tracing::error!(%owner_id, "Failed to save settings: {:?}", err);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::postgres::repositories::outbox;

    async fn service_and_pool() -> (SettingsService, sqlx::PgPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        (SettingsService::new(pool.clone()), pool)
    }

    async fn service() -> SettingsService {
        service_and_pool().await.0
    }

    async fn settings_events(pool: &sqlx::PgPool) -> usize {
        outbox::event_types_for(pool, &alice())
            .await
            .unwrap()
            .iter()
            .filter(|t| *t == "SettingsUpdated")
            .count()
    }

    fn alice() -> AccountId {
        AccountId::from_seed("@alice:a")
    }

    async fn run(settings: &SettingsService, body: &str) -> String {
        handle_settings_command(settings, &alice(), body)
            .await
            .expect("the handler must claim its own command")
    }

    #[tokio::test]
    async fn test_bare_command_shows_the_current_values() {
        let settings = service().await;
        let reply = run(&settings, "!settings").await;

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
        let settings = service().await;

        let tz_reply = run(&settings, "!settings timezone Asia/Shanghai").await;
        assert!(tz_reply.contains("Asia/Shanghai"), "{tz_reply}");

        // Setting one must not reset the other.
        let cur_reply = run(&settings, "!settings currency cny").await;
        assert!(
            cur_reply.contains("CNY"),
            "should normalise to upper case: {cur_reply}"
        );

        let shown = run(&settings, "!settings").await;
        assert!(
            shown.contains("Asia/Shanghai"),
            "the zone must survive: {shown}"
        );
        assert!(shown.contains("CNY"), "the currency must survive: {shown}");
    }

    #[tokio::test]
    async fn test_tz_alias_works() {
        let settings = service().await;
        let reply = run(&settings, "!settings tz Europe/Berlin").await;
        assert!(reply.contains("Europe/Berlin"), "{reply}");
    }

    /// A rejected value must leave the stored settings untouched, and say why.
    #[tokio::test]
    async fn test_rejects_bad_values_without_changing_anything() {
        let settings = service().await;
        run(&settings, "!settings timezone Asia/Shanghai").await;

        for bad in [
            "!settings timezone GMT+8",
            "!settings timezone Mars/Olympus",
        ] {
            let reply = run(&settings, bad).await;
            assert!(reply.contains("not an IANA"), "{bad} -> {reply}");
        }
        let reply = run(&settings, "!settings currency EURO").await;
        assert!(reply.contains("3-letter"), "{reply}");

        let shown = run(&settings, "!settings").await;
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
        let settings = service().await;

        assert!(run(&settings, "!settings timezone")
            .await
            .contains("Which timezone"));
        assert!(run(&settings, "!settings currency")
            .await
            .contains("Which currency"));
        assert!(run(&settings, "!settings colour blue")
            .await
            .contains("Unknown setting"));
    }

    /// User text reaches an HTML message, so it has to be escaped.
    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let settings = service().await;
        let reply = run(&settings, "!settings timezone <script>alert(1)</script>").await;

        assert!(
            !reply.contains("<script>"),
            "raw markup must not survive: {reply}"
        );
        assert!(reply.contains("&lt;script&gt;"));
    }

    /// An open web client formats every timestamp against these settings, so a
    /// change made over Matrix has to reach the SSE stream.
    #[tokio::test]
    async fn test_publishes_the_change_to_the_owner() {
        let (settings, pool) = service_and_pool().await;

        handle_settings_command(&settings, &alice(), "!settings tz Asia/Tokyo")
            .await
            .unwrap();

        assert_eq!(
            settings_events(&pool).await,
            1,
            "a settings change must be published"
        );
    }

    /// A rejected value changed nothing, so it must not announce a change either.
    #[tokio::test]
    async fn test_does_not_publish_when_nothing_was_saved() {
        let (settings, pool) = service_and_pool().await;

        handle_settings_command(&settings, &alice(), "!settings tz Mars/Olympus")
            .await
            .unwrap();
        handle_settings_command(&settings, &alice(), "!settings")
            .await
            .unwrap();

        assert_eq!(
            settings_events(&pool).await,
            0,
            "a read or a rejection must not publish"
        );
    }

    #[tokio::test]
    async fn test_sets_dev_room_and_clears_it() {
        let settings = service().await;

        let reply = run(&settings, "!settings dev_room !room123:mbm.one").await;
        assert!(reply.contains("!room123:mbm.one"), "{reply}");

        let shown = run(&settings, "!settings").await;
        assert!(shown.contains("!room123:mbm.one"), "{shown}");

        let clear_reply = run(&settings, "!settings dev none").await;
        assert!(clear_reply.contains("cleared"), "{clear_reply}");

        let shown2 = run(&settings, "!settings").await;
        assert!(shown2.contains("Not configured"), "{shown2}");
    }

    #[tokio::test]
    async fn test_sets_max_history_turns() {
        let settings = service().await;

        let reply = run(&settings, "!settings history 25").await;
        assert!(reply.contains("25 messages"), "{reply}");

        let shown = run(&settings, "!settings").await;
        assert!(shown.contains("25 messages"), "{shown}");

        // Rejects invalid values
        let bad = run(&settings, "!settings history 0").await;
        assert!(bad.contains("between 1 and 100"), "{bad}");

        let bad2 = run(&settings, "!settings history 150").await;
        assert!(bad2.contains("between 1 and 100"), "{bad2}");

        let bad3 = run(&settings, "!settings history abc").await;
        assert!(bad3.contains("between 1 and 100"), "{bad3}");

        // Previous setting remains intact
        let shown2 = run(&settings, "!settings").await;
        assert!(shown2.contains("25 messages"), "{shown2}");
    }

    #[tokio::test]
    async fn test_sets_voice_and_the_speaker_alias_works() {
        let settings = service().await;

        let reply = run(&settings, "!settings voice alloy").await;
        assert!(reply.contains("alloy"), "{reply}");
        assert_eq!(
            settings.get(&alice()).await.unwrap().tts_voice.as_deref(),
            Some("alloy")
        );

        let reply2 = run(&settings, "!settings speaker 21m00Tcm4TlvDq8ikWAM").await;
        assert!(reply2.contains("21m00Tcm4TlvDq8ikWAM"), "{reply2}");
        assert_eq!(
            settings.get(&alice()).await.unwrap().tts_voice.as_deref(),
            Some("21m00Tcm4TlvDq8ikWAM")
        );
    }

    /// Speaker 0 is a real, selectable voice — saving it must not be
    /// confused with clearing the preference.
    #[tokio::test]
    async fn test_sets_voice_to_zero_and_zero_survives() {
        let settings = service().await;

        let reply = run(&settings, "!settings voice 0").await;
        assert!(reply.contains('0'), "{reply}");

        let saved = settings.get(&alice()).await.unwrap();
        assert_eq!(saved.tts_voice.as_deref(), Some("0"));
    }

    #[tokio::test]
    async fn test_clearing_the_voice_resets_to_no_preference() {
        let settings = service().await;
        run(&settings, "!settings voice alloy").await;

        let reply = run(&settings, "!settings voice none").await;
        assert!(reply.contains("cleared"), "{reply}");

        let saved = settings.get(&alice()).await.unwrap();
        assert_eq!(saved.tts_voice, None);
    }

    #[tokio::test]
    async fn test_rejects_a_malformed_voice_without_changing_anything() {
        let settings = service().await;
        run(&settings, "!settings voice alloy").await;

        let too_long = "a".repeat(65);
        for bad in [
            "!settings voice a/b".to_string(),
            "!settings voice ../x".to_string(),
            format!("!settings voice {}", too_long),
        ] {
            let reply = run(&settings, &bad).await;
            assert!(reply.contains("⚠️"), "{bad} -> {reply}");
        }

        let saved = settings.get(&alice()).await.unwrap();
        assert_eq!(
            saved.tts_voice.as_deref(),
            Some("alloy"),
            "a rejected value must not clobber the saved voice"
        );
    }

    #[tokio::test]
    async fn test_voice_publishes_on_success_and_not_on_rejection() {
        let (settings, pool) = service_and_pool().await;

        handle_settings_command(&settings, &alice(), "!settings voice alloy")
            .await
            .unwrap();
        assert_eq!(
            settings_events(&pool).await,
            1,
            "a valid voice change must publish"
        );

        handle_settings_command(&settings, &alice(), "!settings voice a/b")
            .await
            .unwrap();
        assert_eq!(
            settings_events(&pool).await,
            1,
            "a rejected voice must not publish"
        );
    }

    /// Dispatch returns the first match, so a module that over-claims silently
    /// swallows another's traffic.
    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let settings = service().await;

        assert!(handle_settings_command(&settings, &alice(), "!settings")
            .await
            .is_some());
        assert!(
            handle_settings_command(&settings, &alice(), "!settings tz Asia/Tokyo")
                .await
                .is_some()
        );
        assert!(handle_settings_command(&settings, &alice(), "!todo list")
            .await
            .is_none());
        assert!(
            handle_settings_command(&settings, &alice(), "!expense list")
                .await
                .is_none()
        );
        assert!(handle_settings_command(&settings, &alice(), "hello there")
            .await
            .is_none());
    }

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@tester:x".to_string(),
            owner_id: alice(),
            sender_name: "alice".to_string(),
            command: body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('!')
                .to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        Database { pool }
    }

    #[test]
    fn test_advertises_its_commands() {
        let module = SettingsModule::new();
        assert_eq!(module.name(), "Settings");

        let commands = module.commands();
        assert!(commands.iter().any(|c| c.name == "settings timezone"));
        assert!(commands
            .iter()
            .all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    /// The `BotModule` trait impl itself dispatches to `handle_settings_command`
    /// via `services.settings` — exercised end to end, not just the free
    /// function, since `ModuleRegistry::dispatch` calls this method.
    #[tokio::test]
    async fn test_the_bot_module_claims_only_its_own_commands() {
        let module = SettingsModule::new();
        let db = db().await;

        assert!(module
            .handle_command(&ctx("!settings"), &AppServices::new(&db.pool), &db)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!todo list"), &AppServices::new(&db.pool), &db)
            .await
            .is_none());
    }

    /// The setting is only worth having if the other modules observe it.
    #[tokio::test]
    async fn test_a_change_here_is_visible_to_the_todo_module() {
        use crate::interface::chat::todos::TodoModule;

        let db = db().await;
        let settings = SettingsModule::new();
        let todo = TodoModule::new();

        todo.handle_command(&ctx("!todo add ship it"), &AppServices::new(&db.pool), &db)
            .await
            .unwrap();
        todo.handle_command(&ctx("!todo due 1 2d"), &AppServices::new(&db.pool), &db)
            .await
            .unwrap();

        settings
            .handle_command(
                &ctx("!settings timezone Asia/Tokyo"),
                &AppServices::new(&db.pool),
                &db,
            )
            .await
            .unwrap();

        let listed = todo
            .handle_command(&ctx("!todo list"), &AppServices::new(&db.pool), &db)
            .await
            .unwrap();
        assert!(
            listed.contains("JST"),
            "the todo list must follow the setting: {listed}"
        );
    }
}
