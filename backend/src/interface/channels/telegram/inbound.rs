//! Telegram updates → `IncomingMessage`s for the shared pipeline. Pure: no
//! network, no services, so it is tested against fixtures alone.

use teloxide_core::types::UpdateKind as TgUpdateKind;
use teloxide_core::types::{CallbackQuery, Chat, MaybeInaccessibleMessage, Message, Update, User};

use super::commands::{inbound_text, parse_callback, parse_command, CommandRefusal, Parsed};
use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
use crate::domain::chat::QuotedMessage;
use crate::interface::channels::inbound::InboundText;
use crate::interface::channels::message::{IncomingMessage, MessageRef};

/// What the worker dispatches, and where the platform-side acknowledgement goes.
pub struct Normalized {
    pub message: IncomingMessage,
    pub text: InboundText,
    pub kind: UpdateKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateKind {
    Message {
        chat_id: i64,
        message_id: i32,
    },
    /// A button press: `query_id` must be answered to stop the client spinner.
    Callback {
        query_id: String,
        chat_id: i64,
        message_id: i32,
    },
}

/// One `Update` → what to dispatch, or `None` (T3: not addressed, bot author,
/// edited message, channel post, empty, unusable callback). A refused command
/// carries the update's kind so the worker can answer where it came from.
pub fn normalize(
    update: &Update,
    our_username: &str,
) -> Option<Result<Normalized, (UpdateKind, CommandRefusal)>> {
    match &update.kind {
        TgUpdateKind::Message(message) => from_message(message, our_username),
        TgUpdateKind::CallbackQuery(query) => from_callback(query),
        _ => None,
    }
}

fn from_message(
    message: &Message,
    our_username: &str,
) -> Option<Result<Normalized, (UpdateKind, CommandRefusal)>> {
    let from = message.from.as_ref()?;
    if from.is_bot || message.chat.is_channel() {
        return None;
    }
    let body = message.text()?.trim();
    if body.is_empty() {
        return None;
    }
    let sender = identity(from)?;
    let direct = message.chat.is_private();
    let kind = UpdateKind::Message {
        chat_id: message.chat.id.0,
        message_id: message.id.0,
    };

    let (text, mentioned) = match parse_command(body, our_username) {
        Parsed::NotForUs => return None,
        Parsed::Command { command, args } => match inbound_text(&command, &args) {
            Ok(text) => (text, true),
            Err(refusal) => return Some(Err((kind, refusal))),
        },
        Parsed::NotACommand if direct => (InboundText::FreeText(body.to_owned()), false),
        Parsed::NotACommand => {
            let reply_to_us = message
                .reply_to_message()
                .is_some_and(|r| is_ours(r, our_username));
            let stripped = strip_mention(body, our_username);
            if stripped.is_none() && !reply_to_us {
                return None;
            }
            let prompt = stripped.unwrap_or_else(|| body.to_owned());
            if prompt.is_empty() {
                return None;
            }
            (InboundText::FreeText(prompt), true)
        }
    };

    let reply = message.reply_to_message();
    let incoming = IncomingMessage {
        channel: Channel::Telegram,
        conversation: ConversationAddress::Telegram(message.chat.id.0),
        sender,
        message_id: message_ref(message.chat.id.0, message.id.0),
        text: body_of(&text),
        reply_to: reply.map(|r| message_ref(message.chat.id.0, r.id.0)),
        quoted: reply.and_then(quoted),
        is_direct: direct,
        is_mentioned: mentioned,
    };
    Some(Ok(Normalized {
        message: incoming,
        text,
        kind,
    }))
}

/// T6: the presser is the sender, the button's chat the conversation; the data
/// is parsed like typed text and never trusted further.
fn from_callback(
    query: &CallbackQuery,
) -> Option<Result<Normalized, (UpdateKind, CommandRefusal)>> {
    if query.from.is_bot {
        return None;
    }
    let MaybeInaccessibleMessage::Regular(button) = query.message.as_ref()? else {
        return None;
    };
    let Parsed::Command { command, args } = parse_callback(query.data.as_deref()?) else {
        return None;
    };
    let sender = identity(&query.from)?;
    let kind = UpdateKind::Callback {
        query_id: query.id.0.clone(),
        chat_id: button.chat.id.0,
        message_id: button.id.0,
    };
    let text = match inbound_text(&command, &args) {
        Ok(text) => text,
        Err(refusal) => return Some(Err((kind, refusal))),
    };
    let incoming = IncomingMessage {
        channel: Channel::Telegram,
        conversation: ConversationAddress::Telegram(button.chat.id.0),
        sender,
        message_id: message_ref(button.chat.id.0, button.id.0),
        text: body_of(&text),
        reply_to: None,
        quoted: None,
        is_direct: is_direct(&button.chat),
        is_mentioned: true,
    };
    Some(Ok(Normalized {
        message: incoming,
        text,
        kind,
    }))
}

fn is_direct(chat: &Chat) -> bool {
    chat.is_private()
}

/// The ref a reply targets; `outbound` parses it back into a message reference.
pub fn message_ref(chat: i64, message: i32) -> MessageRef {
    MessageRef(format!("message:{chat}:{message}"))
}

/// The command as the user would have typed it on Matrix, for `IncomingMessage.text`.
fn body_of(text: &InboundText) -> String {
    match text {
        InboundText::Command { full_body, .. } => full_body.clone(),
        InboundText::FreeText(t) => t.clone(),
    }
}

/// T5: the replied-to text, so `/todo add` and `/remind` can use it as a body.
fn quoted(reply: &Message) -> Option<QuotedMessage> {
    let body = reply.text()?.trim();
    if body.is_empty() {
        return None;
    }
    Some(QuotedMessage::new(
        reply.from.as_ref().and_then(display_name),
        body.to_owned(),
        Some(reply.id.0.to_string()),
    ))
}

/// A reply to one of the bot's own messages counts as addressing it.
fn is_ours(reply: &Message, our_username: &str) -> bool {
    reply.from.as_ref().is_some_and(|u| {
        u.is_bot
            && u.username
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(our_username))
    })
}

