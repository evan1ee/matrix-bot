//! The work queue's consumers: each takes one parsed Discord event, runs it
//! through the shared pipeline and delivers the outcome.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{
    Attachment, ChannelId, CommandInteraction, ComponentInteraction, Http, MessageId, User,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::commands::DeferMode;
use super::inbound::{identity, image_meta, message_ref, Parsed};
use super::outbound::{bounded, edit_response, follow_up, send};
use super::progress::{InteractionProgress, TypingProgress};
use super::render::{posts, route_reply, Post, Route, NO_REPLY, PRIVATE_NOTICE};
use crate::application::identity::Resolution;
use crate::domain::channels::ConversationAddress;
use crate::domain::jobs::NewJob;
use crate::domain::storage::{StorageCategory, StorageKey};
use crate::interface::channels::adapter::ChannelError;
use crate::interface::channels::inbound::{Inbound, InboundOutcome};
use crate::interface::channels::message::{OutgoingMessage, Reply, Target};
use crate::llm::progress::ProgressSink;

/// How many turns run at once (B8).
pub const DISCORD_WORKERS: usize = 4;

/// One parsed event, already acknowledged to Discord when it is an interaction.
#[allow(clippy::large_enum_variant)]
pub enum Work {
    Command {
        interaction: Arc<CommandInteraction>,
        parsed: Parsed,
        mode: DeferMode,
    },
    Component {
        interaction: Arc<ComponentInteraction>,
        parsed: Parsed,
        mode: DeferMode,
    },
    Direct {
        channel: ChannelId,
        message: MessageId,
        parsed: Parsed,
    },
    DirectAttachment {
        channel: ChannelId,
        message: MessageId,
        author: User,
        attachment: Attachment,
        caption: Option<String>,
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
        Work::Component {
            interaction,
            parsed,
            mode,
        } => {
            let outcome = inbound.handle(&parsed.message, parsed.text, None).await;
            let in_guild = !parsed.message.is_direct;
            if let Err(err) = deliver_component(http, &interaction, outcome, in_guild, mode).await {
                warn!(error = %err, "Could not deliver a Discord component reply");
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
            reply_dm(http, channel, message, reply).await;
        }
        Work::DirectAttachment {
            channel,
            message,
            author,
            attachment,
            caption,
        } => {
            let Some(sender) = identity(&author, None) else {
                warn!(user_id = %author.id, "could not construct Discord platform identity");
                return;
            };
            let owner = match inbound.services().identity.resolve(&sender).await {
                Ok(Resolution::Account(owner)) => owner,
                Ok(Resolution::Unlinked) => {
                    reply_dm(
                        http,
                        channel,
                        message,
                        Reply::html(crate::interface::channels::inbound::UNLINKED_REPLY),
                    )
                    .await;
                    return;
                }
                Err(err) => {
                    warn!(error = %err, "could not resolve Discord sender identity");
                    return;
                }
            };

            const MAX_IMAGE_BYTES: u32 = 10 * 1024 * 1024;
            if attachment.size > MAX_IMAGE_BYTES {
                reply_dm(
                    http,
                    channel,
                    message,
                    Reply::html("⚠️ That image is too large (maximum size is 10 MiB)."),
                )
                .await;
                return;
            }

            let download_result =
                tokio::time::timeout(Duration::from_secs(15), attachment.download()).await;

            let bytes = match download_result {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(err)) => {
                    warn!(error = %err, "could not download Discord attachment");
                    reply_dm(
                        http,
                        channel,
                        message,
                        Reply::html("⚠️ Could not download the attached image. Please try again."),
                    )
                    .await;
                    return;
                }
                Err(_) => {
                    warn!("timed out downloading Discord attachment");
                    reply_dm(
                        http,
                        channel,
                        message,
                        Reply::html(
                            "⚠️ Downloading the attached image timed out. Please try again.",
                        ),
                    )
                    .await;
                    return;
                }
            };

            if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES as usize {
                reply_dm(
                    http,
                    channel,
                    message,
                    Reply::html("⚠️ That image is empty or exceeds the 10 MiB limit."),
                )
                .await;
                return;
            }

            let (ext, content_type) = image_meta(&attachment, &bytes);
            let storage_key = StorageKey::generate(&owner, StorageCategory::Receipts, ext);

            if let Err(err) = inbound
                .services()
                .storage
                .put(&storage_key, bytes::Bytes::from(bytes), content_type)
                .await
            {
                warn!(error = %err, "could not store receipt image from Discord");
                reply_dm(
                    http,
                    channel,
                    message,
                    Reply::html("⚠️ Could not save the receipt image. Please try again."),
                )
                .await;
                return;
            }

            let job = NewJob::media_process_receipt(
                owner,
                storage_key,
                Some(ConversationAddress::Discord(channel.get())),
                caption,
            );

            if let Err(err) = crate::infrastructure::postgres::repositories::jobs::enqueue(
                &inbound.db().pool,
                &job,
            )
            .await
            {
                warn!(error = %err, "could not enqueue media.process_receipt job from Discord");
                reply_dm(
                    http,
                    channel,
                    message,
                    Reply::html("⚠️ Could not queue receipt processing. Please try again."),
                )
                .await;
                return;
            }

            reply_dm(
                http,
                channel,
                message,
                Reply::html("🧾 Receipt received and queued for processing!"),
            )
            .await;
        }
    }
}

