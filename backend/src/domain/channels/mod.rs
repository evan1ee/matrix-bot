//! Channels: the platforms a person can talk to the bot on, the identities
//! they hold there, and the addresses of conversations on them.
//!
//! A conversation address has two encodings, and the difference is deliberate:
//!
//! - **Storage form** is what the existing `room_id` columns hold (todos,
//!   reminders, transactions, AI conversations, ...): `web`, a *bare* Matrix
//!   room id (`!abc:server`), or `discord:<id>` / `telegram:<id>` /
//!   `email:<addr>`. A Matrix room id already carries its own `!` sigil, so
//!   every row written before other channels existed is valid storage form and
//!   no data-rewriting migration is needed (R71). The `matrix:` prefix is
//!   therefore *not* accepted here: one canonical spelling per address.
//! - **Destination form** is what outbox deliveries carry: always
//!   `<channel>:<id>`, including `matrix:!abc:server`, so a delivery sink can
//!   route on the prefix alone. `Web` has no destination: nothing is delivered
//!   outside the web UI.
//!
//! Everything here is pure and synchronous; adapters validate raw platform
//! input through these constructors before it reaches a service.

pub mod audio;
mod link;
pub use link::{decide_link, LinkDecision, LINK_TOKEN_TTL};

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The storage-form spelling of "no external channel" (see [`WEB_ROOM`](crate::domain::rooms::WEB_ROOM)).
const WEB: &str = "web";

/// Longest Matrix user id the spec allows.
const MATRIX_USER_ID_MAX: usize = 255;
/// Longest email address (RFC 5321 path limit).
const EMAIL_MAX: usize = 254;

/// A platform the bot can be reached on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Matrix,
    Discord,
    Telegram,
    Email,
}

impl Channel {
    /// Every platform variant; keep exhaustive when adding a channel.
    pub const ALL: [Self; 4] = [Self::Matrix, Self::Discord, Self::Telegram, Self::Email];

