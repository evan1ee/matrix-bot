//! Matrix's side of the inbound pipeline: everything that is Matrix before the
//! shared steps (quote resolution, typing and stage indicators, the `!` and
//! mention syntax) and after them (rendering the reply into the room).

use std::sync::Arc;

use matrix_sdk::{
    room::Room,
    ruma::events::room::message::{MessageType, OriginalSyncRoomMessageEvent},
    ruma::UserId,
};
use tracing::{error, info};

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::channels::{
    Channel, ChannelsError, ConversationAddress, PlatformIdentity, PlatformUserId,
};
use crate::interface::channels::inbound::{Inbound, InboundOutcome, InboundText};
use crate::interface::channels::matrix::dispatcher::CommandParser;
use crate::interface::channels::matrix::reaction_targets::{
    link_turn_targets, parse_entity_from_reply, try_record_from_reply,
};
use crate::interface::channels::matrix::reply::send_html;
use crate::interface::channels::matrix::stage::{StageIndicator, TypingGuard};
use crate::interface::channels::matrix::voice_upload::deliver_audio;
use crate::interface::channels::message::{IncomingMessage, MessageRef, Reply};
use crate::interface::chat::ParsedCommand;

/// The sender as the pipeline sees them: the MXID, displayed by its localpart
/// (the name the parser has always recorded for "who typed this").
pub(crate) fn matrix_sender(mxid: &str) -> Result<PlatformIdentity, ChannelsError> {
    let user_id = PlatformUserId::parse(Channel::Matrix, mxid)?;
    let localpart = mxid
        .trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or(mxid)
        .to_owned();
    Ok(PlatformIdentity::new(
        Channel::Matrix,
        user_id,
        Some(localpart),
    ))
}

/// The neutral form of a parsed Matrix text event.
///
/// Matrix keeps its own parsing (`!`, mentions, quotes), so the pipeline gets a
/// finished `Command`: a direct-message prompt arrives as `ai`, exactly as the
/// parser decided, and the shared free-text rule never applies to Matrix.
pub(crate) fn to_inbound(
    ev: &OriginalSyncRoomMessageEvent,
    body: &str,
    room_id: &str,
    is_direct: bool,
    parsed: &ParsedCommand,
) -> (IncomingMessage, InboundText) {
    let reply_to =
        crate::interface::channels::matrix::quote::extract_replied_event_id(&ev.content.relates_to)
            .map(MessageRef);
    let message = IncomingMessage {
        channel: Channel::Matrix,
        conversation: ConversationAddress::Matrix(room_id.to_owned()),
        sender: parsed.sender.clone(),
        message_id: MessageRef(ev.event_id.to_string()),
        text: body.to_owned(),
        reply_to,
        quoted: parsed.quoted.clone(),
        is_direct,
        is_mentioned: parsed.is_mentioned,
    };
    let text = InboundText::Command {
        command: parsed.command.clone(),
        raw_args: parsed.raw_args.clone(),
        full_body: parsed.full_body.clone(),
    };
    (message, text)
}

