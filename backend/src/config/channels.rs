//! Per-channel settings and their environment parsing. Each channel plan adds
//! its own settings here, so `config/mod.rs` stays about the process as a whole.

use std::collections::HashMap;
use std::fmt;

use crate::domain::channels::Channel;

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
}
