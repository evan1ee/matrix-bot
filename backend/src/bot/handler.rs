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

                let mut typing_guard = crate::bot::stage::TypingGuard::start(room.clone());
                let stage_indicator = crate::bot::stage::StageIndicator::start(
                    &room,
                    "🔍 Scanning image with OCR...",
                    "🔍 <i>正在识别图片文字 (OCR)...</i>",
                )
                .await;

                let owner_id = match crate::accounts::AccountRepository::resolve_or_provision(
                    &db.pool,
                    ev.sender.as_str(),
                )
                .await
                {
                    Ok(owner_id) => owner_id,
                    Err(err) => {
                        error!("Could not resolve account for {}: {:?}", ev.sender, err);
                        typing_guard.stop().await;
                        stage_indicator
                            .cancel_with_error(
                                "❌ Account resolution failed.",
                                "❌ <i>Account resolution failed.</i>",
                            )
                            .await;
                        return;
                    }
                };

                let image_bytes = match crate::bot::media::download_media_file(
                    &room.client(),
                    &image_content.source,
                )
                .await
                {
                    Ok(bytes) if !bytes.is_empty() => bytes,
                    Ok(_) => {
                        warn!(
                            "Media attachment returned empty body in room {}",
                            room.room_id()
                        );
                        typing_guard.stop().await;
                        stage_indicator
                            .cancel_with_error(
                                "❌ Could not download image attachment.",
                                "❌ <i>Could not download image attachment.</i>",
                            )
                            .await;
                        return;
                    }
                    Err(err) => {
                        warn!(
                            "Failed to fetch media file in room {}: {:?}",
                            room.room_id(),
                            err
                        );
                        typing_guard.stop().await;
                        stage_indicator
                            .cancel_with_error(
                                "❌ Failed to download or decrypt image attachment.",
                                "❌ <i>Failed to download or decrypt image attachment.</i>",
                            )
                            .await;
                        return;
                    }
                };

                let model_dir = std::path::PathBuf::from(
                    std::env::var("OCR_MODEL_DIR")
                        .unwrap_or_else(|_| "data/ocr_models".to_string()),
                );

                let reporter = stage_indicator.reporter();
                let reply_html = crate::modules::ocr::process_image_event(
                    &db,
                    broadcaster.as_ref(),
                    &room,
                    &owner_id,
                    Some(&image_content.body),
                    &image_bytes,
                    &model_dir,
                    Some(&reporter),
                )
                .await;

                typing_guard.stop().await;

                let plain_text = crate::utils::strip_html(&reply_html);
                stage_indicator.finish_with_reply(&plain_text, &reply_html).await;
                return;
            }

            // Handle incoming audio attachments / voice notes
            if let MessageType::Audio(ref audio_content) = ev.content.msgtype {
                let is_mentioned = audio_content.body.contains(own_id.as_str())
                    || audio_content.body.contains(own_id.localpart());

                if !is_direct && !is_mentioned {
                    return;
                }

                let mut typing_guard = crate::bot::stage::TypingGuard::start(room.clone());
                let stage_indicator = crate::bot::stage::StageIndicator::start(
                    &room,
                    "🎧 Transcribing voice...",
                    "🎧 <i>正在转写语音...</i>",
                )
                .await;

                let owner_id = match crate::accounts::AccountRepository::resolve_or_provision(
                    &db.pool,
                    ev.sender.as_str(),
                )
                .await
                {
                    Ok(owner_id) => owner_id,
                    Err(err) => {
                        error!("Could not resolve account for {}: {:?}", ev.sender, err);
                        typing_guard.stop().await;
                        stage_indicator
                            .cancel_with_error(
                                "❌ Account resolution failed.",
                                "❌ <i>Account resolution failed.</i>",
                            )
                            .await;
                        return;
                    }
                };

                let audio_bytes = match crate::bot::media::download_media_file(
                    &room.client(),
                    &audio_content.source,
                )
                .await
                {
                    Ok(bytes) if !bytes.is_empty() => bytes,
                    Ok(_) => {
                        warn!(
                            "Audio media attachment returned empty body in room {}",
                            room.room_id()
                        );
                        typing_guard.stop().await;
                        stage_indicator
                            .cancel_with_error(
                                "❌ Could not download audio attachment.",
                                "❌ <i>Could not download audio attachment.</i>",
                            )
                            .await;
                        return;
                    }
                    Err(err) => {
                        warn!(
                            "Failed to fetch audio file in room {}: {:?}",
                            room.room_id(),
                            err
                        );
                        typing_guard.stop().await;
                        stage_indicator
                            .cancel_with_error(
                                "❌ Failed to download or decrypt audio attachment.",
                                "❌ <i>Failed to download or decrypt audio attachment.</i>",
                            )
                            .await;
                        return;
                    }
                };

                let voice_config = crate::modules::voice::VoiceConfig {
                    enabled: std::env::var("VOICE_ENABLED")
                        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                        .unwrap_or(true),
                    model_dir: std::path::PathBuf::from(
                        std::env::var("VOICE_MODEL_DIR")
                            .unwrap_or_else(|_| "data/voice_models".to_string()),
                    ),
                    language: std::env::var("VOICE_LANGUAGE")
                        .unwrap_or_else(|_| "auto".to_string()),
                };

                let reporter = stage_indicator.reporter();
                let reply_html = crate::modules::voice::process_audio_event(
                    &db,
                    broadcaster.as_ref(),
                    &room,
                    &owner_id,
                    &audio_bytes,
                    &voice_config,
                    Some(&reporter),
                )
                .await;

                typing_guard.stop().await;

                let plain_text = crate::utils::strip_html(&reply_html);
                stage_indicator.finish_with_reply(&plain_text, &reply_html).await;
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

            let is_ai = is_ai_prompt_command(&parsed);
            let mut cmd_ctx = parsed.for_owner(owner_id, ev.sender.as_str(), is_direct);

            let typing_guard = if is_direct || cmd_ctx.is_mentioned {
                Some(crate::bot::stage::TypingGuard::start(room.clone()))
            } else {
                None
            };

            let stage_indicator = if is_ai {
                let ind = crate::bot::stage::StageIndicator::start(
                    &room,
                    "🧠 Thinking...",
                    "🧠 <i>正在思考...</i>",
                )
                .await;
                cmd_ctx.stage_reporter = Some(ind.reporter());
                Some(ind)
            } else {
                None
            };

            let reply_opt = registry.dispatch(&cmd_ctx, &db, broadcaster.as_ref()).await;

            if let Some(mut guard) = typing_guard {
                guard.stop().await;
            }

            if let Some(reply_html) = reply_opt {
                let plain_text = crate::utils::strip_html(&reply_html);
                if let Some(indicator) = stage_indicator {
                    indicator.finish_with_reply(&plain_text, &reply_html).await;
                } else {
                    let content = RoomMessageEventContent::text_html(&plain_text, &reply_html);
                    if let Err(err) = room.send(content).await {
                        error!(
                            "Failed to send message to room {}: {:?}",
                            room.room_id(),
                            err
                        );
                    }
                }
            } else if let Some(indicator) = stage_indicator {
                indicator
                    .cancel_with_error(
                        "❌ AI assistant encountered an error.",
                        "❌ <i>AI assistant encountered an error.</i>",
                    )
                    .await;
            }
        }
    });
}

fn is_ai_prompt_command(parsed: &crate::modules::ParsedCommand) -> bool {
    let cmd = parsed.command.to_ascii_lowercase();
    if cmd == "ai" || cmd == "ask" {
        let trimmed = parsed.raw_args.trim();
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        !matches!(
            first_word.to_ascii_lowercase().as_str(),
            "reset" | "clear" | "config" | "retro" | "retrospective" | "weekly"
        )
    } else if parsed.is_mentioned {
        const BUILTIN_COMMANDS: &[&str] = &[
            "ping", "help", "stat", "stats", "version", "uptime", "todo", "todos",
            "remind", "reminder", "reminders", "finance", "expense", "income", "budget",
            "portfolio", "invest", "stock", "crypto", "asset", "footprint", "footprints",
            "trip", "trips", "habit", "habits", "life", "lifegrid", "bill", "bills",
            "settings", "setting", "webhook", "webhooks", "link",
        ];
        !BUILTIN_COMMANDS.contains(&cmd.as_str())
    } else {
        false
    }
}
