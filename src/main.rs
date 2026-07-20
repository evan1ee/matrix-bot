use anyhow::{Context, Result};
use matrix_sdk::{
    config::SyncSettings,
    room::Room,
    ruma::{
        events::room::{
            member::{MembershipState, StrippedRoomMemberEvent},
            message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent},
        },
        ServerName, UserId,
    },
    Client, RoomState,
};
use std::env;
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize environment variables and logging
    dotenvy::dotenv().ok();
    
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,matrix_hello_bot=debug,matrix_sdk=info")),
        )
        .init();

    info!("Starting Matrix Hello Bot in Rust...");

    // 2. Load configuration from environment variables
    let homeserver_input = env::var("MATRIX_HOMESERVER")
        .unwrap_or_else(|_| "https://matrix.org".to_string());
    let username = env::var("MATRIX_USERNAME")
        .context("MATRIX_USERNAME environment variable not set")?;
    let password = env::var("MATRIX_PASSWORD")
        .context("MATRIX_PASSWORD environment variable not set")?;
    let recovery_key = env::var("MATRIX_RECOVERY_KEY").ok();

    info!("Configured homeserver setting: {}", homeserver_input);

    // 3. Build Matrix Client with Automatic Well-Known Discovery & SQLite Store for E2EE
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let client = if let Ok(server_name) = ServerName::parse(clean_domain) {
        info!("Performing Server / Well-Known Discovery for: {}", server_name);
        match Client::builder().server_name(&server_name).sqlite_store("bot_store", None).build().await {
            Ok(c) => c,
            Err(err) => {
                info!("Well-known lookup failed ({}), falling back to direct URL", err);
                Client::builder()
                    .homeserver_url(&homeserver_input)
                    .sqlite_store("bot_store", None)
                    .build()
                    .await
                    .context("Failed to build Matrix client with direct URL")?
            }
        }
    } else {
        info!("Using direct homeserver URL: {}", homeserver_input);
        Client::builder()
            .homeserver_url(&homeserver_input)
            .sqlite_store("bot_store", None)
            .build()
            .await
            .context("Failed to build Matrix client with direct URL")?
    };

    // 4. Log in with fixed device ID to maintain persistent crypto session across restarts
    info!("Logging in as user: {}", username);
    
    // Support either full MXID (@user:domain) or plain localpart (user)
    let user_id = if username.starts_with('@') {
        UserId::parse(&username)
            .context("Failed to parse MATRIX_USERNAME as a valid Matrix User ID (e.g., @user:matrix.org)")?
    } else {
        let raw_id = format!("@{}:{}", username, clean_domain);
        UserId::parse(&raw_id)
            .context("Failed to construct valid Matrix User ID")?
    };

    client
        .matrix_auth()
        .login_username(&user_id, &password)
        .device_id("RUST_HELLO_BOT_SESSION")
        .initial_device_display_name("Rust Matrix Hello Bot")
        .send()
        .await
        .context("Failed to log in to Matrix server")?;

    info!("Successfully logged in as {}", client.user_id().unwrap());

    // 5. Spawn background task for Recovery Key verification after initial sync completes
    if let Some(key) = recovery_key {
        let trimmed_key = key.trim().to_string();
        if !trimmed_key.is_empty() {
            let client_clone = client.clone();
            tokio::spawn(async move {
                // Sleep 3 seconds to allow matrix-sdk sync to populate account data
                tokio::time::sleep(Duration::from_secs(3)).await;
                info!("Attempting session verification using MATRIX_RECOVERY_KEY...");
                match client_clone.encryption().recovery().recover(&trimmed_key).await {
                    Ok(_) => info!("Successfully verified bot session via Recovery Key! 🎉"),
                    Err(err) => warn!("Failed to verify session using Recovery Key: {:?}", err),
                }
            });
        }
    }

    // 6. Register Event Handler for Auto-joining Room Invites
    client.add_event_handler(|ev: StrippedRoomMemberEvent, room: Room| async move {
        if ev.content.membership == MembershipState::Invite {
            info!("Received invitation for room: {}", room.room_id());
            info!("Auto-joining room: {}", room.room_id());
            if let Err(err) = room.join().await {
                error!("Failed to join room {}: {:?}", room.room_id(), err);
            } else {
                info!("Successfully joined room: {}", room.room_id());
            }
        }
    });

    // 7. Register Event Handler for Incoming Room Messages
    let bot_user_id = client.user_id().unwrap().to_owned();
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
        let own_id = bot_user_id.clone();
        async move {
            // Only process messages in joined rooms
            if room.state() != RoomState::Joined {
                return;
            }

            // Ignore messages sent by the bot itself
            if ev.sender == own_id {
                return;
            }

            // Extract text message content
            let MessageType::Text(ref text_content) = ev.content.msgtype else {
                return;
            };

            let body = text_content.body.trim();
            info!(
                "Message received in room [{}] from [{}]: {}",
                room.room_id(),
                ev.sender,
                body
            );

            // Command handling logic
            let response = match body.to_lowercase().as_str() {
                "!hello" | "hello" | "!hi" | "hi" => Some(format!(
                    "👋 Hello, {}! Greetings from <b>matrix-hello-bot</b> written in Rust! 🦀",
                    ev.sender.localpart()
                )),
                "!ping" => Some(
                    "🏓 <b>Pong!</b> Matrix Hello Bot is active and running!".to_string(),
                ),
                "!help" => Some(
                    "🤖 <b>Matrix Hello Bot Commands:</b><br/>\
                     • <code>!hello</code> - Get a friendly greeting<br/>\
                     • <code>!ping</code> - Check bot availability<br/>\
                     • <code>!help</code> - View available commands".to_string(),
                ),
                _ if body.starts_with("!hello ") => {
                    let target_name = body.trim_start_matches("!hello ").trim();
                    Some(format!(
                        "👋 Hello, <b>{}</b>! Greetings from Rust! 🦀",
                        target_name
                    ))
                }
                _ => None,
            };

            if let Some(reply_html) = response {
                // Construct standard HTML formatted message (m.text)
                let content = RoomMessageEventContent::text_html(&reply_html, &reply_html);
                if let Err(err) = room.send(content).await {
                    error!("Failed to send reply to room {}: {:?}", room.room_id(), err);
                } else {
                    info!("Successfully sent reply to room {}", room.room_id());
                }
            }
        }
    });

    info!("Bot is listening for messages and invites... (Press Ctrl+C to stop)");

    // 8. Start Matrix Sync Loop
    client.sync(SyncSettings::default()).await?;

    Ok(())
}