    /// The name shown to people ("Telegram"), as opposed to the storage key.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Matrix => "Matrix",
            Self::Discord => "Discord",
            Self::Telegram => "Telegram",
            Self::Email => "Email",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Matrix => "matrix",
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::Email => "email",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Channel {
    type Err = ChannelsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "matrix" => Ok(Self::Matrix),
            "discord" => Ok(Self::Discord),
            "telegram" => Ok(Self::Telegram),
            "email" => Ok(Self::Email),
            other => Err(ChannelsError::InvalidAddress(format!(
                "unknown channel `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelsError {
    #[error("invalid {channel} user id: {reason}")]
    InvalidUserId {
        channel: Channel,
        reason: &'static str,
    },
    #[error("invalid conversation address: {0}")]
    InvalidAddress(String),
}

/// A validated, canonical user id on one platform (an MXID, a Discord
/// snowflake, a Telegram user id or a lowercased email address).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlatformUserId(String);

impl PlatformUserId {
    pub fn parse(channel: Channel, raw: &str) -> Result<Self, ChannelsError> {
        let invalid = |reason| ChannelsError::InvalidUserId { channel, reason };
        let canonical = match channel {
            Channel::Matrix => {
                if !raw.starts_with('@') || !raw.contains(':') {
                    return Err(invalid("expected @user:server"));
                }
                if raw.len() > MATRIX_USER_ID_MAX {
                    return Err(invalid("longer than 255 bytes"));
                }
                raw.to_owned()
            }
            Channel::Discord => parse_discord_id(raw)
                .ok_or_else(|| invalid("expected a non-zero numeric id"))?
                .to_string(),
            Channel::Telegram => match raw.parse::<i64>() {
                Ok(id) if id > 0 => id.to_string(),
                _ => return Err(invalid("expected a positive numeric id")),
            },
            Channel::Email => {
                normalize_email(raw).ok_or_else(|| invalid("expected an email address"))?
            }
        };
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who someone is on one platform, plus the name that platform shows for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformIdentity {
    channel: Channel,
    user_id: PlatformUserId,
    display_name: Option<String>,
}

impl PlatformIdentity {
    pub fn new(channel: Channel, user_id: PlatformUserId, display_name: Option<String>) -> Self {
        Self {
            channel,
            user_id,
            display_name,
        }
    }

    /// A Matrix identity without a display name.
    pub fn matrix(mxid: &str) -> Result<Self, ChannelsError> {
        Ok(Self::new(
            Channel::Matrix,
            PlatformUserId::parse(Channel::Matrix, mxid)?,
            None,
        ))
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    pub fn user_id(&self) -> &PlatformUserId {
        &self.user_id
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }
}

/// An identity bound to an account, as listed back to its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedIdentity {
    pub channel: Channel,
    pub user_id: String,
    pub display_name: Option<String>,
    pub linked_at: DateTime<Utc>,
}

impl LinkedIdentity {
    pub fn new(
        channel: Channel,
        user_id: String,
        display_name: Option<String>,
        linked_at: DateTime<Utc>,
    ) -> Self {
        Self {
            channel,
            user_id,
            display_name,
            linked_at,
        }
    }
}

/// Where a conversation lives. See the module docs for the two encodings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConversationAddress {
    Web,
    Matrix(String),
    Discord(u64),
    Telegram(i64),
    Email(String),
}

impl ConversationAddress {
    /// `None` for `Web`: it is not a platform the bot delivers to.
    pub fn channel(&self) -> Option<Channel> {
        match self {
            Self::Web => None,
            Self::Matrix(_) => Some(Channel::Matrix),
            Self::Discord(_) => Some(Channel::Discord),
            Self::Telegram(_) => Some(Channel::Telegram),
            Self::Email(_) => Some(Channel::Email),
        }
    }

    /// Parses the form held in `room_id` columns: `web`, a bare `!room:server`,
    /// or `<channel>:<id>` for the other channels. `matrix:` is refused.
    pub fn parse_storage(raw: &str) -> Result<Self, ChannelsError> {
        let bad = || ChannelsError::InvalidAddress(raw.to_owned());
        if raw == WEB {
            return Ok(Self::Web);
        }
        if raw.starts_with('!') {
            return parse_matrix_room(raw).ok_or_else(bad);
        }
        let (prefix, id) = raw.split_once(':').ok_or_else(bad)?;
        match prefix.parse::<Channel>() {
            Ok(Channel::Matrix) | Err(_) => Err(bad()),
            Ok(channel) => Self::parse_id(channel, id).ok_or_else(bad),
        }
    }

    pub fn to_storage(&self) -> String {
        match self {
            Self::Web => WEB.to_owned(),
            Self::Matrix(room) => room.clone(),
            other => other.prefixed(),
        }
    }

    /// Parses the outbox form `<channel>:<id>` (Matrix included).
    pub fn parse_destination(raw: &str) -> Option<Self> {
        let (prefix, id) = raw.split_once(':')?;
        Self::parse_id(prefix.parse().ok()?, id)
    }

    pub fn to_destination(&self) -> Option<String> {
        match self {
            Self::Web => None,
            other => Some(other.prefixed()),
        }
    }

    fn parse_id(channel: Channel, id: &str) -> Option<Self> {
        match channel {
            Channel::Matrix => parse_matrix_room(id),
            Channel::Discord => parse_discord_id(id).map(Self::Discord),
            Channel::Telegram => id
                .parse::<i64>()
                .ok()
                .filter(|chat| *chat != 0)
                .map(Self::Telegram),
            Channel::Email => normalize_email(id).map(Self::Email),
        }
    }

    /// `<channel>:<id>` for every non-web address.
    fn prefixed(&self) -> String {
        match self {
            Self::Web => WEB.to_owned(),
            Self::Matrix(room) => format!("matrix:{room}"),
            Self::Discord(id) => format!("discord:{id}"),
            Self::Telegram(id) => format!("telegram:{id}"),
            Self::Email(addr) => format!("email:{addr}"),
        }
    }
}

/// What an adapter does with a sender nobody has linked yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisioningPolicy {
    /// Create an account on first contact.
    ProvisionUnknown,
    /// Refuse until the sender links the identity to an existing account.
    RequireLink,
}

/// Matrix keeps its historic behaviour (the bot auto-provisions an account for
/// whoever DMs it); every newer channel must be linked from an existing,
/// passkey-verified account so a stranger cannot mint accounts.
pub fn provisioning_policy(channel: Channel) -> ProvisioningPolicy {
    match channel {
        Channel::Matrix => ProvisioningPolicy::ProvisionUnknown,
        Channel::Discord | Channel::Telegram | Channel::Email => ProvisioningPolicy::RequireLink,
    }
}

fn parse_matrix_room(raw: &str) -> Option<ConversationAddress> {
    (raw.starts_with('!') && raw.contains(':')).then(|| ConversationAddress::Matrix(raw.to_owned()))
}

fn parse_discord_id(raw: &str) -> Option<u64> {
    raw.parse::<u64>().ok().filter(|id| *id != 0)
}

/// Trims and lowercases; accepts exactly one `@`, a non-empty local part and a
/// dotted domain. Deliberately not a full RFC 5322 parser.
fn normalize_email(raw: &str) -> Option<String> {
    let addr = raw.trim().to_lowercase();
    let (local, domain) = addr.split_once('@')?;
    let valid = addr.len() <= EMAIL_MAX
        && !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.');
    valid.then_some(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_form_round_trips_and_keeps_bare_matrix_ids() {
        for raw in [
            "web",
            "!abc:mbm.one",
            "discord:123456789012345678",
            "telegram:-1001234567890",
            "email:evan@example.com",
        ] {
            let addr = ConversationAddress::parse_storage(raw).unwrap();
            assert_eq!(addr.to_storage(), raw);
        }
        assert_eq!(
            ConversationAddress::parse_storage("!abc:mbm.one").unwrap(),
            ConversationAddress::Matrix("!abc:mbm.one".into())
        );
    }

    #[test]
    fn destination_form_always_carries_the_channel_prefix() {
        let m = ConversationAddress::Matrix("!abc:mbm.one".into());
        assert_eq!(m.to_destination().as_deref(), Some("matrix:!abc:mbm.one"));
        assert_eq!(
            ConversationAddress::parse_destination("matrix:!abc:mbm.one"),
            Some(m)
        );
        assert_eq!(ConversationAddress::Web.to_destination(), None);
        assert_eq!(
            ConversationAddress::parse_destination("telegram:42"),
            Some(ConversationAddress::Telegram(42))
        );
    }

    #[test]
    fn malformed_addresses_are_rejected() {
        for raw in [
            "",
            "matrix:",
            "discord:abc",
            "telegram:",
            "email:not-an-address",
            "slack:1",
            "abc",
        ] {
            assert!(
                ConversationAddress::parse_storage(raw).is_err(),
                "{raw} accepted"
            );
        }
    }

    #[test]
    fn user_ids_are_validated_per_channel() {
        assert!(PlatformUserId::parse(Channel::Matrix, "@evan:mbm.one").is_ok());
        assert!(PlatformUserId::parse(Channel::Matrix, "evan").is_err());
        assert!(PlatformUserId::parse(Channel::Discord, "123456789012345678").is_ok());
        assert!(PlatformUserId::parse(Channel::Discord, "-1").is_err());
        assert!(PlatformUserId::parse(Channel::Telegram, "987654321").is_ok());
        assert!(PlatformUserId::parse(Channel::Telegram, "x").is_err());
        assert_eq!(
            PlatformUserId::parse(Channel::Email, " Evan@Example.COM ")
                .unwrap()
                .as_str(),
            "evan@example.com"
        );
        assert!(PlatformUserId::parse(Channel::Email, "evan@").is_err());
        assert!(
            PlatformUserId::parse(Channel::Email, &format!("{}@x.io", "a".repeat(250))).is_err()
        );
    }

    #[test]
    fn only_matrix_provisions_unknown_identities() {
        assert_eq!(
            provisioning_policy(Channel::Matrix),
            ProvisioningPolicy::ProvisionUnknown
        );
        for c in [Channel::Discord, Channel::Telegram, Channel::Email] {
            assert_eq!(provisioning_policy(c), ProvisioningPolicy::RequireLink);
        }
    }

    #[test]
    fn channel_names_round_trip() {
        for c in [
            Channel::Matrix,
            Channel::Discord,
            Channel::Telegram,
            Channel::Email,
        ] {
            assert_eq!(c.as_str().parse::<Channel>().unwrap(), c);
        }
    }

    #[test]
    fn conversation_address_serde_round_trips() {
        let cases = [
            ConversationAddress::Web,
            ConversationAddress::Matrix("!room:server.org".into()),
            ConversationAddress::Discord(123456789),
            ConversationAddress::Telegram(987654),
            ConversationAddress::Email("alice@example.com".into()),
        ];
        for addr in cases {
            let json = serde_json::to_string(&addr).unwrap();
            let back: ConversationAddress = serde_json::from_str(&json).unwrap();
            assert_eq!(back, addr);
        }
    }
}
