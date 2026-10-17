//! Discord events → `IncomingMessage`s for the shared pipeline. Pure: no
//! network, no services, so it is tested against fixtures alone.

use serenity::all::{
    Attachment, CommandDataOption, CommandDataOptionValue, CommandInteraction,
    ComponentInteraction, Member, Message, User,
};

use super::commands::{inbound_text, CommandRefusal, ARGS_MAX};
use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
use crate::interface::channels::inbound::InboundText;
use crate::interface::channels::message::{IncomingMessage, MessageRef};

pub struct Parsed {
    pub message: IncomingMessage,
    pub text: InboundText,
}

/// A slash command. Conversation is the channel it was used in; `is_direct`
/// when it came from a DM; `is_mentioned` always (a slash command addresses
/// the bot, B12).
pub fn from_command(interaction: &CommandInteraction) -> Result<Parsed, CommandRefusal> {
    let text = if let Some((sub_name, sub_opts)) =
        interaction
            .data
            .options
            .iter()
            .find_map(|option| match &option.value {
                CommandDataOptionValue::SubCommand(sub_opts) => {
                    Some((option.name.as_str(), sub_opts.as_slice()))
                }
                _ => None,
            }) {
        parse_subcommand(&interaction.data.name, sub_name, sub_opts)?
    } else {
        let value = interaction
            .data
            .options
            .iter()
            .find_map(|option| match &option.value {
                CommandDataOptionValue::String(s) => Some(s.as_str()),
                _ => None,
            });
        inbound_text(&interaction.data.name, value)?
    };
    let sender = identity(&interaction.user, interaction.member.as_deref())
        .ok_or_else(|| CommandRefusal::Unknown(interaction.data.name.clone()))?;
    let message = IncomingMessage {
        channel: Channel::Discord,
        conversation: ConversationAddress::Discord(interaction.channel_id.get()),
        sender,
        message_id: MessageRef(format!("interaction:{}", interaction.id.get())),
        text: interaction_body(&text),
        reply_to: None,
        quoted: None,
        is_direct: interaction.guild_id.is_none(),
        is_mentioned: true,
    };
    Ok(Parsed { message, text })
}

