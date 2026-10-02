//! `ChannelAdapter::send` for Telegram: executes the plan from `render` with
//! teloxide-core's `Bot`. teloxide errors become `ChannelError` here and nowhere
//! else, with the bot token scrubbed from every message.

use std::future::Future;
use std::time::Duration;

use teloxide_core::payloads::{SendAudioSetters, SendDocumentSetters, SendMessageSetters};
use teloxide_core::payloads::{SendPhotoSetters, SendVoiceSetters};
use teloxide_core::prelude::*;
use teloxide_core::types::{
    ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, Message, MessageId, ParseMode,
    ReplyParameters,
};
use teloxide_core::{ApiError, RequestError};

use super::inbound::{message_ref, parse_message_ref};
use super::render::{file_method, plan, FileMethod, Part, Plan};
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::message::{Attachment, OutgoingMessage, Target};

/// Every Bot API call is bounded by this (uploads included).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Posts `message`: `Conversation(Telegram(id))` to that chat, `Direct` on
/// Telegram to the user's private chat (its id is the user id; Telegram answers
/// 403 until the user has started the bot, which is `Rejected`). Anything else
/// is `Rejected` before any call. T14: no dedupe, `idempotency_key` and
/// `join_if_needed` are ignored. Returns the last message's ref.
pub async fn send(bot: &Bot, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
    let chat = match &message.target {
        Target::Conversation(ConversationAddress::Telegram(id)) => *id,
        Target::Direct(identity) if identity.channel() == Channel::Telegram => identity
            .user_id()
            .as_str()
            .parse::<i64>()
            .map_err(|_| ChannelError::Rejected("not a Telegram user id".to_owned()))?,
        other => {
            return Err(ChannelError::Rejected(format!(
                "not a Telegram target: {other:?}"
            )))
        }
    };
    // Telegram group and supergroup ids are negative; users' private chats are positive.
    let planned = plan(&message.content, message.visibility, chat < 0)?;
    // A reply target in another chat, or a malformed ref, is simply not replied to.
    let reply_to = message
        .reply_to
        .as_ref()
        .and_then(parse_message_ref)
        .filter(|(c, _)| *c == chat)
        .map(|(_, m)| m);
    deliver(bot, ChatId(chat), reply_to, planned).await
}

async fn deliver(
    bot: &Bot,
    chat: ChatId,
    reply_to: Option<i32>,
    plan: Plan,
) -> Result<SentRef, ChannelError> {
    let last_text = plan.parts.iter().rposition(|p| matches!(p, Part::Text(_)));
    let markup =
        (!plan.keyboard.is_empty()).then(|| {
            InlineKeyboardMarkup::new(plan.keyboard.iter().map(|(label, data)| {
                vec![InlineKeyboardButton::callback(label.clone(), data.clone())]
            }))
        });
    let mut last = None;
    for (i, part) in plan.parts.iter().enumerate() {
        let reply = reply_to
            .filter(|_| i == 0)
            .map(|m| ReplyParameters::new(MessageId(m)).allow_sending_without_reply());
        let sent = match part {
            Part::Text(text) => {
                let mut request = bot.send_message(chat, text).parse_mode(ParseMode::Html);
                if let Some(reply) = reply {
                    request = request.reply_parameters(reply);
                }
                if let Some(markup) = markup.clone().filter(|_| Some(i) == last_text) {
                    request = request.reply_markup(markup);
                }
                bounded(request.send()).await?
            }
            Part::File(file) => send_file(bot, chat, reply, file).await?,
        };
        last = Some(message_ref(chat.0, sent.id.0));
    }
    Ok(SentRef { message: last })
}

async fn send_file(
    bot: &Bot,
    chat: ChatId,
    reply: Option<ReplyParameters>,
    file: &Attachment,
) -> Result<Message, ChannelError> {
    let input = InputFile::memory(file.bytes.clone()).file_name(file.filename.clone());
    let seconds = file
        .duration_ms
        .map(|ms| u32::try_from(ms / 1000).unwrap_or(u32::MAX));
    // The setters differ per payload, so each method builds its own request.
    match file_method(file) {
        FileMethod::Voice => {
            let mut r = bot.send_voice(chat, input);
            if let Some(s) = seconds {
                r = r.duration(s);
            }
            if let Some(reply) = reply {
                r = r.reply_parameters(reply);
            }
            bounded(r.send()).await
        }
        FileMethod::Audio => {
            let mut r = bot.send_audio(chat, input);
            if let Some(s) = seconds {
                r = r.duration(s);
            }
            if let Some(reply) = reply {
                r = r.reply_parameters(reply);
            }
            bounded(r.send()).await
        }
        FileMethod::Photo => {
            let mut r = bot.send_photo(chat, input);
            if let Some(reply) = reply {
                r = r.reply_parameters(reply);
            }
            bounded(r.send()).await
        }
        FileMethod::Document => {
            let mut r = bot.send_document(chat, input);
            if let Some(reply) = reply {
                r = r.reply_parameters(reply);
            }
            bounded(r.send()).await
        }
    }
}

