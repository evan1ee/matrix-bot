//! The work queue's consumers: each takes one parsed Discord event, runs it
//! through the shared pipeline and delivers the outcome.

use std::sync::Arc;

use serenity::all::{ChannelId, CommandInteraction, Http, MessageId};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::commands::DeferMode;
use super::inbound::{message_ref, Parsed};
use super::outbound::{bounded, edit_response, follow_up, send};
use super::progress::{InteractionProgress, TypingProgress};
use super::render::{posts, route_reply, Post, Route, NO_REPLY, PRIVATE_NOTICE};
use crate::domain::channels::ConversationAddress;
use crate::interface::channels::adapter::ChannelError;
use crate::interface::channels::inbound::{Inbound, InboundOutcome};
use crate::interface::channels::message::{OutgoingMessage, Target};
use crate::llm::progress::ProgressSink;

/// How many turns run at once (B8).
pub const DISCORD_WORKERS: usize = 4;

/// One parsed event, already acknowledged to Discord when it is an interaction.
pub enum Work {
    Command {
        interaction: Arc<CommandInteraction>,
        parsed: Parsed,
        mode: DeferMode,
    },
    Direct {
        channel: ChannelId,
        message: MessageId,
        parsed: Parsed,
    },
}

/// How an interaction's outcome is shown.
#[derive(Debug, PartialEq)]
pub struct CommandReplyPlan {
    pub original: Post,
    pub follow_ups: Vec<Post>,
    pub follow_ups_ephemeral: bool,
}

/// Decides, without serenity, what the deferred response and its follow-ups say.
pub fn plan_command_reply(
    outcome: InboundOutcome,
    in_guild: bool,
    mode: DeferMode,
) -> Result<CommandReplyPlan, ChannelError> {
    let reply = match outcome {
        InboundOutcome::Replied(reply) => reply,
        // A deferred interaction shows "thinking…" until edited (B11).
        InboundOutcome::Ignored => {
            return Ok(CommandReplyPlan {
                original: Post::text(NO_REPLY),
                follow_ups: Vec::new(),
                follow_ups_ephemeral: mode == DeferMode::Ephemeral,
            })
        }
    };
    let mut rendered = posts(&reply.content)?.into_iter();
    Ok(match route_reply(reply.visibility, in_guild, mode) {
        Route::EditOriginal => CommandReplyPlan {
            original: rendered.next().unwrap_or_else(|| Post::text(NO_REPLY)),
            follow_ups: rendered.collect(),
            follow_ups_ephemeral: mode == DeferMode::Ephemeral,
        },
        Route::PrivateFollowUp => CommandReplyPlan {
            original: Post::text(PRIVATE_NOTICE),
            follow_ups: rendered.collect(),
            follow_ups_ephemeral: true,
        },
    })
}

/// Drains the queue until it closes or `cancel` fires. Delivery failures are
/// logged (never a token or a body) and not retried: an interaction token lives
/// fifteen minutes and a turn is not idempotent.
pub async fn run_worker(
    queue: Arc<Mutex<mpsc::Receiver<Work>>>,
    inbound: Arc<Inbound>,
    http: Arc<Http>,
    cancel: CancellationToken,
) {
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return,
            work = async { queue.lock().await.recv().await } => work,
        };
        let Some(work) = next else {
            return;
        };
        tokio::select! {
            // An in-flight turn is dropped on shutdown, as a Matrix turn is.
            _ = cancel.cancelled() => return,
            _ = handle(work, &inbound, &http) => {}
        }
    }
}

async fn handle(work: Work, inbound: &Inbound, http: &Arc<Http>) {
    match work {
        Work::Command {
            interaction,
            parsed,
            mode,
        } => {
            let progress: Arc<dyn ProgressSink> = Arc::new(InteractionProgress::new(
                Arc::clone(http),
                Arc::clone(&interaction),
            ));
            let outcome = inbound
                .handle(&parsed.message, parsed.text, Some(progress))
                .await;
            let in_guild = !parsed.message.is_direct;
            if let Err(err) = deliver_command(http, &interaction, outcome, in_guild, mode).await {
                warn!(error = %err, "Could not deliver a Discord command reply");
            }
        }
        Work::Direct {
            channel,
            message,
            parsed,
        } => {
            let progress: Arc<dyn ProgressSink> =
                Arc::new(TypingProgress::new(Arc::clone(http), channel));
            let outcome = inbound
                .handle(&parsed.message, parsed.text, Some(progress))
                .await;
            let InboundOutcome::Replied(reply) = outcome else {
                return;
            };
            let mut out = OutgoingMessage::to(
                Target::Conversation(ConversationAddress::Discord(channel.get())),
                reply,
            );
            out.reply_to = Some(message_ref(channel.get(), message.get()));
            if let Err(err) = send(http, out).await {
                warn!(error = %err, "Could not deliver a Discord DM reply");
            }
        }
    }
}

async fn deliver_command(
    http: &Http,
    interaction: &CommandInteraction,
    outcome: InboundOutcome,
    in_guild: bool,
    mode: DeferMode,
) -> Result<(), ChannelError> {
    let plan = match plan_command_reply(outcome, in_guild, mode) {
        Ok(plan) => plan,
        // A reply Discord can never carry (an oversized file) still resolves the
        // interaction, with the reason instead of a spinner.
        Err(err) => CommandReplyPlan {
            original: Post::text(format!("That reply could not be sent here ({err}).")),
            follow_ups: Vec::new(),
            follow_ups_ephemeral: false,
        },
    };
    bounded(interaction.edit_response(http, edit_response(plan.original))).await?;
    for post in plan.follow_ups {
        bounded(interaction.create_followup(http, follow_up(post, plan.follow_ups_ephemeral)))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::message::Reply;

    #[test]
    fn an_ignored_interaction_still_gets_an_answer() {
        let plan = plan_command_reply(InboundOutcome::Ignored, true, DeferMode::Ephemeral).unwrap();
        assert_eq!(plan.original, Post::text(NO_REPLY));
        assert!(plan.follow_ups.is_empty());
    }

    #[test]
    fn a_private_reply_on_a_public_guild_response_goes_out_ephemerally() {
        let reply = Reply::private("<b>balance</b> 42");
        let plan =
            plan_command_reply(InboundOutcome::Replied(reply), true, DeferMode::Public).unwrap();
        assert_eq!(plan.original, Post::text(PRIVATE_NOTICE));
        assert_eq!(plan.follow_ups, vec![Post::text("**balance** 42")]);
        assert!(plan.follow_ups_ephemeral);
    }

    #[test]
    fn a_long_reply_edits_the_first_chunk_and_follows_up_with_the_rest() {
        let html = vec!["line of text"; 1000].join("<br/>");
        for mode in [DeferMode::Public, DeferMode::Ephemeral] {
            let plan = plan_command_reply(
                InboundOutcome::Replied(Reply::html(html.clone())),
                true,
                mode,
            )
            .unwrap();
            assert!(plan.original.description.is_some());
            assert!(!plan.follow_ups.is_empty());
            assert_eq!(plan.follow_ups_ephemeral, mode == DeferMode::Ephemeral);
        }
    }

    #[test]
    fn a_private_reply_in_a_dm_edits_the_original() {
        let plan = plan_command_reply(
            InboundOutcome::Replied(Reply::private("x")),
            false,
            DeferMode::Public,
        )
        .unwrap();
        assert_eq!(plan.original, Post::text("x"));
    }
}