/// `text` without every `@<our_username>` mention (case-insensitive, whole
/// handle), or `None` when it holds none. `@MyBotFriend` is not a mention.
fn strip_mention(text: &str, our_username: &str) -> Option<String> {
    let handle = format!("@{}", our_username.to_lowercase());
    let lower = text.to_lowercase();
    // Lower-casing can change byte lengths for non-ASCII text; then positions no
    // longer line up with `text`, and a mention (ASCII handle) is not reliable.
    if lower.len() != text.len() {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let (mut found, mut at) = (false, 0);
    while let Some(i) = lower[at..].find(&handle).map(|i| i + at) {
        let end = i + handle.len();
        let before_ok = !text[..i].chars().next_back().is_some_and(is_word);
        let after_ok = !text[end..].chars().next().is_some_and(is_word);
        if before_ok && after_ok {
            out.push_str(&text[at..i]);
            found = true;
            at = end;
        } else {
            out.push_str(&text[at..end]);
            at = end;
        }
    }
    out.push_str(&text[at..]);
    found.then(|| out.trim().to_owned())
}

fn display_name(user: &User) -> Option<String> {
    let full = match &user.last_name {
        Some(last) => format!("{} {last}", user.first_name),
        None => user.first_name.clone(),
    };
    let full = full.trim();
    if full.is_empty() {
        user.username.clone().filter(|u| !u.is_empty())
    } else {
        Some(full.to_owned())
    }
}

/// `None` only for an id Telegram never sends (non-positive for a user).
fn identity(user: &User) -> Option<PlatformIdentity> {
    let user_id = match PlatformUserId::parse(Channel::Telegram, &user.id.0.to_string()) {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(%error, "telegram sender id rejected; update dropped");
            return None;
        }
    };
    Some(PlatformIdentity::new(
        Channel::Telegram,
        user_id,
        display_name(user),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::{Channel, ConversationAddress};
    use serde_json::{json, Value};

    const PRIVATE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_private_text.json"
    ));
    const GROUP: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_group_command.json"
    ));
    const CALLBACK: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_callback_query.json"
    ));

    fn update(fixture: &str, edit: impl FnOnce(&mut Value)) -> Update {
        let mut value: Value = serde_json::from_str(fixture).unwrap();
        edit(&mut value);
        // From a string, never `from_value`: teloxide-core's `Update` deserializer
        // goes through a raw value and silently yields `UpdateKind::Error` otherwise.
        // Production deserializes the webhook/polling bytes the same way.
        let update: Update = serde_json::from_str(&value.to_string())
            .unwrap_or_else(|e| panic!("the fixture is a teloxide-core Update: {e}"));
        assert!(
            !matches!(update.kind, TgUpdateKind::Error(_)),
            "the fixture parsed as an unknown update"
        );
        update
    }

    fn group_text(text: &str) -> Update {
        update(GROUP, |v| {
            v["message"]["text"] = text.into();
            v["message"].as_object_mut().unwrap().remove("entities");
        })
    }

    fn ok(update: &Update) -> Normalized {
        match normalize(update, "MyBot") {
            Some(Ok(n)) => n,
            _ => panic!("expected a normalized update"),
        }
    }

    #[test]
    fn a_private_text_is_direct_free_text() {
        let n = ok(&update(PRIVATE, |_| {}));
        let m = &n.message;
        assert_eq!(m.channel, Channel::Telegram);
        assert_eq!(m.conversation, ConversationAddress::Telegram(4_100_000_003));
        assert_eq!(m.sender.user_id().as_str(), "4100000003");
        assert_eq!(m.sender.display_name(), Some("Evan Fixture"));
        assert_eq!(m.message_id, MessageRef("message:4100000003:101".into()));
        assert!(m.is_direct && !m.is_mentioned);
        assert!(m.quoted.is_none());
        assert!(matches!(n.text, InboundText::FreeText(ref t) if t == "what is on my list?"));
        assert!(matches!(
            n.kind,
            UpdateKind::Message {
                chat_id: 4_100_000_003,
                message_id: 101
            }
        ));
    }

    #[test]
    fn a_private_command_is_a_bare_command() {
        let n = ok(&update(PRIVATE, |v| {
            v["message"]["text"] = "/Expense 5 coffee".into()
        }));
        match n.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "expense");
                assert_eq!(raw_args, "5 coffee");
                assert_eq!(full_body, "expense 5 coffee");
            }
            InboundText::FreeText(_) => panic!("expected a command"),
        }
        assert_eq!(n.message.text, "expense 5 coffee");
    }

    #[test]
    fn a_group_command_for_us_is_a_mentioned_command() {
        let n = ok(&update(GROUP, |_| {}));
        let m = &n.message;
        assert_eq!(
            m.conversation,
            ConversationAddress::Telegram(-1_004_100_000_009)
        );
        assert_eq!(m.sender.display_name(), Some("Evan"));
        assert!(!m.is_direct && m.is_mentioned);
        match n.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "add milk");
                assert_eq!(full_body, "todo add milk");
            }
            InboundText::FreeText(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn a_group_command_for_another_bot_is_dropped() {
        let u = update(GROUP, |v| {
            v["message"]["text"] = "/todo@OtherBot add milk".into()
        });
        assert!(normalize(&u, "MyBot").is_none());
    }

    #[test]
    fn group_text_without_a_mention_is_dropped() {
        assert!(normalize(&group_text("anyone seen my keys?"), "MyBot").is_none());
    }

    #[test]
    fn group_text_with_our_mention_is_free_text_with_the_mention_stripped() {
        let n = ok(&group_text("@mybot what is on my list?"));
        assert!(n.message.is_mentioned && !n.message.is_direct);
        assert!(matches!(n.text, InboundText::FreeText(ref t) if t == "what is on my list?"));
        let n = ok(&group_text("what is on my list? @MyBot"));
        assert!(matches!(n.text, InboundText::FreeText(ref t) if t == "what is on my list?"));
    }

    #[test]
    fn a_mention_of_another_user_is_not_ours() {
        assert!(normalize(&group_text("@MyBotFriend hello"), "MyBot").is_none());
    }

    #[test]
    fn a_bare_mention_has_no_prompt_and_is_dropped() {
        assert!(normalize(&group_text("@MyBot"), "MyBot").is_none());
    }

    #[test]
    fn a_reply_to_one_of_our_messages_is_addressed() {
        let u = update(GROUP, |v| {
            v["message"]["text"] = "and tomorrow?".into();
            v["message"].as_object_mut().unwrap().remove("entities");
            v["message"]["reply_to_message"] = json!({
                "message_id": 200,
                "from": { "id": 4100000099u64, "is_bot": true, "first_name": "My Bot", "username": "MyBot" },
                "chat": { "id": -1004100000009i64, "title": "Household", "type": "supergroup" },
                "date": 1790000050,
                "text": "Two items due today."
            });
        });
        let n = ok(&u);
        assert!(n.message.is_mentioned);
        assert!(matches!(n.text, InboundText::FreeText(ref t) if t == "and tomorrow?"));
        let quoted = n.message.quoted.expect("T5: the replied-to text is quoted");
        assert_eq!(quoted.body, "Two items due today.");
        assert_eq!(quoted.event_id.as_deref(), Some("200"));
        assert_eq!(
            n.message.reply_to,
            Some(MessageRef("message:-1004100000009:200".into()))
        );
    }

    #[test]
    fn a_reply_to_someone_else_is_quoted_but_not_addressed() {
        let reply = |text: &str| {
            update(GROUP, |v| {
                v["message"]["text"] = text.into();
                v["message"].as_object_mut().unwrap().remove("entities");
                v["message"]["reply_to_message"] = json!({
                    "message_id": 150,
                    "from": { "id": 4100000005u64, "is_bot": false, "first_name": "Sam" },
                    "chat": { "id": -1004100000009i64, "title": "Household", "type": "supergroup" },
                    "date": 1790000050,
                    "text": "buy oat milk"
                });
            })
        };
        assert!(normalize(&reply("sounds good"), "MyBot").is_none());
        let n = ok(&reply("/todo@MyBot add"));
        let quoted = n.message.quoted.expect("quoted");
        assert_eq!(quoted.author.as_deref(), Some("Sam"));
        assert_eq!(quoted.body, "buy oat milk");
    }

    #[test]
    fn a_callback_is_dispatched_as_the_presser_in_the_buttons_chat() {
        let n = ok(&update(CALLBACK, |_| {}));
        let m = &n.message;
        assert_eq!(m.sender.user_id().as_str(), "4100000004");
        assert_eq!(
            m.conversation,
            ConversationAddress::Telegram(-1_004_100_000_009)
        );
        assert!(!m.is_direct && m.is_mentioned);
        assert_eq!(
            m.message_id,
            MessageRef("message:-1004100000009:303".into())
        );
        match n.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "done 12");
                assert_eq!(full_body, "todo done 12");
            }
            InboundText::FreeText(_) => panic!("expected a command"),
        }
        match n.kind {
            UpdateKind::Callback {
                query_id,
                chat_id,
                message_id,
            } => {
                assert_eq!(query_id, "4100000000000000001");
                assert_eq!((chat_id, message_id), (-1_004_100_000_009, 303));
            }
            UpdateKind::Message { .. } => panic!("expected a callback"),
        }
    }

    #[test]
    fn callbacks_without_usable_data_or_message_are_dropped() {
        let no_data = update(CALLBACK, |v| {
            v["callback_query"].as_object_mut().unwrap().remove("data");
        });
        assert!(normalize(&no_data, "MyBot").is_none());
        let empty = update(CALLBACK, |v| v["callback_query"]["data"] = "".into());
        assert!(normalize(&empty, "MyBot").is_none());
        let no_message = update(CALLBACK, |v| {
            v["callback_query"]
                .as_object_mut()
                .unwrap()
                .remove("message");
        });
        assert!(normalize(&no_message, "MyBot").is_none());
        let inaccessible = update(CALLBACK, |v| {
            v["callback_query"]["message"] = json!({
                "message_id": 303,
                "chat": { "id": -1004100000009i64, "title": "Household", "type": "supergroup" },
                "date": 0
            });
        });
        assert!(normalize(&inaccessible, "MyBot").is_none());
    }

    #[test]
    fn an_over_long_command_is_refused_with_its_update_kind() {
        let u = update(PRIVATE, |v| {
            v["message"]["text"] = format!("/todo {}", "x".repeat(501)).into();
        });
        match normalize(&u, "MyBot") {
            Some(Err((
                UpdateKind::Message {
                    chat_id,
                    message_id,
                },
                CommandRefusal::TooLong { max },
            ))) => {
                assert_eq!((chat_id, message_id, max), (4_100_000_003, 101, 500));
            }
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn bots_edits_channel_posts_and_empty_text_are_ignored() {
        let bot = update(PRIVATE, |v| v["message"]["from"]["is_bot"] = true.into());
        assert!(normalize(&bot, "MyBot").is_none());
        let blank = update(PRIVATE, |v| v["message"]["text"] = "   ".into());
        assert!(normalize(&blank, "MyBot").is_none());
        let no_text = update(PRIVATE, |v| {
            v["message"].as_object_mut().unwrap().remove("text");
        });
        assert!(normalize(&no_text, "MyBot").is_none());
        let edited = update(PRIVATE, |v| {
            let message = v.as_object_mut().unwrap().remove("message").unwrap();
            v["edited_message"] = message;
        });
        assert!(normalize(&edited, "MyBot").is_none());
        let channel = update(PRIVATE, |v| {
            let mut message = v.as_object_mut().unwrap().remove("message").unwrap();
            message["chat"] =
                json!({ "id": -1004100000077i64, "title": "News", "type": "channel" });
            message.as_object_mut().unwrap().remove("from");
            v["channel_post"] = message;
        });
        assert!(normalize(&channel, "MyBot").is_none());
    }

    #[test]
    fn a_sender_without_a_names_falls_back_to_the_username() {
        let u = update(PRIVATE, |v| {
            let from = v["message"]["from"].as_object_mut().unwrap();
            from.insert("first_name".into(), "".into());
            from.remove("last_name");
        });
        assert_eq!(ok(&u).message.sender.display_name(), Some("evan_fixture"));
    }
}
