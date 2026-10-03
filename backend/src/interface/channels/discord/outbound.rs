//! `ChannelAdapter::send` for Discord, and the serenity builders every post
//! goes through. serenity errors become `ChannelError` here and nowhere else.

use std::future::Future;
use std::time::Duration;

use serenity::all::{
    ChannelId, CreateAllowedMentions, CreateAttachment, CreateEmbed,
    CreateInteractionResponseFollowup, CreateMessage, EditInteractionResponse, Http, MessageId,
    Nonce, UserId,
};

use super::inbound::{message_ref, parse_message_ref};
use super::render::{posts, Post};
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::message::{Attachment, OutgoingMessage, Target};

/// Every Discord HTTP call is bounded by this.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// Discord's nonce limit (B16).
const NONCE_MAX: usize = 25;

/// Posts `message`: `Conversation(Discord(id))` to that channel, `Direct` on
/// Discord to the user's DM channel (opened or reused); anything else is
/// `Rejected` before any HTTP call. Returns the last posted message's ref.
pub async fn send(http: &Http, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
    // Shape errors first, so a message that can never go out costs no HTTP call.
    let posts = posts(&message.content)?;
    let channel = match &message.target {
        Target::Conversation(ConversationAddress::Discord(id)) => ChannelId::new(*id),
        Target::Direct(identity) if identity.channel() == Channel::Discord => {
            let user: u64 = identity.user_id().as_str().parse().map_err(|_| {
                ChannelError::Rejected(format!(
                    "not a Discord user id: {}",
                    identity.user_id().as_str()
                ))
            })?;
            let dm = bounded(UserId::new(user).create_dm_channel(http)).await?;
            dm.id
        }
        other => {
            return Err(ChannelError::Rejected(format!(
                "not a Discord target: {other:?}"
            )))
        }
    };
    let reference = message
        .reply_to
        .as_ref()
        .and_then(parse_message_ref)
        .filter(|(c, _)| *c == channel.get())
        .map(|(c, m)| (ChannelId::new(c), MessageId::new(m)));
    let nonce = message
        .idempotency_key
        .as_ref()
        .filter(|key| key.len() <= NONCE_MAX)
        .cloned();

    let mut last = None;
    for (i, post) in posts.into_iter().enumerate() {
        let mut builder = create_message(post);
        if i == 0 {
            if let Some(reference) = reference {
                builder = builder.reference_message(reference);
            }
            if let Some(key) = &nonce {
                builder = builder
                    .nonce(Nonce::String(key.clone()))
                    .enforce_nonce(true);
            }
        }
        let sent = bounded(channel.send_message(http, builder)).await?;
        last = Some(message_ref(channel.get(), sent.id.get()));
    }
    Ok(SentRef { message: last })
}

/// No post may ping anyone: user text can contain `@everyone` or `<@id>`.
fn no_mentions() -> CreateAllowedMentions {
    CreateAllowedMentions::new()
}

fn embed(description: Option<String>) -> Option<CreateEmbed> {
    description.map(|d| CreateEmbed::new().description(d))
}

fn attachment(file: Attachment) -> CreateAttachment {
    CreateAttachment::bytes(file.bytes, file.filename)
}

pub fn create_message(post: Post) -> CreateMessage {
    let mut builder = CreateMessage::new().allowed_mentions(no_mentions());
    if let Some(e) = embed(post.description) {
        builder = builder.embed(e);
    }
    builder.add_files(post.files.into_iter().map(attachment))
}

pub fn edit_response(post: Post) -> EditInteractionResponse {
    let mut builder = EditInteractionResponse::new().allowed_mentions(no_mentions());
    if let Some(e) = embed(post.description) {
        builder = builder.embed(e);
    }
    for file in post.files {
        builder = builder.new_attachment(attachment(file));
    }
    builder
}

pub fn follow_up(post: Post, ephemeral: bool) -> CreateInteractionResponseFollowup {
    let mut builder = CreateInteractionResponseFollowup::new()
        .allowed_mentions(no_mentions())
        .ephemeral(ephemeral);
    if let Some(e) = embed(post.description) {
        builder = builder.embed(e);
    }
    builder.add_files(post.files.into_iter().map(attachment))
}