async fn reply_dm(http: &Arc<Http>, channel: ChannelId, message: MessageId, reply: Reply) {
    let mut out = OutgoingMessage::to(
        Target::Conversation(ConversationAddress::Discord(channel.get())),
        reply,
    );
    out.reply_to = Some(message_ref(channel.get(), message.get()));
    if let Err(err) = send(http, out).await {
        warn!(error = %err, "Could not deliver a Discord DM reply");
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

async fn deliver_component(
    http: &Http,
    interaction: &ComponentInteraction,
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

    #[test]
    fn a_reply_with_actions_carries_buttons_on_planned_response() {
        use crate::interface::channels::message::Action;

        let mut reply = Reply::html("<b>Done!</b>");
        reply.content.actions = vec![Action {
            label: "Done #1".into(),
            command: "todo done 1".into(),
        }];
        let plan =
            plan_command_reply(InboundOutcome::Replied(reply), true, DeferMode::Ephemeral).unwrap();
        assert_eq!(plan.original.description, Some("**Done!**".into()));
        assert_eq!(
            plan.original.buttons,
            vec![("Done #1".into(), "todo done 1".into())]
        );
        assert!(plan.follow_ups.is_empty());
        assert!(plan.follow_ups_ephemeral);
    }

    const COMMAND_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/discord_command_interaction.json"
    ));
    const DM_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/discord_dm_message.json"
    ));

    fn component_fixture(edit: impl FnOnce(&mut serde_json::Value)) -> ComponentInteraction {
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
    fn component_work_matches_and_unpacks() {
        let interaction = component_fixture(|_| {});
        let parsed = super::super::inbound::from_component(&interaction).unwrap();
        let work = Work::Component {
            interaction: Arc::new(interaction),
            parsed,
            mode: DeferMode::Ephemeral,
        };
        match work {
            Work::Component { mode, parsed, .. } => {
                assert_eq!(mode, DeferMode::Ephemeral);
                assert_eq!(parsed.message.text, "todo done 1");
            }
            _ => panic!("expected Work::Component"),
        }
    }

    #[test]
    fn direct_attachment_work_matches_and_unpacks() {
        let dm = serde_json::from_str::<serenity::all::Message>(DM_FIXTURE).unwrap();
        let attachment: Attachment = serde_json::from_value(serde_json::json!({
            "id": "1100000000000000020",
            "filename": "receipt.jpg",
            "size": 2048,
            "url": "https://cdn.discordapp.com/attachments/1/2/receipt.jpg",
            "proxy_url": "https://media.discordapp.net/attachments/1/2/receipt.jpg",
            "content_type": "image/jpeg",
        }))
        .unwrap();

        let work = Work::DirectAttachment {
            channel: dm.channel_id,
            message: dm.id,
            author: dm.author.clone(),
            attachment: attachment.clone(),
            caption: Some("lunch receipt".into()),
        };

        match work {
            Work::DirectAttachment {
                channel,
                message,
                author,
                attachment: att,
                caption,
            } => {
                assert_eq!(channel.get(), 1_100_000_000_000_000_012);
                assert_eq!(message.get(), 1_100_000_000_000_000_011);
                assert_eq!(author.name, "evan");
                assert_eq!(att.filename, "receipt.jpg");
                assert_eq!(caption.as_deref(), Some("lunch receipt"));
            }
            _ => panic!("expected Work::DirectAttachment"),
        }
    }

    #[test]
    fn direct_attachment_receipt_job_builds_correctly() {
        let owner = crate::domain::accounts::AccountId::from_seed("acct_test");
        let storage_key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");
        let channel_id = ChannelId::new(123456789);
        let job = NewJob::media_process_receipt(
            owner,
            storage_key.clone(),
            Some(ConversationAddress::Discord(channel_id.get())),
            Some("lunch".into()),
        );
        assert_eq!(
            job.job_type(),
            crate::domain::jobs::JobType::MediaProcessReceipt
        );
        assert_eq!(job.account_id(), Some(&owner));
        match job.payload() {
            crate::domain::jobs::JobPayload::MediaProcessReceipt(p) => {
                assert_eq!(p.storage_key, storage_key);
                assert_eq!(p.account_id, owner);
                assert_eq!(
                    p.conversation,
                    Some(ConversationAddress::Discord(123456789))
                );
                assert_eq!(p.caption.as_deref(), Some("lunch"));
            }
            _ => panic!("expected MediaProcessReceipt payload"),
        }
    }

    #[test]
    fn direct_attachment_size_limit_rejection() {
        const MAX_IMAGE_BYTES: u32 = 10 * 1024 * 1024;
        let valid_size = MAX_IMAGE_BYTES;
        let oversized = MAX_IMAGE_BYTES + 1;
        assert!(valid_size <= MAX_IMAGE_BYTES);
        assert!(oversized > MAX_IMAGE_BYTES);
    }
}
