use matrix_sdk::{
    room::Room,
    ruma::events::room::message::{MessageType, OriginalSyncRoomMessageEvent},
    Client, RoomMemberships, RoomState,
};
use std::sync::Arc;
use tracing::error;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::ModuleRegistry;

pub fn register_handlers(
    client: &Client,
    config: &crate::config::Config,
    registry: Arc<ModuleRegistry>,
    db: Database,
    broadcaster: Option<EventBroadcaster>,
    pending_merges: Option<Arc<crate::bot::linking::PendingMerges>>,
) {
    // 1. Authorized invite event handler
    crate::bot::invite::register_invite_handler(
        client,
        db.clone(),
        config.verification_allowlist.clone(),
    );

    // 2. Reaction event handler
    crate::bot::reaction::register_reaction_handler(client, db.clone(), broadcaster.clone());

    // 3. Incoming message event handler
    let bot_user_id = match client.user_id() {
        Some(id) => id.to_owned(),
        None => {
            error!("Matrix client has no user_id — cannot register message handler");
            return;
        }
    };
    let ocr_client = crate::services::ocr::OcrClient::new(config.ocr_base_url.clone());
    let voice_config = crate::modules::voice::VoiceConfig {
        enabled: config.voice_enabled,
        model_dir: config.voice_model_dir.clone(),
        language: config.voice_language.clone(),
    };
    let ai_config = config.ai.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        let own_id = bot_user_id.clone();
        let registry = registry.clone();
        let db = db.clone();
        let broadcaster = broadcaster.clone();
        let pending_merges = pending_merges.clone();
        let ocr_client = ocr_client.clone();
        let voice_config = voice_config.clone();
        let ai_config = ai_config.clone();

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

            match &ev.content.msgtype {
                MessageType::Image(_) | MessageType::Audio(_) => {
                    crate::bot::attachments::handle_attachment(
                        &ev,
                        &room,
                        &own_id,
                        is_direct,
                        &db,
                        &ai_config,
                        broadcaster.as_ref(),
                        &ocr_client,
                        &voice_config,
                    )
                    .await;
                }
                MessageType::Text(_) => {
                    crate::bot::command_flow::handle_text(
                        &ev,
                        &room,
                        &own_id,
                        is_direct,
                        &registry,
                        &db,
                        broadcaster.as_ref(),
                        pending_merges.as_deref(),
                    )
                    .await;
                }
                _ => {}
            }
        }
    });
}
