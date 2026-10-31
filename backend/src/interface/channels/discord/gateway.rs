//! serenity's event handler. It does no business work: it parses, acknowledges
//! interactions within Discord's three seconds, and queues the rest (B8).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serenity::all::{
    Command, ConnectionStage, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    EventHandler, GuildId, Interaction, Message, Ready, ShardStageUpdateEvent,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::audio::is_audio_attachment;
use super::commands::{create_commands, defer_mode, DeferMode};
use super::inbound::{from_command, from_component, from_direct_message, is_image_attachment};
use super::outbound::{bounded, edit_response};
use super::render::Post;
use super::worker::Work;
use crate::interface::channels::inbound::InboundText;

/// How many events may wait for a worker before the bot answers "busy".
pub const QUEUE_CAPACITY: usize = 32;
pub const BUSY_REPLY: &str = "I'm busy right now; try again in a minute.";

pub struct Gateway {
    pub queue: mpsc::Sender<Work>,
    pub connected: Arc<AtomicBool>,
    pub guild_id: Option<u64>,
}

#[async_trait::async_trait]
impl EventHandler for Gateway {
    async fn ready(&self, ctx: Context, _ready: Ready) {
        self.connected.store(true, Ordering::Relaxed);
        let commands = create_commands();
        let registered = match self.guild_id {
            Some(guild) => bounded(GuildId::new(guild).set_commands(&ctx.http, commands))
                .await
                .map(|c| c.len()),
            None => bounded(Command::set_global_commands(&ctx.http, commands))
                .await
                .map(|c| c.len()),
        };
        match registered {
            Ok(count) => info!(
                count,
                guild = self.guild_id.is_some(),
                "Discord commands registered"
            ),
            // Not fatal: the previous registration stays in force.
            Err(err) => warn!(error = %err, "Could not register the Discord commands"),
        }
    }

    async fn shard_stage_update(&self, _ctx: Context, event: ShardStageUpdateEvent) {
        self.connected
            .store(event.new == ConnectionStage::Connected, Ordering::Relaxed);
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) => {
                let parsed = match from_command(&command) {
                    Ok(parsed) => parsed,
                    Err(refusal) => {
                        let answer = CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(refusal.reply_text())
                                .ephemeral(true),
                        );
                        if let Err(err) = bounded(command.create_response(&ctx.http, answer)).await
                        {
                            warn!(error = %err, "Could not refuse a Discord command");
                        }
                        return;
                    }
                };
                let mode = defer_mode(&command.data.name, command.guild_id.is_some());
                let deferred = match mode {
                    DeferMode::Ephemeral => bounded(command.defer_ephemeral(&ctx.http)).await,
                    DeferMode::Public => bounded(command.defer(&ctx.http)).await,
                };
                if let Err(err) = deferred {
                    warn!(error = %err, "Could not acknowledge a Discord command; dropped");
                    return;
                }
                let command = Arc::new(command);
                let work = Work::Command {
                    interaction: Arc::clone(&command),
                    parsed,
                    mode,
                };
                if self.queue.try_send(work).is_err() {
                    let busy =
                        command.edit_response(&ctx.http, edit_response(Post::text(BUSY_REPLY)));
                    if let Err(err) = bounded(busy).await {
                        warn!(error = %err, "Could not tell a Discord user the bot is busy");
                    }
                }
            }
            Interaction::Component(component) => {
                let parsed = match from_component(&component) {
                    Ok(parsed) => parsed,
                    Err(refusal) => {
                        let answer = CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(refusal.reply_text())
                                .ephemeral(true),
                        );
                        if let Err(err) =
                            bounded(component.create_response(&ctx.http, answer)).await
                        {
                            warn!(
                                error = %err,
                                "Could not refuse a Discord component interaction"
                            );
                        }
                        return;
                    }
                };
                let command_name = match &parsed.text {
                    InboundText::Command { command, .. } => command.as_str(),
                    InboundText::FreeText(_) => parsed.message.text.as_str(),
                };
                let mode = defer_mode(command_name, component.guild_id.is_some());
                let deferred = match mode {
                    DeferMode::Ephemeral => bounded(component.defer_ephemeral(&ctx.http)).await,
                    DeferMode::Public => bounded(component.defer(&ctx.http)).await,
                };
                if let Err(err) = deferred {
                    warn!(
                        error = %err,
                        "Could not acknowledge a Discord component interaction; dropped"
                    );
                    return;
                }
                let component = Arc::new(component);
                let work = Work::Component {
                    interaction: Arc::clone(&component),
                    parsed,
                    mode,
                };
                if self.queue.try_send(work).is_err() {
                    let busy =
                        component.edit_response(&ctx.http, edit_response(Post::text(BUSY_REPLY)));
                    if let Err(err) = bounded(busy).await {
                        warn!(error = %err, "Could not tell a Discord user the bot is busy");
                    }
                }
            }
            _ => {}
        }
    }

    async fn message(&self, _ctx: Context, message: Message) {
        if message.author.bot || message.guild_id.is_some() {
            return;
        }
        let caption = Some(message.content.trim())
            .filter(|c| !c.is_empty())
            .map(str::to_owned);
        // Audio first: a voice message is never a receipt.
        if let Some(attachment) = message.attachments.iter().find(|a| is_audio_attachment(a)) {
            let work = Work::DirectAudio {
                channel: message.channel_id,
                message: message.id,
                author: message.author.clone(),
                attachment: attachment.clone(),
                caption,
            };
            if self.queue.try_send(work).is_err() {
                warn!("Discord work queue full; a direct voice message was dropped");
            }
            return;
        }
        if let Some(attachment) = message.attachments.iter().find(|a| is_image_attachment(a)) {
            let work = Work::DirectAttachment {
                channel: message.channel_id,
                message: message.id,
                author: message.author.clone(),
                attachment: attachment.clone(),
                caption,
            };
            if self.queue.try_send(work).is_err() {
                warn!("Discord work queue full; a direct attachment was dropped");
            }
            return;
        }
        let Some(parsed) = from_direct_message(&message) else {
            return;
        };
        let work = Work::Direct {
            channel: message.channel_id,
            message: message.id,
            parsed,
        };
        if self.queue.try_send(work).is_err() {
            warn!("Discord work queue full; a direct message was dropped");
        }
    }
}
