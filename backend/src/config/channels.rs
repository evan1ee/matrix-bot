//! Per-channel settings and their environment parsing. Each channel plan adds
//! its own settings here, so `config/mod.rs` stays about the process as a whole.

use std::collections::HashMap;
use std::fmt;

use crate::domain::channels::{Channel, PlatformUserId};

/// The Discord adapter's settings; present only when `DISCORD_TOKEN` is set.
#[derive(Clone)]
pub struct DiscordConfig {
    /// `DISCORD_TOKEN`: the bot token. A credential; never logged.
    pub token: String,
    /// `DISCORD_GUILD_ID`: register the slash commands to this one guild,
    /// where updates are instant, instead of globally (up to an hour).
    pub guild_id: Option<u64>,
}

// Hand-written so the token can never reach a log through `{:?}` of `Config`.
impl fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("token", &"<redacted>")
            .field("guild_id", &self.guild_id)
            .finish()
    }
}

/// `None` when `DISCORD_TOKEN` is unset or blank. `Err` when `DISCORD_GUILD_ID`
/// is set but is not a non-zero integer: a typo there would silently register
/// the commands globally instead.
pub(super) fn parse_discord(
    get: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<DiscordConfig>> {
    let Some(token) = get("DISCORD_TOKEN")
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        return Ok(None);
    };
    let guild_id = match get("DISCORD_GUILD_ID").map(|g| g.trim().to_owned()) {
        None => None,
        Some(raw) if raw.is_empty() => None,
        Some(raw) => match raw.parse::<u64>() {
            Ok(id) if id != 0 => Some(id),
            _ => anyhow::bail!("DISCORD_GUILD_ID must be a Discord server id (got '{raw}')"),
        },
    };
    Ok(Some(DiscordConfig { token, guild_id }))
}

/// How the Telegram adapter receives updates.
#[derive(Clone)]
pub enum TelegramMode {
    /// Telegram POSTs updates to us, proving itself with this secret token.
    Webhook { secret: String },
    /// We long-poll `getUpdates`; needs no public URL.
    Polling,
}

/// The Telegram adapter's settings; present only when `TELEGRAM_BOT_TOKEN` is set.
#[derive(Clone)]
pub struct TelegramConfig {
    /// `TELEGRAM_BOT_TOKEN`: the bot token. A credential; never logged.
    pub token: String,
    pub mode: TelegramMode,
}

// Hand-written so neither the token nor the webhook secret can reach a log
// through `{:?}` of `Config`.
impl fmt::Debug for TelegramConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.mode {
            TelegramMode::Webhook { .. } => "webhook",
            TelegramMode::Polling => "polling",
        };
        let secret = match self.mode {
            TelegramMode::Webhook { .. } => "<redacted>",
            TelegramMode::Polling => "<none>",
        };
        f.debug_struct("TelegramConfig")
            .field("token", &"<redacted>")
            .field("mode", &mode)
            .field("webhook_secret", &secret)
            .finish()
    }
}

/// `None` when `TELEGRAM_BOT_TOKEN` is unset or blank. `Err` naming the variable
/// for an unknown `TELEGRAM_MODE` (`webhook`, the default, or `polling`) or for
/// webhook mode without a valid `TELEGRAM_WEBHOOK_SECRET`: Telegram accepts 1-256
/// characters of `A-Z a-z 0-9 _ -`, and a bot that silently accepts unsigned
/// webhooks would take updates from anyone.
pub(super) fn parse_telegram(
    get: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<TelegramConfig>> {
    let Some(token) = get("TELEGRAM_BOT_TOKEN")
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        return Ok(None);
    };
    let mode = get("TELEGRAM_MODE")
        .map(|m| m.trim().to_lowercase())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "webhook".to_owned());
    let mode = match mode.as_str() {
        "polling" => TelegramMode::Polling,
        "webhook" => {
            let secret = get("TELEGRAM_WEBHOOK_SECRET")
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            let valid = (1..=256).contains(&secret.len())
                && secret
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            if !valid {
                anyhow::bail!(
                    "TELEGRAM_WEBHOOK_SECRET is required in webhook mode and must be 1-256 \
                     characters of A-Z a-z 0-9 _ -"
                );
            }
            TelegramMode::Webhook { secret }
        }
        other => anyhow::bail!("TELEGRAM_MODE must be 'webhook' or 'polling' (got '{other}')"),
    };
    Ok(Some(TelegramConfig { token, mode }))
}