/// Parses a text message and hands it to the shared pipeline, then shows the
/// reply in the room: over a stage indicator for an AI turn, else as a plain
/// reply.
///
/// A no-op for any other message type.
pub async fn handle_text(
    ev: &OriginalSyncRoomMessageEvent,
    room: &Room,
    own_id: &UserId,
    is_direct: bool,
    inbound: &Inbound,
    db: &Database,
    services: &AppServices,
) {
    let MessageType::Text(ref text_content) = ev.content.msgtype else {
        return;
    };

    // Built before the parse (the parser records who sent the command); a
    // sender that is not a valid MXID cannot be answered, so the message is dropped.
    let sender = match matrix_sender(ev.sender.as_str()) {
        Ok(sender) => sender,
        Err(err) => {
            error!(
                "Could not parse the sender {}: {:?} — dropping the command",
                ev.sender, err
            );
            return;
        }
    };
    let formatted_body = text_content.formatted.as_ref().map(|f| f.body.as_str());

    let Some(mut parsed) = CommandParser::parse_or_dm_prompt(
        &text_content.body,
        formatted_body,
        own_id,
        room.room_id().as_str(),
        &sender,
        is_direct,
    ) else {
        return;
    };

    let replied_event_id =
        crate::interface::channels::matrix::quote::extract_replied_event_id(&ev.content.relates_to);
    if let Some(ref eid) = replied_event_id {
        if parsed.quoted.is_none() {
            parsed.quoted =
                crate::interface::channels::matrix::quote::resolve_quoted_event_from_room(
                    room, eid,
                )
                .await;
        } else if let Some(ref mut q) = parsed.quoted {
            if q.event_id.is_none() {
                q.event_id = Some(eid.clone());
            }
        }
        // Whether the quote is the bot's own message (its confirmation
        // question) comes from the replied-to event's sender, never from the
        // quote text.
        if let Some(ref mut q) = parsed.quoted {
            q.from_bot =
                crate::interface::channels::matrix::quote::replied_event_is_from(room, eid, own_id)
                    .await;
        }
    }

    info!(
        "Command received in [{}] from [{}]: command='{}', args='{}'",
        parsed.conversation.to_storage(),
        parsed.sender_name(),
        parsed.command,
        parsed.raw_args
    );

    let is_link = parsed.command.eq_ignore_ascii_case("link");
    let is_ai = CommandParser::is_ai_prompt_command(&parsed);
    let (message, text) = to_inbound(
        ev,
        &text_content.body,
        room.room_id().as_str(),
        is_direct,
        &parsed,
    );

    // `!link` shows no typing indicator: it is answered before the sender is
    // resolved and never reaches a module.
    let typing_guard = if !is_link && (is_direct || parsed.is_mentioned) {
        Some(TypingGuard::start(room.clone()))
    } else {
        None
    };

    let stage_indicator = if is_ai {
        let ind = StageIndicator::start(room, "🧠 Thinking...", "🧠 <i>正在思考...</i>").await;
        Some(ind)
    } else {
        None
    };
    let progress: Option<Arc<dyn crate::llm::progress::ProgressSink>> = stage_indicator
        .as_ref()
        .map(|ind| Arc::new(ind.reporter()) as Arc<dyn crate::llm::progress::ProgressSink>);

    let outcome = inbound.handle(&message, text, progress).await;

    if let Some(mut guard) = typing_guard {
        guard.stop().await;
    }

    match outcome {
        InboundOutcome::Replied(reply) if is_link => {
            send_html(room, reply.html_body()).await;
        }
        InboundOutcome::Replied(reply) => {
            show_reply(room, ev, &parsed, reply, stage_indicator, db, services).await;
        }
        InboundOutcome::Ignored => {
            if let Some(indicator) = stage_indicator {
                indicator
                    .cancel_with_error(
                        "⚠️ AI 处理失败：未能生成回复或处理超时。请尝试重新提问。",
                        "⚠️ <b>AI 处理失败</b><br/>• <b>原因:</b> 未能生成有效回复或处理超时<br/>• <b>建议:</b> 请尝试重新提问，或换一种表达方式。",
                    )
                    .await;
            }
        }
    }
}