fn parse_subcommand(
    command_name: &str,
    sub_name: &str,
    sub_opts: &[CommandDataOption],
) -> Result<InboundText, CommandRefusal> {
    match command_name {
        "todo" => match sub_name {
            "add" => {
                let task = sub_opts
                    .iter()
                    .find_map(|opt| match &opt.value {
                        CommandDataOptionValue::String(s) if opt.name == "task" => {
                            Some(s.as_str().trim())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                if task.chars().count() > ARGS_MAX {
                    return Err(CommandRefusal::TooLong { max: ARGS_MAX });
                }
                let raw_args = if task.is_empty() {
                    "add".to_owned()
                } else {
                    format!("add {task}")
                };
                let full_body = format!("todo {raw_args}");
                Ok(InboundText::Command {
                    command: "todo".to_owned(),
                    raw_args,
                    full_body,
                })
            }
            "done" => {
                let id = sub_opts.iter().find_map(|opt| match &opt.value {
                    CommandDataOptionValue::Integer(i) if opt.name == "id" => Some(*i),
                    _ => None,
                });
                let raw_args = match id {
                    Some(id) => format!("done {id}"),
                    None => "done".to_owned(),
                };
                let full_body = format!("todo {raw_args}");
                Ok(InboundText::Command {
                    command: "todo".to_owned(),
                    raw_args,
                    full_body,
                })
            }
            "list" => {
                let page = sub_opts.iter().find_map(|opt| match &opt.value {
                    CommandDataOptionValue::Integer(i) if opt.name == "page" => Some(*i),
                    _ => None,
                });
                let raw_args = match page {
                    Some(p) => format!("list {p}"),
                    None => "list".to_owned(),
                };
                let full_body = format!("todo {raw_args}");
                Ok(InboundText::Command {
                    command: "todo".to_owned(),
                    raw_args,
                    full_body,
                })
            }
            _ => Err(CommandRefusal::Unknown(format!("todo {sub_name}"))),
        },
        "expense" => match sub_name {
            "add" => {
                let amount = sub_opts
                    .iter()
                    .find_map(|opt| match &opt.value {
                        CommandDataOptionValue::String(s) if opt.name == "amount" => {
                            Some(s.as_str().trim())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                let note = sub_opts
                    .iter()
                    .find_map(|opt| match &opt.value {
                        CommandDataOptionValue::String(s) if opt.name == "note" => {
                            Some(s.as_str().trim())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                if amount.chars().count() + note.chars().count() > ARGS_MAX {
                    return Err(CommandRefusal::TooLong { max: ARGS_MAX });
                }
                let raw_args = match (amount.is_empty(), note.is_empty()) {
                    (true, true) => "add".to_owned(),
                    (false, true) => format!("add {amount}"),
                    (true, false) => format!("add {note}"),
                    (false, false) => format!("add {amount} {note}"),
                };
                let full_body = format!("expense {raw_args}");
                Ok(InboundText::Command {
                    command: "expense".to_owned(),
                    raw_args,
                    full_body,
                })
            }
            "list" => {
                let raw_args = "list".to_owned();
                let full_body = "expense list".to_owned();
                Ok(InboundText::Command {
                    command: "expense".to_owned(),
                    raw_args,
                    full_body,
                })
            }
            _ => Err(CommandRefusal::Unknown(format!("expense {sub_name}"))),
        },
        _ => Err(CommandRefusal::Unknown(format!(
            "{command_name} {sub_name}"
        ))),
    }
}

/// A component interaction (button click). Conversation is the channel it was
/// used in; `is_direct` when it came from a DM; `is_mentioned` always (a button
/// interaction addresses the bot).
pub fn from_component(interaction: &ComponentInteraction) -> Result<Parsed, CommandRefusal> {
    let trimmed = interaction.data.custom_id.trim();
    let (name, args) = match trimmed.split_once(char::is_whitespace) {
        Some((name, args)) => (name, Some(args.trim())),
        None => (trimmed, None),
    };
    let text = inbound_text(name, args)?;
    let sender = identity(&interaction.user, interaction.member.as_ref())
        .ok_or_else(|| CommandRefusal::Unknown(interaction.data.custom_id.clone()))?;
    let message = IncomingMessage {
        channel: Channel::Discord,
        conversation: ConversationAddress::Discord(interaction.channel_id.get()),
        sender,
        message_id: MessageRef(format!("interaction:component:{}", interaction.id.get())),
        text: interaction_body(&text),
        reply_to: None,
        quoted: None,
        is_direct: interaction.guild_id.is_none(),
        is_mentioned: true,
    };
    Ok(Parsed { message, text })
}

/// A DM text message → free text for the agent (B3). `None` for bot authors,
/// guild messages (never subscribed, but checked) and messages without text.
pub fn from_direct_message(message: &Message) -> Option<Parsed> {
    if message.author.bot || message.guild_id.is_some() {
        return None;
    }
    let text = message.content.trim();
    if text.is_empty() {
        return None;
    }
    let sender = identity(&message.author, None)?;
    let incoming = IncomingMessage {
        channel: Channel::Discord,
        conversation: ConversationAddress::Discord(message.channel_id.get()),
        sender,
        message_id: message_ref(message.channel_id.get(), message.id.get()),
        text: text.to_owned(),
        reply_to: None,
        quoted: None,
        is_direct: true,
        is_mentioned: false,
    };
    Some(Parsed {
        message: incoming,
        text: InboundText::FreeText(text.to_owned()),
    })
}

/// The ref a DM reply quotes; `outbound` parses it back into a message reference.
pub fn message_ref(channel: u64, message: u64) -> MessageRef {
    MessageRef(format!("message:{channel}:{message}"))
}

/// `(channel, message)` from a ref minted by [`message_ref`].
pub fn parse_message_ref(r: &MessageRef) -> Option<(u64, u64)> {
    let (channel, message) = r.0.strip_prefix("message:")?.split_once(':')?;
    Some((channel.parse().ok()?, message.parse().ok()?))
}

/// The command as the user would have typed it on Matrix, for `IncomingMessage.text`.
fn interaction_body(text: &InboundText) -> String {
    match text {
        InboundText::Command { full_body, .. } => full_body.clone(),
        InboundText::FreeText(t) => t.clone(),
    }
}

/// Display name: guild nickname, else global name, else username. `None` only
/// for an id that is no snowflake, which Discord never sends.
pub(crate) fn identity(user: &User, member: Option<&Member>) -> Option<PlatformIdentity> {
    let user_id = PlatformUserId::parse(Channel::Discord, &user.id.get().to_string()).ok()?;
    let display_name = member
        .and_then(|m| m.nick.clone())
        .or_else(|| user.global_name.clone())
        .unwrap_or_else(|| user.name.clone());
    Some(PlatformIdentity::new(
        Channel::Discord,
        user_id,
        Some(display_name),
    ))
}

/// True when an attachment looks like an image by its MIME type or extension.
pub fn is_image_attachment(attachment: &Attachment) -> bool {
    if let Some(ct) = &attachment.content_type {
        if ct.starts_with("image/") {
            return true;
        }
    }
    let lower = attachment.filename.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
}

/// Sniffs extension and content-type from magic bytes or attachment metadata.
pub fn image_meta(attachment: &Attachment, bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        ("jpg", "image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("png", "image/png")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ("webp", "image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ("gif", "image/gif")
    } else {
        let mime = attachment
            .content_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if mime.contains("jpeg") || mime.contains("jpg") {
            ("jpg", "image/jpeg")
        } else if mime.contains("png") {
            ("png", "image/png")
        } else if mime.contains("webp") {
            ("webp", "image/webp")
        } else if mime.contains("gif") {
            ("gif", "image/gif")
        } else {
            let lower = attachment.filename.to_ascii_lowercase();
            if lower.ends_with(".png") {
                ("png", "image/png")
            } else if lower.ends_with(".webp") {
                ("webp", "image/webp")
            } else if lower.ends_with(".gif") {
                ("gif", "image/gif")
            } else {
                ("jpg", "image/jpeg")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/discord_command_interaction.json"
    ));
    const DM_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/discord_dm_message.json"
    ));

    fn interaction(edit: impl FnOnce(&mut serde_json::Value)) -> CommandInteraction {
        let mut value: serde_json::Value = serde_json::from_str(COMMAND_FIXTURE).unwrap();
        edit(&mut value);
        serde_json::from_value(value).expect("the fixture is a serenity CommandInteraction")
    }

    fn dm(edit: impl FnOnce(&mut serde_json::Value)) -> Message {
        let mut value: serde_json::Value = serde_json::from_str(DM_FIXTURE).unwrap();
        edit(&mut value);
        serde_json::from_value(value).expect("the fixture is a serenity Message")
    }

    fn component(edit: impl FnOnce(&mut serde_json::Value)) -> ComponentInteraction {
        let mut value: serde_json::Value = serde_json::from_str(COMMAND_FIXTURE).unwrap();
        let dm_msg: serde_json::Value = serde_json::from_str(DM_FIXTURE).unwrap();
        value["type"] = 3.into();
        value["data"] = serde_json::json!({
            "custom_id": "todo done 1",
            "component_type": 2
        });
        value["message"] = dm_msg;
        edit(&mut value);
        serde_json::from_value(value).expect("the fixture is a serenity ComponentInteraction")
    }

    #[test]
    fn a_guild_slash_command_becomes_an_incoming_command() {
        let parsed = from_command(&interaction(|_| {})).unwrap();
        let m = &parsed.message;
        assert_eq!(m.channel, Channel::Discord);
        assert_eq!(
            m.conversation,
            ConversationAddress::Discord(1_100_000_000_000_000_002)
        );
        assert_eq!(m.sender.channel(), Channel::Discord);
        assert_eq!(m.sender.user_id().as_str(), "1100000000000000003");
        assert_eq!(m.sender.display_name(), Some("Evan in the guild"));
        assert_eq!(
            m.message_id,
            MessageRef("interaction:1100000000000000001".into())
        );
        assert_eq!(m.text, "todo add milk");
        assert!(!m.is_direct);
        assert!(m.is_mentioned);
        assert!(m.reply_to.is_none() && m.quoted.is_none());
        match parsed.text {
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
    fn a_dm_slash_command_is_direct_and_named_by_the_global_name() {
        let parsed = from_command(&interaction(|v| {
            let o = v.as_object_mut().unwrap();
            o.remove("guild_id");
            o.remove("member");
        }))
        .unwrap();
        assert!(parsed.message.is_direct);
        assert_eq!(parsed.message.sender.display_name(), Some("Evan"));
    }

    #[test]
    fn an_over_long_argument_is_refused_before_anything_else() {
        let long = "x".repeat(501);
        let refused = from_command(&interaction(|v| {
            v["data"]["options"][0]["value"] = serde_json::Value::String(long);
        }));
        assert!(matches!(refused, Err(CommandRefusal::TooLong { .. })));
    }

    #[test]
    fn a_dm_text_message_is_free_text() {
        let parsed = from_direct_message(&dm(|_| {})).unwrap();
        let m = &parsed.message;
        assert_eq!(
            m.conversation,
            ConversationAddress::Discord(1_100_000_000_000_000_012)
        );
        assert!(m.is_direct);
        assert!(!m.is_mentioned);
        assert_eq!(m.sender.user_id().as_str(), "1100000000000000003");
        assert_eq!(
            parse_message_ref(&m.message_id),
            Some((1_100_000_000_000_000_012, 1_100_000_000_000_000_011))
        );
        assert!(matches!(parsed.text, InboundText::FreeText(ref t) if t == "what is on my list?"));
    }

    #[test]
    fn bots_guild_messages_and_empty_text_are_ignored() {
        assert!(from_direct_message(&dm(|v| v["author"]["bot"] = true.into())).is_none());
        assert!(from_direct_message(&dm(|v| v["content"] = "  ".into())).is_none());
        assert!(
            from_direct_message(&dm(|v| v["guild_id"] = "1100000000000000099".into())).is_none()
        );
    }

    #[test]
    fn message_refs_round_trip_and_reject_others() {
        assert_eq!(parse_message_ref(&message_ref(1, 2)), Some((1, 2)));
        assert_eq!(parse_message_ref(&MessageRef("interaction:5".into())), None);
        assert_eq!(parse_message_ref(&MessageRef("message:x:2".into())), None);
    }

    #[test]
    fn a_guild_component_interaction_becomes_an_incoming_command() {
        let parsed = from_component(&component(|_| {})).unwrap();
        let m = &parsed.message;
        assert_eq!(m.channel, Channel::Discord);
        assert_eq!(
            m.conversation,
            ConversationAddress::Discord(1_100_000_000_000_000_002)
        );
        assert_eq!(m.sender.channel(), Channel::Discord);
        assert_eq!(m.sender.user_id().as_str(), "1100000000000000003");
        assert_eq!(m.sender.display_name(), Some("Evan in the guild"));
        assert_eq!(
            m.message_id,
            MessageRef("interaction:component:1100000000000000001".into())
        );
        assert_eq!(m.text, "todo done 1");
        assert!(!m.is_direct);
        assert!(m.is_mentioned);
        assert!(m.reply_to.is_none() && m.quoted.is_none());
        match parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "done 1");
                assert_eq!(full_body, "todo done 1");
            }
            InboundText::FreeText(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn a_dm_component_interaction_is_direct_and_named_by_the_global_name() {
        let parsed = from_component(&component(|v| {
            let o = v.as_object_mut().unwrap();
            o.remove("guild_id");
            o.remove("member");
        }))
        .unwrap();
        assert!(parsed.message.is_direct);
        assert_eq!(parsed.message.sender.display_name(), Some("Evan"));
    }

    #[test]
    fn a_component_interaction_without_args_parses_bare_command() {
        let parsed = from_component(&component(|v| {
            v["data"]["custom_id"] = "help".into();
        }))
        .unwrap();
        match parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "help");
                assert_eq!(raw_args, "");
                assert_eq!(full_body, "help");
            }
            InboundText::FreeText(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn an_unknown_command_in_component_interaction_is_refused() {
        let refused = from_component(&component(|v| {
            v["data"]["custom_id"] = "unknown_command".into();
        }));
        assert!(matches!(refused, Err(CommandRefusal::Unknown(_))));
    }

    #[test]
    fn an_over_long_command_in_component_interaction_is_refused() {
        let long = format!("todo {}", "x".repeat(501));
        let refused = from_component(&component(|v| {
            v["data"]["custom_id"] = long.into();
        }));
        assert!(matches!(refused, Err(CommandRefusal::TooLong { .. })));
    }

    #[test]
    fn typed_subcommands_for_todo_parse_into_inbound_commands() {
        // /todo add task:"milk"
        let add_parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "todo".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "add",
                    "type": 1,
                    "options": [
                        { "name": "task", "type": 3, "value": "milk" }
                    ]
                }
            ]);
        }))
        .unwrap();
        match add_parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "add milk");
                assert_eq!(full_body, "todo add milk");
            }
            _ => panic!("expected a command"),
        }

        // /todo done id:3
        let done_parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "todo".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "done",
                    "type": 1,
                    "options": [
                        { "name": "id", "type": 4, "value": 3 }
                    ]
                }
            ]);
        }))
        .unwrap();
        match done_parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "done 3");
                assert_eq!(full_body, "todo done 3");
            }
            _ => panic!("expected a command"),
        }

        // /todo list
        let list_parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "todo".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "list",
                    "type": 1,
                    "options": []
                }
            ]);
        }))
        .unwrap();
        match list_parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "list");
                assert_eq!(full_body, "todo list");
            }
            _ => panic!("expected a command"),
        }

        // /todo list page:2
        let list_paged = from_command(&interaction(|v| {
            v["data"]["name"] = "todo".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "list",
                    "type": 1,
                    "options": [
                        { "name": "page", "type": 4, "value": 2 }
                    ]
                }
            ]);
        }))
        .unwrap();
        match list_paged.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "list 2");
                assert_eq!(full_body, "todo list 2");
            }
            _ => panic!("expected a command"),
        }

        // overlong task refused
        let overlong_parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "todo".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "add",
                    "type": 1,
                    "options": [
                        { "name": "task", "type": 3, "value": "x".repeat(501) }
                    ]
                }
            ]);
        }));
        assert!(matches!(
            overlong_parsed,
            Err(CommandRefusal::TooLong { .. })
        ));
    }

    #[test]
    fn typed_subcommands_for_expense_parse_into_inbound_commands() {
        // /expense add amount:"12.50" note:"coffee"
        let add_parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "expense".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "add",
                    "type": 1,
                    "options": [
                        { "name": "amount", "type": 3, "value": "12.50" },
                        { "name": "note", "type": 3, "value": "coffee" }
                    ]
                }
            ]);
        }))
        .unwrap();
        match add_parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "expense");
                assert_eq!(raw_args, "add 12.50 coffee");
                assert_eq!(full_body, "expense add 12.50 coffee");
            }
            _ => panic!("expected a command"),
        }

        // /expense list
        let list_parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "expense".into();
            v["data"]["options"] = serde_json::json!([
                {
                    "name": "list",
                    "type": 1,
                    "options": []
                }
            ]);
        }))
        .unwrap();
        match list_parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "expense");
                assert_eq!(raw_args, "list");
                assert_eq!(full_body, "expense list");
            }
            _ => panic!("expected a command"),
        }
    }

    #[test]
    fn fallback_args_option_still_parses_for_todo() {
        let parsed = from_command(&interaction(|v| {
            v["data"]["name"] = "todo".into();
            v["data"]["options"] = serde_json::json!([
                { "name": "args", "type": 3, "value": "add milk" }
            ]);
        }))
        .unwrap();
        match parsed.text {
            InboundText::Command {
                command,
                raw_args,
                full_body,
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "add milk");
                assert_eq!(full_body, "todo add milk");
            }
            _ => panic!("expected a command"),
        }
    }

    fn attachment_fixture(filename: &str, content_type: Option<&str>) -> Attachment {
        serde_json::from_value(serde_json::json!({
            "id": "1100000000000000020",
            "filename": filename,
            "size": 1024,
            "url": "https://cdn.discordapp.com/attachments/1/2/pic.png",
            "proxy_url": "https://media.discordapp.net/attachments/1/2/pic.png",
            "content_type": content_type,
        }))
        .unwrap()
    }

    #[test]
    fn is_image_attachment_detects_mime_and_extensions() {
        let png = attachment_fixture("receipt.png", Some("image/png"));
        assert!(is_image_attachment(&png));

        let jpeg_mime = attachment_fixture("receipt.dat", Some("image/jpeg"));
        assert!(is_image_attachment(&jpeg_mime));

        let jpg_ext = attachment_fixture("scan.JPG", None);
        assert!(is_image_attachment(&jpg_ext));

        let webp_ext = attachment_fixture("photo.webp", Some("application/octet-stream"));
        assert!(is_image_attachment(&webp_ext));

        let gif_ext = attachment_fixture("anim.gif", None);
        assert!(is_image_attachment(&gif_ext));

        let pdf = attachment_fixture("document.pdf", Some("application/pdf"));
        assert!(!is_image_attachment(&pdf));

        let txt = attachment_fixture("notes.txt", None);
        assert!(!is_image_attachment(&txt));
    }

    #[test]
    fn image_meta_resolves_magic_bytes_mime_and_extension() {
        let dummy = attachment_fixture("unknown.bin", None);

        // Magic bytes
        let (ext, mime) = image_meta(&dummy, b"\xFF\xD8\xFF\xE0\x00\x10JFIF");
        assert_eq!((ext, mime), ("jpg", "image/jpeg"));

        let (ext, mime) = image_meta(&dummy, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR");
        assert_eq!((ext, mime), ("png", "image/png"));

        let (ext, mime) = image_meta(&dummy, b"RIFF\x20\x00\x00\x00WEBPVP8 ");
        assert_eq!((ext, mime), ("webp", "image/webp"));

        let (ext, mime) = image_meta(&dummy, b"GIF89a\x01\x00\x01\x00");
        assert_eq!((ext, mime), ("gif", "image/gif"));

        // Content-type fallback
        let webp_att = attachment_fixture("receipt", Some("image/webp"));
        let (ext, mime) = image_meta(&webp_att, b"not_magic");
        assert_eq!((ext, mime), ("webp", "image/webp"));

        // Filename fallback
        let png_att = attachment_fixture("receipt.png", None);
        let (ext, mime) = image_meta(&png_att, b"not_magic");
        assert_eq!((ext, mime), ("png", "image/png"));

        // Default
        let (ext, mime) = image_meta(&dummy, b"not_magic");
        assert_eq!((ext, mime), ("jpg", "image/jpeg"));
    }
}