/// Webhook mode needs an `https://` `WEB_URL` (Telegram only calls HTTPS); an
/// allowlist entry that is not a Telegram user id is refused by name, since a
/// typo would silently keep that person from signing up (spec D4).
pub(super) fn validate_telegram(
    cfg: Option<&TelegramConfig>,
    web_url: &str,
    allowlist: &[String],
) -> anyhow::Result<()> {
    if matches!(cfg.map(|c| &c.mode), Some(TelegramMode::Webhook { .. }))
        && !web_url.starts_with("https://")
    {
        anyhow::bail!(
            "WEB_URL must be an https:// URL for Telegram webhook mode (got '{web_url}')"
        );
    }
    for entry in allowlist {
        if PlatformUserId::parse(Channel::Telegram, entry).is_err() {
            anyhow::bail!("TELEGRAM_ALLOWED_USERS entry '{entry}' is not a Telegram user id");
        }
    }
    Ok(())
}

/// A typo in `DISCORD_ALLOWED_USERS` would silently keep that person from
/// signing up (spec D4), so a malformed entry is refused by name.
pub(super) fn validate_discord_allowlist(allowlist: &[String]) -> anyhow::Result<()> {
    for entry in allowlist {
        if PlatformUserId::parse(Channel::Discord, entry).is_err() {
            anyhow::bail!("DISCORD_ALLOWED_USERS entry '{entry}' is not a Discord user id");
        }
    }
    Ok(())
}

/// A typo in `EMAIL_ALLOWED_SENDERS` would silently keep that person from
/// signing up (spec D4), so a malformed entry is refused by name.
pub(super) fn validate_email_allowlist(allowlist: &[String]) -> anyhow::Result<()> {
    for entry in allowlist {
        if PlatformUserId::parse(Channel::Email, entry).is_err() {
            anyhow::bail!("EMAIL_ALLOWED_SENDERS entry '{entry}' is not an email address");
        }
    }
    Ok(())
}

/// Default polling interval in seconds when `EMAIL_POLL_SECONDS` is unset.
pub const DEFAULT_EMAIL_POLL_SECONDS: u64 = 60;
/// Lower bound on `EMAIL_POLL_SECONDS` to prevent hammering IMAP servers.
pub const MIN_EMAIL_POLL_SECONDS: u64 = 5;

/// The Email adapter's settings; present only when `EMAIL_IMAP_URL` is set.
#[derive(Clone)]
pub struct EmailConfig {
    /// `EMAIL_IMAP_URL`: IMAPS endpoint for polling mail (e.g. `imaps://imap.example.com:993`).
    pub imap_url: url::Url,
    /// `EMAIL_SMTP_URL`: SMTP endpoint for sending mail (e.g. `smtps://smtp.example.com:465` or `smtp://smtp.example.com:587`).
    pub smtp_url: url::Url,
    /// `EMAIL_USERNAME`: account username for IMAP and SMTP auth. Credential; never logged.
    pub username: String,
    /// `EMAIL_PASSWORD`: account password for IMAP and SMTP auth. Credential; never logged.
    pub password: String,
    /// `EMAIL_FROM`: address the bot sends from and expects to be reached at.
    pub from: String,
    /// `EMAIL_AUTHSERV_ID`: authserv-id used by the receiving MTA in `Authentication-Results` (e.g. `mail.example.com`).
    pub authserv_id: String,
    /// `EMAIL_POLL_SECONDS`: polling interval in seconds (default 60, min 5).
    pub poll_seconds: u64,
    /// `EMAIL_PROCESSED_FOLDER`: optional IMAP folder to move processed messages to.
    pub processed_folder: Option<String>,
}