/// Runs one Bot API call under [`CALL_TIMEOUT`], mapping its error.
pub async fn bounded<T>(
    call: impl Future<Output = Result<T, RequestError>>,
) -> Result<T, ChannelError> {
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(classify(&err)),
        Err(_) => Err(ChannelError::Transient(
            "Telegram did not answer in time".into(),
        )),
    }
}

/// teloxide's error -> `ChannelError`. An API refusal (blocked bot, unknown chat,
/// bad request) will not succeed on retry; flood control, transport and parse
/// failures may. An `Unknown` API text is judged by its prefix: Telegram words
/// client errors "Bad Request"/"Forbidden"/"Unauthorized", anything else (a 5xx)
/// is worth retrying.
pub(super) fn classify(err: &RequestError) -> ChannelError {
    let detail = redact(&err.to_string());
    match err {
        RequestError::Api(ApiError::Unknown(text))
            if !["Bad Request", "Forbidden", "Unauthorized", "Conflict"]
                .iter()
                .any(|p| text.starts_with(p)) =>
        {
            ChannelError::Transient(detail)
        }
        RequestError::Api(_) | RequestError::MigrateToChatId(_) => ChannelError::Rejected(detail),
        RequestError::RetryAfter(_)
        | RequestError::Network(_)
        | RequestError::InvalidJson { .. }
        | RequestError::Io(_) => ChannelError::Transient(detail),
    }
}

/// teloxide already hides the token in transport errors; this is the second
/// line, because a URL such as `.../bot<token>/sendMessage` must never reach a
/// log or a delivery error whatever builds the text.
pub(super) fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("/bot") {
        let (head, tail) = rest.split_at(at + "/bot".len());
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == ')' || c == '"')
            .unwrap_or(tail.len());
        // Only a token-shaped segment (`<digits>:<secret>`) is replaced.
        if tail[..end].contains(':') {
            out.push_str("<redacted>");
        } else {
            out.push_str(&tail[..end]);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::PlatformIdentity;
    use crate::interface::channels::message::Reply;
    use teloxide_core::{ApiError, RequestError};

    fn is_rejected(e: &ChannelError) -> bool {
        matches!(e, ChannelError::Rejected(_))
    }
    fn is_transient(e: &ChannelError) -> bool {
        matches!(e, ChannelError::Transient(_))
    }

    #[test]
    fn classify_splits_refusals_from_retryable_failures() {
        use teloxide_core::types::Seconds;
        assert!(is_rejected(&classify(&RequestError::Api(
            ApiError::BotBlocked
        ))));
        assert!(is_rejected(&classify(&RequestError::Api(
            ApiError::ChatNotFound
        ))));
        assert!(is_rejected(&classify(&RequestError::Api(
            ApiError::InvalidToken
        ))));
        assert!(is_rejected(&classify(&RequestError::Api(
            ApiError::Unknown("Bad Request: nope".into())
        ))));
        assert!(is_transient(&classify(&RequestError::Api(
            ApiError::Unknown("Internal Server Error".into())
        ))));
        assert!(is_transient(&classify(&RequestError::RetryAfter(
            Seconds::from_seconds(3)
        ))));
    }

    #[test]
    fn redact_removes_a_bot_token_from_any_text() {
        let url = "error sending request for url (https://api.telegram.org/bot123456:AAEsecret-token_x/sendMessage): timed out";
        let clean = redact(url);
        assert!(!clean.contains("AAEsecret"), "{clean}");
        assert!(!clean.contains("123456"), "{clean}");
        assert!(clean.contains("/sendMessage"), "{clean}");
        assert_eq!(redact("nothing to hide"), "nothing to hide");
    }

    #[tokio::test]
    async fn targets_off_telegram_are_rejected_before_any_call() {
        let bot = Bot::new("x");
        let discord = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Discord(5)),
            Reply::html("x"),
        );
        assert!(is_rejected(&send(&bot, discord).await.unwrap_err()));
        let matrix = OutgoingMessage::to(
            Target::Direct(PlatformIdentity::matrix("@a:b").unwrap()),
            Reply::html("x"),
        );
        assert!(is_rejected(&send(&bot, matrix).await.unwrap_err()));
        let empty = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Telegram(5)),
            Reply::html("  "),
        );
        assert!(is_rejected(&send(&bot, empty).await.unwrap_err()));
    }
}
