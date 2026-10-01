//! Turning an incoming text message into a command dispatch or an AI reply.

use std::sync::Arc;

use futures_util::FutureExt;
use matrix_sdk::{
    room::Room,
    ruma::events::room::message::{MessageType, OriginalSyncRoomMessageEvent},
    ruma::UserId,
};
use tracing::{error, info};

use crate::application::identity::Resolution;
use crate::db::Database;
use crate::domain::channels::PlatformIdentity;
use crate::interface::channels::matrix::dispatcher::CommandParser;
use crate::interface::channels::matrix::linking::PendingMerges;
use crate::interface::chat::ModuleRegistry;

/// Parses a text message and either answers `!link`, dispatches it to a
/// module, or hands it to the AI assistant as a direct-message prompt.
///
/// A no-op for any other message type.
#[allow(clippy::too_many_arguments)]
pub async fn handle_text(
    ev: &OriginalSyncRoomMessageEvent,
    room: &Room,
    own_id: &UserId,
    is_direct: bool,
    registry: &Arc<ModuleRegistry>,
    db: &Database,
    services: &crate::application::AppServices,
    pending_merges: Option<&PendingMerges>,
) {
    let MessageType::Text(ref text_content) = ev.content.msgtype else {
        return;
    };

    // Built before the parse (the parser records who sent the command); a
    // sender that is not a valid MXID cannot be answered, so the message is dropped.
    let sender = match PlatformIdentity::matrix(ev.sender.as_str()) {
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
    }

    info!(
        "Command received in [{}] from [{}]: command='{}', args='{}'",
        parsed.conversation.to_storage(),
        parsed.sender_name(),
        parsed.command,
        parsed.raw_args
    );

    // `!link` is handled *before* the sender is resolved, and that
    // ordering is load-bearing. Resolving first would provision an
    // empty account for an unknown MXID, and the link would then
    // find that fresh account already attached — turning every
    // first-time link into a pointless merge of nothing.
    if parsed.command.eq_ignore_ascii_case("link") {
        let reply = crate::interface::channels::matrix::linking::handle_link_command(
            &services.accounts,
            db,
            &parsed.raw_args,
            ev.sender.as_str(),
            is_direct,
            pending_merges,
        )
        .await;

        crate::interface::channels::matrix::reply::send_html(room, &reply).await;
        return;
    }

    // Resolve the sender to the account that will own whatever this
    // command creates, provisioning one if this MXID has never been
    // seen. Deliberately after the parse: an unrelated room message
    // must not cost a query, and must not create an account for
    // someone who only ever said hello to a room the bot is in.
    let owner_id = match services.identity.resolve(&sender).await {
        Ok(Resolution::Account(owner_id)) => owner_id,
        Ok(Resolution::Unlinked) => {
            // Matrix provisions every sender; reaching this is a policy bug.
            error!(
                "Matrix sender {} resolved as unlinked — dropping the command",
                ev.sender
            );
            return;
        }
        Err(err) => {
            error!(
                "Could not resolve an account for {}: {:?} — dropping the command",
                ev.sender, err
            );
            return;
        }
    };

    let is_ai = CommandParser::is_ai_prompt_command(&parsed);
    let mut cmd_ctx = parsed.for_owner(owner_id, is_direct);

    let typing_guard = if is_direct || cmd_ctx.is_mentioned {
        Some(crate::interface::channels::matrix::stage::TypingGuard::start(room.clone()))
    } else {
        None
    };

    let stage_indicator = if is_ai {
        let ind = crate::interface::channels::matrix::stage::StageIndicator::start(
            room,
            "🧠 Thinking...",
            "🧠 <i>正在思考...</i>",
        )
        .await;
        cmd_ctx.progress = Some(Arc::new(ind.reporter()));
        Some(ind)
    } else {
        None
    };

    let dispatch_res = std::panic::AssertUnwindSafe(registry.dispatch(&cmd_ctx, services, db))
        .catch_unwind()
        .await;

    if let Some(mut guard) = typing_guard {
        guard.stop().await;
    }

    match dispatch_res {
        Ok(Some(reply)) => {
            // Audio goes first, the caption after it: the order a voice reply
            // has always had in the room.
            let reply =
                crate::interface::channels::matrix::voice_upload::deliver_audio(room, reply).await;
            let reply_html = reply.html_body();
            let (sent_eid, ph_eid) = if let Some(indicator) = stage_indicator {
                let ev_id = indicator
                    .finish_with_reply(&reply.content.plain, reply_html)
                    .await;
                (ev_id, indicator.event_id().cloned())
            } else {
                (
                    crate::interface::channels::matrix::reply::send_html(room, reply_html).await,
                    None,
                )
            };

            if let Some(ref eid) = sent_eid {
                crate::interface::channels::matrix::reaction_targets::try_record_from_reply(
                    &db.pool,
                    eid.as_str(),
                    room.room_id().as_str(),
                    &cmd_ctx.owner_id,
                    reply_html,
                )
                .await;
            }

            crate::interface::channels::matrix::reaction_targets::link_turn_targets(
                &db.pool,
                ph_eid.as_deref().map(|e| e.as_str()),
                sent_eid.as_deref().map(|e| e.as_str()),
                ev.event_id.as_str(),
            )
            .await;
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
            if let Some(indicator) = stage_indicator {
                let plain_text = crate::utils::strip_html(err_html);
                indicator.finish_with_reply(&plain_text, err_html).await;
            } else {
                crate::interface::channels::matrix::reply::send_html(room, err_html).await;
            }
        }
    }
}