impl fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmailConfig")
            .field("imap_url", &self.imap_url.as_str())
            .field("smtp_url", &self.smtp_url.as_str())
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .field("from", &self.from)
            .field("authserv_id", &self.authserv_id)
            .field("poll_seconds", &self.poll_seconds)
            .field("processed_folder", &self.processed_folder)
            .finish()
    }
}

fn extract_addr_spec(raw: &str) -> &str {
    if let Some((_, rest)) = raw.split_once('<') {
        if let Some((addr, _)) = rest.split_once('>') {
            return addr.trim();
        }
    }
    raw.trim()
}

/// `None` when `EMAIL_IMAP_URL` is unset or blank. `Err` when required companion
/// settings are missing or malformed.
pub(super) fn parse_email(
    get: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<EmailConfig>> {
    let Some(imap_raw) = get("EMAIL_IMAP_URL")
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
    else {
        return Ok(None);
    };

    let imap_url = url::Url::parse(&imap_raw).map_err(|e| {
        anyhow::anyhow!("EMAIL_IMAP_URL must be a valid URL (got '{imap_raw}'): {e}")
    })?;

    let smtp_raw = get("EMAIL_SMTP_URL")
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| anyhow::anyhow!("EMAIL_SMTP_URL is required when EMAIL_IMAP_URL is set"))?;
    let smtp_url = url::Url::parse(&smtp_raw).map_err(|e| {
        anyhow::anyhow!("EMAIL_SMTP_URL must be a valid URL (got '{smtp_raw}'): {e}")
    })?;

    let username = get("EMAIL_USERNAME")
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| anyhow::anyhow!("EMAIL_USERNAME is required when EMAIL_IMAP_URL is set"))?;

    let password = get("EMAIL_PASSWORD")
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| anyhow::anyhow!("EMAIL_PASSWORD is required when EMAIL_IMAP_URL is set"))?;

    let from = get("EMAIL_FROM")
        .map(|f| f.trim().to_owned())
        .filter(|f| !f.is_empty())
        .ok_or_else(|| anyhow::anyhow!("EMAIL_FROM is required when EMAIL_IMAP_URL is set"))?;

    let authserv_id = get("EMAIL_AUTHSERV_ID")
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("EMAIL_AUTHSERV_ID is required when EMAIL_IMAP_URL is set")
        })?;

    let poll_seconds = match get("EMAIL_POLL_SECONDS").map(|s| s.trim().to_owned()) {
        None => DEFAULT_EMAIL_POLL_SECONDS,
        Some(raw) if raw.is_empty() => DEFAULT_EMAIL_POLL_SECONDS,
        Some(raw) => raw.parse::<u64>().map_err(|_| {
            anyhow::anyhow!("EMAIL_POLL_SECONDS must be a positive number of seconds (got '{raw}')")
        })?,
    };

    let processed_folder = get("EMAIL_PROCESSED_FOLDER")
        .map(|f| f.trim().to_owned())
        .filter(|f| !f.is_empty());

    Ok(Some(EmailConfig {
        imap_url,
        smtp_url,
        username,
        password,
        from,
        authserv_id,
        poll_seconds,
        processed_folder,
    }))
}

