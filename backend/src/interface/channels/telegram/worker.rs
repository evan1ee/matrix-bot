//! The queue's consumers: each takes one raw update, parses and de-duplicates
//! it, runs it through the shared pipeline and delivers the outcome.

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use teloxide_core::prelude::*;
use teloxide_core::types::{ChatId, Update};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::audio::{audio_of, TelegramAudio};
use super::commands::CommandRefusal;
use super::inbound::{normalize, parse_message_ref, Normalized, UpdateKind};
use super::ingress::RecentUpdates;
use super::outbound::{bounded, deliver};
use super::progress::TelegramProgress;
use super::render::{plan, Plan};
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::inbound::{Inbound, InboundOutcome, InboundText};
use crate::interface::channels::message::IncomingMessage;
use crate::llm::progress::ProgressSink;

/// How many updates run at once (T11).
pub const TELEGRAM_WORKERS: usize = 4;

/// What the pipeline's answer says to send: nothing for `Ignored`, otherwise
/// the rendered plan (a `Private` reply in a group is only the notice, T8).
pub fn plan_reply(outcome: InboundOutcome, in_group: bool) -> Result<Option<Plan>, ChannelError> {
    match outcome {
        InboundOutcome::Ignored => Ok(None),
        InboundOutcome::Replied(reply) => {
            plan(&reply.content, reply.visibility, in_group).map(Some)
        }
    }
}

/// T4: the `help` a linked user's `start` becomes. Deep-link payloads are ignored.
pub fn help_for_start(text: &InboundText) -> Option<InboundText> {
    match text {
        InboundText::Command { command, .. } if command == "start" => Some(InboundText::Command {
            command: "help".into(),
            raw_args: String::new(),
            full_body: "help".into(),
        }),
        _ => None,
    }
}

/// Only turns that may call the agent show progress.
pub fn wants_progress(text: &InboundText) -> bool {
    match text {
        InboundText::FreeText(_) => true,
        InboundText::Command { command, .. } => command == "ai",
    }
}

/// One raw update, parsed.
pub enum Prepared {
    /// Malformed, duplicate, not addressed to us or not an update we handle.
    Drop,
    /// A command the adapter itself refuses (too long): answer where it came from.
    Refused {
        kind: UpdateKind,
        text: String,
    },
    Dispatch(Box<Normalized>),
    /// A voice note or audio file in a private chat.
    Audio(Box<TelegramAudio>),
}

/// Deserializes with `from_slice` (never `from_value`: that silently yields
/// `UpdateKind::Error`), de-duplicates on `update_id`, normalizes. The body is
/// never logged.
pub fn prepare(body: &[u8], recent: &Mutex<RecentUpdates>, username: &str) -> Prepared {
    let update: Update = match serde_json::from_slice(body) {
        Ok(update) => update,
        Err(err) => {
            warn!(error = %err, "Dropped a malformed Telegram update");
            return Prepared::Drop;
        }
    };
    let first = recent
        .lock()
        .map(|mut recent| recent.first_time(update.id.0))
        // A poisoned set is a bug elsewhere; processing twice beats dropping.
        .unwrap_or(true);
    if !first {
        debug!(
            update_id = update.id.0,
            "Dropped a duplicate Telegram update"
        );
        return Prepared::Drop;
    }
    if let Some(audio) = audio_of(&update) {
        return Prepared::Audio(Box::new(audio));
    }
    match normalize(&update, username) {
        None => Prepared::Drop,
        Some(Ok(normalized)) => Prepared::Dispatch(Box::new(normalized)),
        Some(Err((kind, refusal))) => Prepared::Refused {
            kind,
            text: CommandRefusal::reply_text(&refusal),
        },
    }
}

/// The pipeline call, with T4: a linked `start` that nobody claims (`Ignored`)
/// is dispatched once more as `help`.
pub async fn dispatch(
    inbound: &Inbound,
    message: &IncomingMessage,
    text: InboundText,
    progress: Option<Arc<dyn ProgressSink>>,
) -> InboundOutcome {
    let help = help_for_start(&text);
    let outcome = inbound.handle(message, text, progress).await;
    match (outcome, help) {
        (InboundOutcome::Ignored, Some(help)) => inbound.handle(message, help, None).await,
        (outcome, _) => outcome,
    }
}

