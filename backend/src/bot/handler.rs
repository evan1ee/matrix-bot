use matrix_sdk::{
    room::Room,
    ruma::events::room::{
        member::{MembershipState, StrippedRoomMemberEvent},
        message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent},
    },
    Client, RoomState,
};
use std::sync::Arc;
use tracing::{error, info};

use crate::bot::dispatcher::CommandParser;
use crate::db::Database;
use crate::modules::ModuleRegistry;

pub fn register_handlers(
    client: &Client,
    registry: Arc<ModuleRegistry>,
    db: Database,
) {
    // 1. Auto-join invite event handler
    client.add_event_handler(|ev: StrippedRoomMemberEvent, room: Room| async move {
        if ev.content.membership == MembershipState::Invite {
            info!("Received invitation for room: {}", room.room_id());
            if let Err(err) = room.join().await {
                error!("Failed to join room {}: {:?}", room.room_id(), err);
            } else {
                info!("Successfully joined room: {}", room.room_id());
            }
        }
    });

    // 2. Incoming message event handler
    let bot_user_id = client.user_id().unwrap().to_owned();
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        let own_id = bot_user_id.clone();
        let registry = registry.clone();
        let db = db.clone();

        async move {
            if room.state() != RoomState::Joined || ev.sender == own_id {
                return;
            }

            let MessageType::Text(ref text_content) = ev.content.msgtype else {
                return;
            };

            let formatted_body = text_content.formatted.as_ref().map(|f| f.body.as_str());

            if let Some(cmd_ctx) = CommandParser::parse_message(
                &text_content.body,
                formatted_body,
                &own_id,
                room.room_id().as_str(),
                ev.sender.as_str(),
                ev.sender.localpart(),
            ) {
                info!(
                    "Command received in [{}] from [{}]: command='{}', args='{}'",
                    cmd_ctx.room_id, cmd_ctx.sender_name, cmd_ctx.command, cmd_ctx.raw_args
                );

                if let Some(reply_html) = registry.dispatch(&cmd_ctx, &db).await {
                    let plain_text = crate::utils::strip_html(&reply_html);
                    let content = RoomMessageEventContent::text_html(&plain_text, &reply_html);
                    if let Err(err) = room.send(content).await {
                        error!("Failed to send message to room {}: {:?}", room.room_id(), err);
                    }
                }
            }
        }
    });
}