pub(super) fn validate_email(cfg: &EmailConfig) -> anyhow::Result<()> {
    if cfg.imap_url.scheme() != "imaps" {
        anyhow::bail!(
            "EMAIL_IMAP_URL must use the imaps:// scheme for TLS (got '{}')",
            cfg.imap_url.scheme()
        );
    }
    if cfg.imap_url.host_str().is_none() {
        anyhow::bail!("EMAIL_IMAP_URL must specify a host");
    }
    let smtp_scheme = cfg.smtp_url.scheme();
    if smtp_scheme != "smtps" && smtp_scheme != "smtp" {
        anyhow::bail!("EMAIL_SMTP_URL must use smtp:// or smtps:// scheme (got '{smtp_scheme}')");
    }
    if cfg.smtp_url.host_str().is_none() {
        anyhow::bail!("EMAIL_SMTP_URL must specify a host");
    }
    let addr = extract_addr_spec(&cfg.from);
    if PlatformUserId::parse(Channel::Email, addr).is_err() {
        anyhow::bail!(
            "EMAIL_FROM must be a valid email address (got '{}')",
            cfg.from
        );
    }
    if cfg.authserv_id.trim().is_empty() {
        anyhow::bail!("EMAIL_AUTHSERV_ID must not be empty");
    }
    if cfg.poll_seconds < MIN_EMAIL_POLL_SECONDS {
        anyhow::bail!(
            "EMAIL_POLL_SECONDS must be at least {MIN_EMAIL_POLL_SECONDS} seconds (got {})",
            cfg.poll_seconds
        );
    }
    Ok(())
}

