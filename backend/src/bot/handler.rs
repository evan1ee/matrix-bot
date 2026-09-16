use matrix_sdk::{
    room::Room,
    ruma::events::room::{
        member::{MembershipState, StrippedRoomMemberEvent},
        message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent},
    },
    Client, RoomMemberships, RoomState,
};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::bot::dispatcher::CommandParser;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{ModuleRegistry, ParsedCommand};

pub fn register_handlers(
    client: &Client,
    registry: Arc<ModuleRegistry>,
    db: Database,
    broadcaster: Option<EventBroadcaster>,
    pending_merges: Option<Arc<crate::bot::linking::PendingMerges>>,
) {
    // 1. Authorized invite event handler
    let invite_db = db.clone();
    let allowlist: Vec<String> = std::env::var("MATRIX_VERIFICATION_ALLOWLIST")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    client.add_event_handler(move |ev: StrippedRoomMemberEvent, room: Room| {
        let db = invite_db.clone();
        let allowlist = allowlist.clone();
        async move {
            if ev.content.membership == MembershipState::Invite {
                let sender = ev.sender.as_str();
                let is_allowlisted = allowlist.iter().any(|a| a == sender);
                let is_linked =
                    sqlx::query_scalar::<_, i64>("SELECT 1 FROM matrix_links WHERE mxid = ?")
                        .bind(sender)
                        .fetch_optional(&db.pool)
                        .await
                        .ok()
                        .flatten()
                        .is_some();

                if is_allowlisted || is_linked {
                    info!(
                        "Authorized invitation from {} for room: {}",
                        sender,
                        room.room_id()
                    );
                    if let Err(err) = room.join().await {
                        error!("Failed to join room {}: {:?}", room.room_id(), err);
                    } else {
                        info!("Successfully joined room: {}", room.room_id());
                    }
                } else {
                    warn!(
                        "Rejected unauthorized room invitation from {} for room: {}",
                        sender,
                        room.room_id()
                    );
                    let _ = room.leave().await;
                }
            }
        }
    });

    // 2. Incoming message event handler
    let bot_user_id = match client.user_id() {
        Some(id) => id.to_owned(),
        None => {
            error!("Matrix client has no user_id — cannot register message handler");
            return;
        }
    };
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        let own_id = bot_user_id.clone();
        let registry = registry.clone();
        let db = db.clone();
        let broadcaster = broadcaster.clone();
        let pending_merges = pending_merges.clone();

        async move {
            if room.state() != RoomState::Joined || ev.sender == own_id {
                return;
            }

            let is_direct = match room.members(RoomMemberships::JOIN).await {
                Ok(members) if members.len() == 2 => {
                    members.iter().any(|m| m.user_id() == own_id)
                        && members.iter().any(|m| m.user_id() == ev.sender)
                }
                Ok(_) => false,
                Err(_) => room.is_direct().await.unwrap_or(false),
            };

            // Handle incoming image attachments (Receipts, Invoices, Tasks, Notes)
            if let MessageType::Image(ref image_content) = ev.content.msgtype {
                let is_mentioned = image_content.body.contains(own_id.as_str())
                    || image_content.body.contains(own_id.localpart());

                if !is_direct && !is_mentioned {
                    return;
                }

                let _ = room.typing_notice(true).await;
                let owner_id = match crate::accounts::AccountRepository::resolve_or_provision(
                    &db.pool,
                    ev.sender.as_str(),
                )
                .await
                {
                    Ok(owner_id) => owner_id,
                    Err(err) => {
                        error!("Could not resolve account for {}: {:?}", ev.sender, err);
                        let _ = room.typing_notice(false).await;
                        return;
                    }
                };

                let image_bytes = match room
                    .client()
                    .media()
                    .get_file(image_content.clone(), true)
                    .await
                {
                    Ok(Some(bytes)) => bytes,
                    Ok(None) => {
                        warn!(
                            "Media attachment returned empty body in room {}",
                            room.room_id()
                        );
                        let _ = room.typing_notice(false).await;
                        let content = RoomMessageEventContent::text_plain(
                            "❌ Could not download image attachment.",
                        );
                        let _ = room.send(content).await;
                        return;
                    }
                    Err(err) => {
                        warn!(
                            "Failed to fetch media file in room {}: {:?}",
                            room.room_id(),
                            err
                        );
                        let _ = room.typing_notice(false).await;
                        let content = RoomMessageEventContent::text_plain(
                            "❌ Failed to download or decrypt image attachment.",
                        );
                        let _ = room.send(content).await;
                        return;
                    }
                };

                let model_dir = std::path::PathBuf::from(
                    std::env::var("OCR_MODEL_DIR")
                        .unwrap_or_else(|_| "data/ocr_models".to_string()),
                );

                let reply_html = crate::modules::ocr::process_image_event(
                    &db,
                    broadcaster.as_ref(),
                    &room,
                    &owner_id,
                    Some(&image_content.body),
                    &image_bytes,
                    &model_dir,
                )
                .await;

                let _ = room.typing_notice(false).await;

                let plain_text = crate::utils::strip_html(&reply_html);
                let content = RoomMessageEventContent::text_html(&plain_text, &reply_html);
                if let Err(err) = room.send(content).await {
                    error!(
                        "Failed to send image OCR reply to room {}: {:?}",
                        room.room_id(),
                        err
                    );
                }
                return;
            }

            let MessageType::Text(ref text_content) = ev.content.msgtype else {
                return;
            };

            let formatted_body = text_content.formatted.as_ref().map(|f| f.body.as_str());

            let mut parsed = if let Some(parsed) = CommandParser::parse_message(
                &text_content.body,
                formatted_body,
                &own_id,
                room.room_id().as_str(),
                ev.sender.localpart(),
            ) {
                parsed
            } else {
                let trimmed = text_content.body.trim();
                if is_direct && !trimmed.is_empty() {
                    let (quoted, prompt) =
                        crate::bot::quote::extract_quote_and_prompt(trimmed, formatted_body);
                    ParsedCommand {
                        room_id: room.room_id().to_string(),
                        sender_name: ev.sender.localpart().to_string(),
                        command: "ai".to_string(),
                        raw_args: prompt.clone(),
                        full_body: prompt,
                        is_mentioned: false,
                        quoted,
                    }
                } else {
                    return;
                }
            };

            let replied_event_id =
                crate::bot::quote::extract_replied_event_id(&ev.content.relates_to);
            if let Some(ref eid) = replied_event_id {
                if parsed.quoted.is_none() {
                    parsed.quoted =
                        crate::bot::quote::resolve_quoted_event_from_room(&room, eid).await;
                } else if let Some(ref mut q) = parsed.quoted {
                    if q.event_id.is_none() {
                        q.event_id = Some(eid.clone());
                    }
                }
            }

            info!(
                "Command received in [{}] from [{}]: command='{}', args='{}'",
                parsed.room_id, parsed.sender_name, parsed.command, parsed.raw_args
            );

            // `!link` is handled *before* the sender is resolved, and that
            // ordering is load-bearing. Resolving first would provision an
            // empty account for an unknown MXID, and the link would then
            // find that fresh account already attached — turning every
            // first-time link into a pointless merge of nothing.
            if parsed.command.eq_ignore_ascii_case("link") {
                let reply = crate::bot::linking::handle_link_command(
                    &db,
                    &parsed.raw_args,
                    ev.sender.as_str(),
                    is_direct,
                    pending_merges.as_deref(),
                )
                .await;

                let plain = crate::utils::strip_html(&reply);
                let content = RoomMessageEventContent::text_html(&plain, &reply);
                if let Err(err) = room.send(content).await {
                    error!("Failed to answer !link in {}: {:?}", room.room_id(), err);
                }
                return;
            }

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

            let cmd_ctx = parsed.for_owner(owner_id, ev.sender.as_str(), is_direct);

            if is_direct || cmd_ctx.is_mentioned {
                let _ = room.typing_notice(true).await;
            }

            let reply_opt = registry.dispatch(&cmd_ctx, &db, broadcaster.as_ref()).await;

            if is_direct || cmd_ctx.is_mentioned {
                let _ = room.typing_notice(false).await;
            }

            if let Some(reply_html) = reply_opt {
                let plain_text = crate::utils::strip_html(&reply_html);
                let content = RoomMessageEventContent::text_html(&plain_text, &reply_html);
                if let Err(err) = room.send(content).await {
                    error!(
                        "Failed to send message to room {}: {:?}",
                        room.room_id(),
                        err
                    );
                }
            }
        }
    });
}