/// What every worker needs.
#[derive(Clone)]
pub(super) struct WorkerCtx {
    pub inbound: Arc<Inbound>,
    pub bot: Bot,
    pub username: Arc<str>,
    pub recent: Arc<Mutex<RecentUpdates>>,
}

/// Drains the queue until `cancel` fires. Delivery failures are logged (never a
/// token or a body) and not retried: a turn is not idempotent.
pub(super) async fn run_worker(
    rx: Arc<AsyncMutex<mpsc::Receiver<Bytes>>>,
    ctx: WorkerCtx,
    cancel: CancellationToken,
) {
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return,
            body = async { rx.lock().await.recv().await } => body,
        };
        let Some(body) = next else {
            return;
        };
        tokio::select! {
            // An in-flight turn is dropped on shutdown, as a Matrix turn is.
            _ = cancel.cancelled() => return,
            _ = handle(&body, &ctx) => {}
        }
    }
}

async fn handle(body: &[u8], ctx: &WorkerCtx) {
    match prepare(body, &ctx.recent, &ctx.username) {
        Prepared::Drop => {}
        Prepared::Refused { kind, text } => {
            acknowledge(&ctx.bot, &kind).await;
            post(&ctx.bot, &kind, Plan::text(text)).await;
        }
        Prepared::Dispatch(n) => run_turn(ctx, *n).await,
        Prepared::Audio(audio) => super::audio::run(ctx, *audio).await,
    }
}