/// The signup allowlist, one environment variable per channel, each a
/// comma-separated list. Entries are trimmed, empty ones dropped, email
/// addresses lowercased (the canonical form identities are stored in); a
/// channel with no entries is left out of the map.
pub(super) fn parse_link_signup_allowlist(
    get: impl Fn(&str) -> Option<String>,
) -> HashMap<Channel, Vec<String>> {
    [
        (Channel::Discord, "DISCORD_ALLOWED_USERS"),
        (Channel::Telegram, "TELEGRAM_ALLOWED_USERS"),
        (Channel::Email, "EMAIL_ALLOWED_SENDERS"),
    ]
    .into_iter()
    .filter_map(|(channel, name)| {
        let entries: Vec<String> = get(name)?
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| match channel {
                Channel::Email => entry.to_lowercase(),
                _ => entry.to_owned(),
            })
            .collect();
        (!entries.is_empty()).then_some((channel, entries))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_signup_allowlist_reads_one_env_name_per_channel() {
        use crate::domain::channels::Channel;
        let env = |name: &str| match name {
            "DISCORD_ALLOWED_USERS" => Some(" 111 , 222,, ".to_string()),
            "TELEGRAM_ALLOWED_USERS" => Some("42".to_string()),
            "EMAIL_ALLOWED_SENDERS" => Some("Evan@Example.com, ,b@x.org".to_string()),
            _ => None,
        };
        let list = parse_link_signup_allowlist(env);
        assert_eq!(list[&Channel::Discord], vec!["111", "222"]);
        assert_eq!(list[&Channel::Telegram], vec!["42"]);
        assert_eq!(
            list[&Channel::Email],
            vec!["evan@example.com", "b@x.org"],
            "email entries are lowercased"
        );
        assert!(!list.contains_key(&Channel::Matrix));
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn discord_is_absent_without_a_token() {
        assert!(parse_discord(env_of(&[])).unwrap().is_none());
        assert!(parse_discord(env_of(&[("DISCORD_TOKEN", "  ")]))
            .unwrap()
            .is_none());
    }

    #[test]
    fn discord_reads_token_and_optional_guild() {
        let cfg = parse_discord(env_of(&[("DISCORD_TOKEN", " abc ")]))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.token, "abc");
        assert_eq!(cfg.guild_id, None);
        let cfg = parse_discord(env_of(&[
            ("DISCORD_TOKEN", "abc"),
            ("DISCORD_GUILD_ID", "123"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(cfg.guild_id, Some(123));
    }

    #[test]
    fn a_malformed_guild_id_is_refused() {
        for bad in ["0", "abc", "-1"] {
            let err = parse_discord(env_of(&[
                ("DISCORD_TOKEN", "abc"),
                ("DISCORD_GUILD_ID", bad),
            ]))
            .unwrap_err();
            assert!(err.to_string().contains("DISCORD_GUILD_ID"), "{err}");
        }
    }

    #[test]
    fn the_discord_token_never_appears_in_debug_output() {
        let cfg = DiscordConfig {
            token: "very-secret-token".into(),
            guild_id: Some(1),
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("very-secret-token"), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn the_link_signup_allowlist_is_empty_when_unset_or_blank() {
        assert!(parse_link_signup_allowlist(|_| None).is_empty());
        assert!(parse_link_signup_allowlist(|_| Some(" , ,".to_string())).is_empty());
    }

    fn tg(pairs: &[(&str, &str)]) -> anyhow::Result<Option<TelegramConfig>> {
        parse_telegram(env_of(pairs))
    }

    #[test]
    fn telegram_is_absent_without_a_token() {
        assert!(tg(&[]).unwrap().is_none());
        assert!(tg(&[("TELEGRAM_BOT_TOKEN", "  ")]).unwrap().is_none());
    }

    #[test]
    fn telegram_defaults_to_webhook_mode_and_needs_a_secret() {
        let cfg = tg(&[
            ("TELEGRAM_BOT_TOKEN", " 123:abc "),
            ("TELEGRAM_WEBHOOK_SECRET", "s3cret_-X"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(cfg.token, "123:abc");
        assert!(matches!(cfg.mode, TelegramMode::Webhook { ref secret } if secret == "s3cret_-X"));
    }

    #[test]
    fn telegram_polling_needs_no_secret() {
        let cfg = tg(&[
            ("TELEGRAM_BOT_TOKEN", "123:abc"),
            ("TELEGRAM_MODE", " Polling "),
        ])
        .unwrap()
        .unwrap();
        assert!(matches!(cfg.mode, TelegramMode::Polling));
    }

    #[test]
    fn telegram_webhook_refuses_a_missing_or_invalid_secret() {
        let long = "a".repeat(257);
        for secret in [None, Some(""), Some("bad secret!"), Some(long.as_str())] {
            let mut pairs = vec![("TELEGRAM_BOT_TOKEN", "123:abc")];
            if let Some(s) = secret {
                pairs.push(("TELEGRAM_WEBHOOK_SECRET", s));
            }
            let err = tg(&pairs).unwrap_err();
            assert!(err.to_string().contains("TELEGRAM_WEBHOOK_SECRET"), "{err}");
        }
        let max = "a".repeat(256);
        assert!(tg(&[
            ("TELEGRAM_BOT_TOKEN", "123:abc"),
            ("TELEGRAM_WEBHOOK_SECRET", max.as_str()),
        ])
        .is_ok());
    }

    #[test]
    fn telegram_refuses_an_unknown_mode() {
        let err = tg(&[("TELEGRAM_BOT_TOKEN", "123:abc"), ("TELEGRAM_MODE", "push")]).unwrap_err();
        assert!(err.to_string().contains("TELEGRAM_MODE"), "{err}");
    }

    #[test]
    fn telegram_token_and_secret_never_appear_in_debug_output() {
        let cfg = TelegramConfig {
            token: "very-secret-token".into(),
            mode: TelegramMode::Webhook {
                secret: "very-secret-hook".into(),
            },
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("very-secret"), "{shown}");
        assert!(
            shown.contains("redacted") && shown.contains("webhook"),
            "{shown}"
        );
    }

    #[test]
    fn validate_telegram_needs_https_only_in_webhook_mode() {
        let webhook = TelegramConfig {
            token: "t".into(),
            mode: TelegramMode::Webhook { secret: "s".into() },
        };
        let polling = TelegramConfig {
            token: "t".into(),
            mode: TelegramMode::Polling,
        };
        assert!(validate_telegram(Some(&webhook), "https://bot.example.com", &[]).is_ok());
        let err = validate_telegram(Some(&webhook), "http://bot.example.com", &[]).unwrap_err();
        assert!(err.to_string().contains("WEB_URL"), "{err}");
        assert!(validate_telegram(Some(&polling), "http://localhost:8080", &[]).is_ok());
        assert!(validate_telegram(None, "http://localhost:8080", &[]).is_ok());
    }

    #[test]
    fn validate_telegram_refuses_a_malformed_allowlist_entry_by_name() {
        let ok = vec!["42".to_string(), "987654321".to_string()];
        assert!(validate_telegram(None, "http://x", &ok).is_ok());
        let bad = vec!["42".to_string(), "abc".to_string()];
        let err = validate_telegram(None, "http://x", &bad).unwrap_err();
        let shown = err.to_string();
        assert!(
            shown.contains("abc") && shown.contains("TELEGRAM_ALLOWED_USERS"),
            "{shown}"
        );
    }

    #[test]
    fn email_is_absent_without_imap_url() {
        assert!(parse_email(env_of(&[])).unwrap().is_none());
        assert!(parse_email(env_of(&[("EMAIL_IMAP_URL", "  ")]))
            .unwrap()
            .is_none());
    }

    #[test]
    fn email_reads_all_fields_with_defaults() {
        let env = env_of(&[
            ("EMAIL_IMAP_URL", "imaps://imap.example.com:993"),
            ("EMAIL_SMTP_URL", "smtp://smtp.example.com:587"),
            ("EMAIL_USERNAME", "bot@example.com"),
            ("EMAIL_PASSWORD", "secret123"),
            ("EMAIL_FROM", "bot@example.com"),
            ("EMAIL_AUTHSERV_ID", "mail.example.com"),
        ]);
        let cfg = parse_email(env).unwrap().unwrap();
        assert_eq!(cfg.imap_url.as_str(), "imaps://imap.example.com:993");
        assert_eq!(cfg.smtp_url.as_str(), "smtp://smtp.example.com:587");
        assert_eq!(cfg.username, "bot@example.com");
        assert_eq!(cfg.password, "secret123");
        assert_eq!(cfg.from, "bot@example.com");
        assert_eq!(cfg.authserv_id, "mail.example.com");
        assert_eq!(cfg.poll_seconds, DEFAULT_EMAIL_POLL_SECONDS);
        assert_eq!(cfg.processed_folder, None);
        assert!(validate_email(&cfg).is_ok());
    }

    #[test]
    fn email_credentials_never_appear_in_debug_output() {
        let cfg = EmailConfig {
            imap_url: url::Url::parse("imaps://imap.example.com").unwrap(),
            smtp_url: url::Url::parse("smtps://smtp.example.com").unwrap(),
            username: "secret-user".into(),
            password: "super-secret-password".into(),
            from: "bot@example.com".into(),
            authserv_id: "mail.example.com".into(),
            poll_seconds: 60,
            processed_folder: Some("Archive".into()),
        };
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("secret-user"), "{shown}");
        assert!(!shown.contains("super-secret-password"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }

    #[test]
    fn validate_email_enforces_imaps_and_smtp_schemes() {
        let mut cfg = EmailConfig {
            imap_url: url::Url::parse("imap://imap.example.com").unwrap(),
            smtp_url: url::Url::parse("smtp://smtp.example.com").unwrap(),
            username: "u".into(),
            password: "p".into(),
            from: "bot@example.com".into(),
            authserv_id: "mail.example.com".into(),
            poll_seconds: 60,
            processed_folder: None,
        };
        let err = validate_email(&cfg).unwrap_err();
        assert!(err.to_string().contains("imaps://"), "{err}");

        cfg.imap_url = url::Url::parse("imaps://imap.example.com").unwrap();
        cfg.smtp_url = url::Url::parse("http://smtp.example.com").unwrap();
        let err = validate_email(&cfg).unwrap_err();
        assert!(err.to_string().contains("EMAIL_SMTP_URL"), "{err}");
    }

    #[test]
    fn validate_email_allowlist_refuses_invalid_addresses() {
        let ok = vec!["alice@example.com".to_string(), "bob@mbm.one".to_string()];
        assert!(validate_email_allowlist(&ok).is_ok());
        let bad = vec!["alice@example.com".to_string(), "not-an-email".to_string()];
        let err = validate_email_allowlist(&bad).unwrap_err();
        assert!(err.to_string().contains("not-an-email"), "{err}");
    }
}
