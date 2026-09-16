use futures_util::FutureExt;
use matrix_sdk::{
    room::Room,
    ruma::events::room::message::{
        MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
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
    crate::bot::invite::register_invite_handler(client, db.clone());

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
                                "⚠️ 账号验证失败：无法解析 Matrix 用户身份。请稍后重试或重新加入房间。",
                                "⚠️ <b>账号验证失败</b><br/>• <b>原因:</b> 无法解析 Matrix 用户身份<br/>• <b>建议:</b> 请稍后重试或重新加入房间。",
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
                                "⚠️ 图片获取失败：图片附件数据为空。请重新上传发送。",
                                "⚠️ <b>图片处理失败</b><br/>• <b>原因:</b> 图片附件数据为空<br/>• <b>建议:</b> 请重新上传发送，或直接输入文字。",
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
                                "⚠️ 图片下载失败：无法下载或解密图片附件。请检查网络后重新发送。",
                                "⚠️ <b>图片下载失败</b><br/>• <b>原因:</b> 无法下载或解密图片附件<br/>• <b>建议:</b> 请检查网络连接后重新发送，或直接输入文字。",
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
                let ocr_res = std::panic::AssertUnwindSafe(crate::modules::ocr::process_image_event(
                    &db,
                    broadcaster.as_ref(),
                    &room,
                    &owner_id,
                    Some(&image_content.body),
                    &image_bytes,
                    &model_dir,
                    Some(&reporter),
                ))
                .catch_unwind()
                .await;

                typing_guard.stop().await;

                let reply_html = match ocr_res {
                    Ok(html) => html,
                    Err(panic_err) => {
                        error!("OCR image processing panicked: {:?}", panic_err);
                        "⚠️ <b>图片处理异常</b><br/>• <b>原因:</b> 图片识别过程中发生系统异常<br/>• <b>建议:</b> 请尝试重新拍摄发送，或直接输入文字。".to_string()
                    }
                };

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
                                "⚠️ 账号验证失败：无法解析 Matrix 用户身份。请稍后重试或重新加入房间。",
                                "⚠️ <b>账号验证失败</b><br/>• <b>原因:</b> 无法解析 Matrix 用户身份<br/>• <b>建议:</b> 请稍后重试或重新加入房间。",
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
                                "⚠️ 语音获取失败：音频附件数据为空。请重新录制发送。",
                                "⚠️ <b>语音处理失败</b><br/>• <b>原因:</b> 音频附件数据为空<br/>• <b>建议:</b> 请重新录制并发送语音，或直接输入文字。",
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
                                "⚠️ 语音下载失败：无法下载或解密音频附件。请检查网络后重新发送。",
                                "⚠️ <b>语音下载失败</b><br/>• <b>原因:</b> 无法下载或解密音频附件<br/>• <b>建议:</b> 请检查网络连接后重新发送，或直接输入文字。",
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
                let voice_res = std::panic::AssertUnwindSafe(crate::modules::voice::process_audio_event(
                    &db,
                    broadcaster.as_ref(),
                    &room,
                    &owner_id,
                    &audio_bytes,
                    &voice_config,
                    Some(&reporter),
                ))
                .catch_unwind()
                .await;

                typing_guard.stop().await;

                let reply_html = match voice_res {
                    Ok(html) => html,
                    Err(panic_err) => {
                        error!("Voice audio processing panicked: {:?}", panic_err);
                        "⚠️ <b>语音处理异常</b><br/>• <b>原因:</b> 音频解码或识别过程中发生系统异常<br/>• <b>建议:</b> 请尝试重新发送语音，或直接输入文字。".to_string()
                    }
                };

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

            let dispatch_res = std::panic::AssertUnwindSafe(
                registry.dispatch(&cmd_ctx, &db, broadcaster.as_ref())
            )
            .catch_unwind()
            .await;

            if let Some(mut guard) = typing_guard {
                guard.stop().await;
            }

            match dispatch_res {
                Ok(Some(reply_html)) => {
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
                }
                Ok(None) => {
                    if let Some(indicator) = stage_indicator {
                        indicator
                            .cancel_with_error(
                                "⚠️ AI 处理失败：未能生成回复或处理超时。请尝试重新提问。",
                                "⚠️ <b>AI 处理失败</b><br/>• <b>原因:</b> 未能生成有效回复或处理超时<br/>• <b>建议:</b> 请尝试重新提问，或换一种表达方式。",
                            )
                            .await;
                    }
                }
                Err(panic_err) => {
                    error!("Command dispatch panicked: {:?}", panic_err);
                    let err_html = "⚠️ <b>指令处理遇到异常</b><br/>• <b>原因:</b> 系统处理请求时发生意外错误<br/>• <b>建议:</b> 请稍后重试，或检查输入内容。";
                    let plain_text = crate::utils::strip_html(err_html);
                    if let Some(indicator) = stage_indicator {
                        indicator.finish_with_reply(&plain_text, err_html).await;
                    } else {
                        let content = RoomMessageEventContent::text_html(&plain_text, err_html);
                        let _ = room.send(content).await;
                    }
                }
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
