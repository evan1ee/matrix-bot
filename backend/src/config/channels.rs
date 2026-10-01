//! Per-channel settings and their environment parsing. Each channel plan adds
//! its own settings here, so `config/mod.rs` stays about the process as a whole.

use std::collections::HashMap;

use crate::domain::channels::Channel;

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

    #[test]
    fn the_link_signup_allowlist_is_empty_when_unset_or_blank() {
        assert!(parse_link_signup_allowlist(|_| None).is_empty());
        assert!(parse_link_signup_allowlist(|_| Some(" , ,".to_string())).is_empty());
    }
}
