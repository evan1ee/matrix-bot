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
use crate::events::EventBroadcaster;
use crate::modules::ModuleRegistry;

pub fn register_handlers(
    client: &Client,
    registry: Arc<ModuleRegistry>,
    db: Database,
    broadcaster: Option<EventBroadcaster>,
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
        let broadcaster = broadcaster.clone();

        async move {
            if room.state() != RoomState::Joined || ev.sender == own_id {
                return;
            }

            let MessageType::Text(ref text_content) = ev.content.msgtype else {
                return;
            };

            let formatted_body = text_content.formatted.as_ref().map(|f| f.body.as_str());

            if let Some(parsed) = CommandParser::parse_message(
                &text_content.body,
                formatted_body,
                &own_id,
                room.room_id().as_str(),
                ev.sender.localpart(),
            ) {
                info!(
                    "Command received in [{}] from [{}]: command='{}', args='{}'",
                    parsed.room_id, parsed.sender_name, parsed.command, parsed.raw_args
                );

                // Resolve the sender to the account that will own whatever this
                // command creates, provisioning one if this MXID has never been
                // seen. Deliberately after the parse: an unrelated room message
                // must not cost a query, and must not create an account for
                // someone who only ever said hello to a room the bot is in.
                let owner_id = match crate::accounts::AccountRepository::resolve_or_provision(
                    &db.pool,
                    ev.sender.as_str(),
                )
                .await
                {
                    Ok(owner_id) => owner_id,
                    Err(err) => {
                        error!(
                            "Could not resolve an account for {}: {:?} — dropping the command",
                            ev.sender, err
                        );
                        return;
                    }
                };

                let cmd_ctx = parsed.for_owner(owner_id);

                if let Some(reply_html) = registry.dispatch(&cmd_ctx, &db, broadcaster.as_ref()).await {
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
