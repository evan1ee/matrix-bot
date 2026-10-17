//! Platform-neutral message types. Platform data stays inside the adapter,
//! reachable only through the opaque [`MessageRef`] it minted (R72): there is
//! deliberately no generic metadata map.

use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity};
use crate::domain::chat::QuotedMessage;
use crate::utils::strip_html;

/// Opaque, adapter-minted handle to a message on its platform.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageRef(pub String);

#[derive(Clone, Debug)]
pub struct IncomingMessage {
    pub channel: Channel,
    pub conversation: ConversationAddress,
    pub sender: PlatformIdentity,
    pub message_id: MessageRef,
    pub text: String,
    pub reply_to: Option<MessageRef>,
    pub quoted: Option<QuotedMessage>,
    pub is_direct: bool,
    pub is_mentioned: bool,
}

/// Whether a reply may be shown where others can read it. `Private` marks
/// replies carrying private data (portfolio, journal, settings, account).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Normal,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Audio,
    Image,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub mime: String,
    pub filename: String,
    // A Vec, not `bytes::Bytes`: `bytes` is not a direct dependency and
    // matrix-sdk uploads take a Vec (R76).
    pub bytes: Vec<u8>,
    pub duration_ms: Option<u64>,
    /// Loudness buckets of an audio clip, each in `0..=1024` (the MSC3246
    /// scale; Discord and Telegram voice notes draw the same bars). `None`
    /// when the synthesis provider sent none: a client then draws no bars.
    pub waveform: Option<Vec<u16>>,
}

/// A button: `command` is re-dispatched as if the clicker had typed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    pub plain: String,
    /// The rich format: the HTML subset modules emit.
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
    pub actions: Vec<Action>,
}

impl Content {
    /// `plain` is derived with `strip_html`.
    pub fn html(html: impl Into<String>) -> Self {
        let html = html.into();
        Self {
            plain: strip_html(&html),
            html: Some(html),
            ..Self::default()
        }
    }

    pub fn text(plain: impl Into<String>) -> Self {
        Self {
            plain: plain.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub content: Content,
    pub visibility: Visibility,
    pub is_ai: bool,
    pub owner_id: Option<AccountId>,
}

impl Reply {
    pub fn html(html: impl Into<String>) -> Self {
        Self {
            content: Content::html(html),
            visibility: Visibility::Normal,
            is_ai: false,
            owner_id: None,
        }
    }

    pub fn ai(html: impl Into<String>) -> Self {
        Self {
            content: Content::html(html),
            visibility: Visibility::Normal,
            is_ai: true,
            owner_id: None,
        }
    }

    pub fn mark_ai(mut self) -> Self {
        self.is_ai = true;
        self
    }

    pub fn with_owner(mut self, owner: AccountId) -> Self {
        self.owner_id = Some(owner);
        self
    }

    pub fn private(html: impl Into<String>) -> Self {
        Self {
            visibility: Visibility::Private,
            ..Self::html(html)
        }
    }

    pub fn with_attachment(mut self, attachment: Attachment) -> Self {
        self.content.attachments.push(attachment);
        self
    }

    pub fn with_actions(mut self, actions: Vec<Action>) -> Self {
        self.content.actions = actions;
        self
    }

    /// The HTML body, or the plain text when the reply has none.
    pub fn html_body(&self) -> &str {
        self.content.html.as_deref().unwrap_or(&self.content.plain)
    }
}

impl From<String> for Reply {
    fn from(html: String) -> Self {
        Self::html(html)
    }
}

#[derive(Clone, Debug)]
pub enum Target {
    Conversation(ConversationAddress),
    Direct(PlatformIdentity),
}

#[derive(Clone, Debug)]
pub struct OutgoingMessage {
    pub target: Target,
    pub content: Content,
    pub reply_to: Option<MessageRef>,
    pub visibility: Visibility,
    /// A key identifying this logical message across retries, for platforms
    /// that can deduplicate (Matrix: the transaction id). Others ignore it.
    pub idempotency_key: Option<String>,
    /// Whether the adapter may join a conversation it is not in to deliver
    /// (job sends do). Outbox deliveries set it false: an alert must never make
    /// the bot join a room, only reach one it already is in.
    pub join_if_needed: bool,
}

impl OutgoingMessage {
    pub fn to(target: Target, reply: Reply) -> Self {
        Self {
            target,
            content: reply.content,
            reply_to: None,
            visibility: reply.visibility,
            idempotency_key: None,
            join_if_needed: true,
        }
    }

    /// The channel the target lives on; `None` for Web.
    pub fn channel(&self) -> Option<Channel> {
        match &self.target {
            Target::Conversation(address) => address.channel(),
            Target::Direct(identity) => Some(identity.channel()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;

    #[test]
    fn reply_from_html_derives_its_plain_text() {
        let r = Reply::from("<b>Done</b> &amp; dusted".to_string());
        assert_eq!(r.content.plain, "Done & dusted");
        assert_eq!(r.visibility, Visibility::Normal);
        assert_eq!(Reply::private("<i>x</i>").visibility, Visibility::Private);
    }

    #[test]
    fn reply_ai_and_owner_provenance() {
        let owner = AccountId::generate();

        let r_html = Reply::html("<b>hello</b>");
        assert!(!r_html.is_ai);
        assert_eq!(r_html.owner_id, None);

        let r_ai = Reply::ai("<b>assistant</b>");
        assert!(r_ai.is_ai);
        assert_eq!(r_ai.owner_id, None);

        let r_marked = Reply::html("plain").mark_ai();
        assert!(r_marked.is_ai);
        assert_eq!(r_marked.owner_id, None);

        let r_owned = Reply::ai("<b>assistant</b>").with_owner(owner);
        assert!(r_owned.is_ai);
        assert_eq!(r_owned.owner_id, Some(owner));

        let r_from = Reply::from("plain text".to_string());
        assert!(!r_from.is_ai);
        assert_eq!(r_from.owner_id, None);

        let r_priv = Reply::private("secret");
        assert!(!r_priv.is_ai);
        assert_eq!(r_priv.owner_id, None);
    }
}