/// Renders a reply into the room: audio first, the caption after it (the order
/// a voice reply has always had), in place of the stage indicator when there is
/// one; then records the reaction targets the reply and the turn created.
async fn show_reply(
    room: &Room,
    ev: &OriginalSyncRoomMessageEvent,
    parsed: &ParsedCommand,
    reply: Reply,
    stage_indicator: Option<StageIndicator>,
    db: &Database,
    services: &AppServices,
) {
    let reply = deliver_audio(room, reply).await;
    let reply_html = reply.html_body();
    let (sent_eid, ph_eid) = if let Some(indicator) = stage_indicator {
        let ev_id = indicator
            .finish_with_reply(&reply.content.plain, reply_html)
            .await;
        (ev_id, indicator.event_id().cloned())
    } else {
        (send_html(room, reply_html).await, None)
    };

    if let Some(ref eid) = sent_eid {
        // The owner is only needed when the reply names a created entity; the
        // pipeline does not hand it back, so look it up for that case alone.
        if parse_entity_from_reply(reply_html).is_some() {
            match services.identity.find_account(&parsed.sender).await {
                Ok(Some(owner_id)) => {
                    try_record_from_reply(
                        &db.pool,
                        eid.as_str(),
                        room.room_id().as_str(),
                        &owner_id,
                        reply_html,
                    )
                    .await;
                }
                Ok(None) => {}
                Err(err) => error!(
                    "Could not resolve the owner of {} to record a reaction target: {:?}",
                    parsed.sender.user_id().as_str(),
                    err
                ),
            }
        }
    }

    link_turn_targets(
        &db.pool,
        ph_eid.as_deref().map(|e| e.as_str()),
        sent_eid.as_deref().map(|e| e.as_str()),
        ev.event_id.as_str(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::{Channel, ConversationAddress};
    use crate::interface::channels::inbound::InboundText;
    use crate::interface::channels::message::MessageRef;
    use matrix_sdk::ruma::events::room::message::{MessageType, OriginalSyncRoomMessageEvent};
    use matrix_sdk::ruma::UserId;

    /// The wire shape of a Matrix text event; `contracts/` pins it for every
    /// consumer so a matrix-sdk upgrade cannot silently change what we read.
    const FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/matrix_text_event.json"
    ));

    fn event() -> OriginalSyncRoomMessageEvent {
        serde_json::from_str(FIXTURE).expect("fixture is a Matrix text event")
    }

    fn convert(is_direct: bool, body: &str) -> (IncomingMessage, InboundText) {
        let ev = event();
        let own = UserId::parse("@bot:example.org").unwrap();
        let sender = matrix_sender(ev.sender.as_str()).unwrap();
        let parsed = CommandParser::parse_or_dm_prompt(
            body,
            None,
            &own,
            "!room:example.org",
            &sender,
            is_direct,
        )
        .expect("addressed to the bot");
        to_inbound(&ev, body, "!room:example.org", is_direct, &parsed)
    }

    #[test]
    fn a_text_event_becomes_a_neutral_incoming_message() {
        let ev = event();
        assert!(matches!(ev.content.msgtype, MessageType::Text(_)));

        let (msg, text) = convert(false, "!todo add milk");

        assert_eq!(msg.channel, Channel::Matrix);
        assert_eq!(
            msg.conversation,
            ConversationAddress::Matrix("!room:example.org".into())
        );
        assert_eq!(msg.sender.user_id().as_str(), "@alice:example.org");
        assert_eq!(msg.sender.display_name(), Some("alice"));
        assert_eq!(msg.message_id, MessageRef("$ev1:example.org".into()));
        assert_eq!(msg.reply_to, Some(MessageRef("$orig:example.org".into())));
        assert_eq!(msg.text, "!todo add milk");
        assert!(!msg.is_direct);
        assert!(!msg.is_mentioned);
        match text {
            InboundText::Command {
                command, raw_args, ..
            } => {
                assert_eq!(command, "todo");
                assert_eq!(raw_args, "add milk");
            }
            InboundText::FreeText(_) => panic!("a `!` message is a command"),
        }
    }

    /// Matrix's own rule stays Matrix's: a direct message that is no command
    /// reaches the pipeline already as the `ai` command, so the shared
    /// free-text rule never changes what Matrix did.
    #[test]
    fn a_direct_message_prompt_is_the_ai_command() {
        let (msg, text) = convert(true, "what is due today?");
        assert!(msg.is_direct);
        match text {
            InboundText::Command {
                command, raw_args, ..
            } => {
                assert_eq!(command, "ai");
                assert_eq!(raw_args, "what is due today?");
            }
            InboundText::FreeText(_) => panic!("the dispatcher already chose `ai`"),
        }
    }

    #[test]
    fn a_sender_that_is_no_mxid_is_refused() {
        assert!(matrix_sender("not-an-mxid").is_err());
    }
}
