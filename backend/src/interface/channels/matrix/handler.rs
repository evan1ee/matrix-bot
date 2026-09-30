use matrix_sdk::{
    room::Room,
    ruma::events::room::message::{MessageType, OriginalSyncRoomMessageEvent},
    Client, RoomMemberships, RoomState,
};
use std::sync::Arc;
use tracing::error;

use crate::db::Database;
use crate::interface::chat::ModuleRegistry;

pub fn register_handlers(
    client: &Client,
    config: &crate::config::Config,
    registry: Arc<ModuleRegistry>,
    db: Database,
    services: crate::application::AppServices,
    pending_merges: Option<Arc<crate::interface::channels::matrix::linking::PendingMerges>>,
) {
    // 1. Authorized invite event handler
    crate::interface::channels::matrix::invite::register_invite_handler(
        client,
        db.clone(),
        config.verification_allowlist.clone(),
    );

    // 2. Reaction event handler
    crate::interface::channels::matrix::reaction::register_reaction_handler(
        client,
        db.clone(),
        services.clone(),
    );

    // 3. Incoming message event handler
    let bot_user_id = match client.user_id() {
        Some(id) => id.to_owned(),
        None => {
            error!("Matrix client has no user_id — cannot register message handler");
            return;
        }
    };
    let ocr_client =
        crate::infrastructure::inference::ocr::OcrClient::new(config.ocr_base_url.clone());
    let voice_config = crate::interface::chat::voice::VoiceConfig {
        enabled: config.voice_enabled,
        language: config.voice_language.clone(),
    };
    let stt_client = crate::infrastructure::inference::stt::SttClient::new(
        config.stt_base_url.clone(),
        config.stt_api_key.clone(),
        config.stt_model.clone(),
    );
    let ai_config = config.ai.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        let own_id = bot_user_id.clone();
        let registry = registry.clone();
        let db = db.clone();
        let services = services.clone();
        let pending_merges = pending_merges.clone();
        let ocr_client = ocr_client.clone();
        let voice_config = voice_config.clone();
        let stt_client = stt_client.clone();
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
                    crate::interface::channels::matrix::attachments::handle_attachment(
                        &ev,
                        &room,
                        &own_id,
                        is_direct,
                        &db,
                        &services,
                        &ai_config,
                        &ocr_client,
                        &voice_config,
                        &stt_client,
                    )
                    .await;
                }
                MessageType::Text(_) => {
                    crate::interface::channels::matrix::command_flow::handle_text(
                        &ev,
                        &room,
                        &own_id,
                        is_direct,
                        &registry,
                        &db,
                        &services,
                        pending_merges.as_deref(),
                    )
                    .await;
                }
                _ => {}
            }
        }
    });
}