async fn run_turn(ctx: &WorkerCtx, n: Normalized) {
    // T6: stop the client's spinner at once, before the (possibly slow) turn.
    acknowledge(&ctx.bot, &n.kind).await;
    let chat = ChatId(chat_of(&n.kind));
    let in_group = !n.message.is_direct;
    let progress =
        wants_progress(&n.text).then(|| Arc::new(TelegramProgress::new(ctx.bot.clone(), chat)));
    let sink = progress.clone().map(|p| p as Arc<dyn ProgressSink>);
    let outcome = dispatch(&ctx.inbound, &n.message, n.text, sink).await;
    let (is_ai, owner_id) = match &outcome {
        InboundOutcome::Replied(reply) => (reply.is_ai, reply.owner_id),
        _ => (false, None),
    };
    match plan_reply(outcome, in_group) {
        Ok(Some(plan)) => {
            if let Some(sent) = post(&ctx.bot, &n.kind, plan).await {
                if is_ai {
                    if let Some(owner) = owner_id {
                        if let Some(ref m_ref) = sent.message {
                            if let Some((_chat, msg_id)) = parse_message_ref(m_ref) {
                                let chat_key = n.message.conversation.to_storage();
                                ctx.inbound.services().llm.record_authentic_reply(
                                    &chat_key,
                                    &msg_id.to_string(),
                                    &owner,
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(None) => {}
        // A reply Telegram can never carry still gets an answer.
        Err(err) => {
            warn!(error = %err, "A Telegram reply could not be planned");
            let notice = format!("That reply could not be sent here ({err}).");
            post(&ctx.bot, &n.kind, Plan::text(notice)).await;
        }
    }
    if let Some(progress) = progress {
        progress.finish().await;
    }
}

fn chat_of(kind: &UpdateKind) -> i64 {
    match kind {
        UpdateKind::Message { chat_id, .. } | UpdateKind::Callback { chat_id, .. } => *chat_id,
    }
}

fn message_of(kind: &UpdateKind) -> i32 {
    match kind {
        UpdateKind::Message { message_id, .. } | UpdateKind::Callback { message_id, .. } => {
            *message_id
        }
    }
}

/// `answerCallbackQuery` with no text; a no-op for a plain message.
async fn acknowledge(bot: &Bot, kind: &UpdateKind) {
    if let UpdateKind::Callback { query_id, .. } = kind {
        let answer = bot.answer_callback_query(query_id.clone().into());
        if let Err(err) = bounded(answer.send()).await {
            debug!(error = %err, "Could not answer a Telegram callback query");
        }
    }
}

/// Delivers `plan` as a reply to the triggering message.
pub(super) async fn post(bot: &Bot, kind: &UpdateKind, plan: Plan) -> Option<SentRef> {
    let chat = ChatId(chat_of(kind));
    match deliver(bot, chat, Some(message_of(kind)), plan).await {
        Ok(sent) => Some(sent),
        Err(err) => {
            warn!(error = %err, "Could not deliver a Telegram reply");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::message::Reply;
    use crate::interface::channels::telegram::render::{Part, PRIVATE_IN_GROUP};

    const PRIVATE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_private_text.json"
    ));

    fn recent() -> Mutex<RecentUpdates> {
        Mutex::new(RecentUpdates::new(8))
    }

    #[test]
    fn four_workers_drain_the_queue() {
        assert_eq!(TELEGRAM_WORKERS, 4);
    }

    #[test]
    fn an_ignored_outcome_sends_nothing() {
        assert!(plan_reply(InboundOutcome::Ignored, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_reply_becomes_text_parts() {
        let plan = plan_reply(InboundOutcome::Replied(Reply::html("<b>hi</b>")), false)
            .unwrap()
            .unwrap();
        assert!(matches!(plan.parts.as_slice(), [Part::Text(t)] if t == "<b>hi</b>"));
    }

    #[test]
    fn a_private_reply_in_a_group_becomes_the_notice() {
        let plan = plan_reply(InboundOutcome::Replied(Reply::private("balance 42")), true)
            .unwrap()
            .unwrap();
        assert!(matches!(plan.parts.as_slice(), [Part::Text(t)] if t == PRIVATE_IN_GROUP));
    }

    #[test]
    fn only_start_is_redispatched_as_help() {
        let start = InboundText::Command {
            command: "start".into(),
            raw_args: "payload".into(),
            full_body: "start payload".into(),
        };
        match help_for_start(&start) {
            Some(InboundText::Command {
                command,
                raw_args,
                full_body,
            }) => {
                assert_eq!(command, "help");
                assert_eq!(raw_args, "");
                assert_eq!(full_body, "help");
            }
            _ => panic!("expected help"),
        }
        let todo = InboundText::Command {
            command: "todo".into(),
            raw_args: String::new(),
            full_body: "todo".into(),
        };
        assert!(help_for_start(&todo).is_none());
        assert!(help_for_start(&InboundText::FreeText("start".into())).is_none());
    }

    #[test]
    fn a_duplicate_update_is_dropped() {
        let recent = recent();
        assert!(matches!(
            prepare(PRIVATE.as_bytes(), &recent, "MyBot"),
            Prepared::Dispatch(_)
        ));
        assert!(matches!(
            prepare(PRIVATE.as_bytes(), &recent, "MyBot"),
            Prepared::Drop
        ));
    }

    #[test]
    fn a_voice_note_is_prepared_as_audio_once() {
        let voice = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../contracts/channels/telegram_private_voice.json"
        ));
        let recent = recent();
        assert!(matches!(
            prepare(voice.as_bytes(), &recent, "MyBot"),
            Prepared::Audio(_)
        ));
        assert!(matches!(
            prepare(voice.as_bytes(), &recent, "MyBot"),
            Prepared::Drop
        ));
    }

    #[test]
    fn a_malformed_body_is_dropped() {
        assert!(matches!(
            prepare(b"{not json", &recent(), "MyBot"),
            Prepared::Drop
        ));
        assert!(matches!(prepare(b"", &recent(), "MyBot"), Prepared::Drop));
    }

    #[test]
    fn an_over_long_command_is_refused_with_its_reason() {
        let mut v: serde_json::Value = serde_json::from_str(PRIVATE).unwrap();
        v["message"]["text"] = format!("/todo {}", "x".repeat(600)).into();
        let body = v.to_string();
        match prepare(body.as_bytes(), &recent(), "MyBot") {
            Prepared::Refused { text, .. } => assert!(text.contains("too long"), "{text}"),
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn progress_is_for_free_text_and_the_ai_command_only() {
        let cmd = |c: &str| InboundText::Command {
            command: c.into(),
            raw_args: String::new(),
            full_body: c.into(),
        };
        assert!(wants_progress(&InboundText::FreeText("hi".into())));
        assert!(wants_progress(&cmd("ai")));
        assert!(!wants_progress(&cmd("todo")));
    }
}