/// Runs one serenity call under [`SEND_TIMEOUT`], mapping its error.
pub async fn bounded<T>(
    call: impl Future<Output = serenity::Result<T>>,
) -> Result<T, ChannelError> {
    match tokio::time::timeout(SEND_TIMEOUT, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(classify(&err)),
        Err(_) => Err(ChannelError::Transient(
            "Discord did not answer in time".into(),
        )),
    }
}

/// HTTP status → error class: a request Discord refused is `Rejected`; a rate
/// limit or a server error may succeed later.
pub fn classify_status(status: u16, detail: &str) -> ChannelError {
    match status {
        429 | 500..=599 => ChannelError::Transient(format!("Discord {status}: {detail}")),
        _ => ChannelError::Rejected(format!("Discord {status}: {detail}")),
    }
}

/// serenity's error → `ChannelError`. An unsuccessful request goes through
/// [`classify_status`]; transport and gateway errors are `Transient`; a payload
/// serenity refused to build is `Rejected`.
pub fn classify(err: &serenity::Error) -> ChannelError {
    match err {
        serenity::Error::Http(http) => match http.status_code() {
            Some(status) => classify_status(status.as_u16(), &http.to_string()),
            None => ChannelError::Transient(http.to_string()),
        },
        serenity::Error::Model(model) => ChannelError::Rejected(model.to_string()),
        other => ChannelError::Transient(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::PlatformIdentity;
    use crate::interface::channels::message::{AttachmentKind, Reply};

    fn file() -> Attachment {
        Attachment {
            kind: AttachmentKind::Audio,
            mime: "audio/ogg".into(),
            filename: "voice.ogg".into(),
            bytes: vec![1, 2, 3],
            duration_ms: None,
            waveform: None,
        }
    }

    #[test]
    fn status_codes_split_into_rejected_and_transient() {
        for status in [400, 401, 403, 404, 413] {
            assert!(
                matches!(classify_status(status, "x"), ChannelError::Rejected(_)),
                "{status}"
            );
        }
        for status in [429, 500, 502, 503] {
            assert!(
                matches!(classify_status(status, "x"), ChannelError::Transient(_)),
                "{status}"
            );
        }
    }

    /// The exact JSON serenity 0.12.5 emits for "mention nobody".
    fn assert_mentions_nobody(json: &serde_json::Value) {
        assert_eq!(
            json["allowed_mentions"],
            serde_json::json!({ "parse": [], "users": [], "roles": [] }),
            "{json}"
        );
    }

    #[test]
    fn every_builder_mentions_nobody_and_carries_the_embed() {
        let message = serde_json::to_value(create_message(Post::text("**hi** @everyone"))).unwrap();
        assert_mentions_nobody(&message);
        assert_eq!(message["embeds"][0]["description"], "**hi** @everyone");

        let edit = serde_json::to_value(edit_response(Post::text("e"))).unwrap();
        assert_mentions_nobody(&edit);
        assert_eq!(edit["embeds"][0]["description"], "e");

        let follow = serde_json::to_value(follow_up(Post::text("f"), true)).unwrap();
        assert_mentions_nobody(&follow);
        assert_eq!(follow["embeds"][0]["description"], "f");
        assert_eq!(follow["flags"], 64, "EPHEMERAL");
        let public = serde_json::to_value(follow_up(Post::text("f"), false)).unwrap();
        assert_ne!(public["flags"], 64);
    }

    #[test]
    fn files_are_attached_to_the_builders() {
        let post = Post {
            description: None,
            files: vec![file()],
        };
        let message = serde_json::to_value(create_message(post)).unwrap();
        assert_eq!(message["attachments"][0]["filename"], "voice.ogg");
        assert!(message
            .get("embeds")
            .is_none_or(|e| e.as_array().is_some_and(Vec::is_empty)));
    }

    #[tokio::test]
    async fn targets_off_discord_are_rejected_before_any_call() {
        let http = Http::new("not-a-token");
        let to_telegram = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Telegram(5)),
            Reply::html("x"),
        );
        assert!(matches!(
            send(&http, to_telegram).await,
            Err(ChannelError::Rejected(_))
        ));
        let to_matrix_user = OutgoingMessage::to(
            Target::Direct(PlatformIdentity::matrix("@a:b").unwrap()),
            Reply::html("x"),
        );
        assert!(matches!(
            send(&http, to_matrix_user).await,
            Err(ChannelError::Rejected(_))
        ));
    }
}
