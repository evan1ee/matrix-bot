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
use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

// Data Structure for To-Do Items
#[derive(Debug, Clone)]
struct TodoItem {
    id: usize,
    sender_mxid: String,
    sender_name: String,
    text: String,
    completed: bool,
}

// In-Memory Thread-Safe To-Do Task Store
#[derive(Clone, Default)]
struct TodoStore {
    tasks: Arc<Mutex<HashMap<String, Vec<TodoItem>>>>,
    next_id: Arc<AtomicUsize>,
}

impl TodoStore {
    fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicUsize::new(1)),
        }
    }

    async fn add(&self, room_id: &str, sender_mxid: &str, sender_name: &str, text: &str) -> TodoItem {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let item = TodoItem {
            id,
            sender_mxid: sender_mxid.to_string(),
            sender_name: sender_name.to_string(),
            text: text.to_string(),
            completed: false,
        };
        let mut guard = self.tasks.lock().await;
        guard.entry(room_id.to_string()).or_default().push(item.clone());
        item
    }

    async fn list_room(&self, room_id: &str) -> Vec<TodoItem> {
        let guard = self.tasks.lock().await;
        guard.get(room_id).cloned().unwrap_or_default()
    }

    async fn list_user(&self, room_id: &str, sender_mxid: &str) -> Vec<TodoItem> {
        let guard = self.tasks.lock().await;
        guard
            .get(room_id)
            .map(|list| {
                list.iter()
                    .filter(|item| item.sender_mxid == sender_mxid)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn mark_done(&self, room_id: &str, id: usize) -> Option<TodoItem> {
        let mut guard = self.tasks.lock().await;
        if let Some(list) = guard.get_mut(room_id) {
            for item in list.iter_mut() {
                if item.id == id {
                    item.completed = true;
                    return Some(item.clone());
                }
            }
        }
        None
    }

    async fn clear_completed_user(&self, room_id: &str, sender_mxid: &str) -> usize {
        let mut guard = self.tasks.lock().await;
        if let Some(list) = guard.get_mut(room_id) {
            let initial_len = list.len();
            list.retain(|item| !(item.completed && item.sender_mxid == sender_mxid));
            return initial_len - list.len();
        }
        0
    }

    async fn clear_completed_room(&self, room_id: &str) -> usize {
        let mut guard = self.tasks.lock().await;
        if let Some(list) = guard.get_mut(room_id) {
            let initial_len = list.len();
            list.retain(|item| !item.completed);
            return initial_len - list.len();
        }
        0
    }
}

// Utility: HTML Escaping
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// Utility: Format Duration
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

// Utility: Parse duration and message (e.g. "10s check oven" -> 10s, "check oven")
fn parse_reminder_args(input: &str) -> Option<(Duration, String)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let mut parts = input.splitn(2, |c: char| c.is_whitespace());
    let time_str = parts.next()?;
    let reminder_msg = parts.next().unwrap_or("Timer is up!").trim();

    let (num_str, unit) = time_str.find(|c: char| c.is_alphabetic()).map_or_else(
        || (time_str, "s"),
        |idx| (&time_str[..idx], &time_str[idx..]),
    );

    let num: u64 = num_str.parse().ok()?;
    let secs = match unit.to_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => num,
        "m" | "min" | "mins" | "minute" | "minutes" => num * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => num * 3600,
        "d" | "day" | "days" => num * 86400,
        _ => return None,
    };

    if secs == 0 {
        return None;
    }

    Some((Duration::from_secs(secs), reminder_msg.to_string()))
}

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

    // Initialize Shared To-Do Store
    let todo_store = TodoStore::new();

    // 2. Load configuration from environment variables
    let homeserver_input =
        env::var("MATRIX_HOMESERVER").unwrap_or_else(|_| "https://matrix.org".to_string());
    let username =
        env::var("MATRIX_USERNAME").context("MATRIX_USERNAME environment variable not set")?;
    let password =
        env::var("MATRIX_PASSWORD").context("MATRIX_PASSWORD environment variable not set")?;
    let recovery_key = env::var("MATRIX_RECOVERY_KEY").ok();

    info!("Configured homeserver setting: {}", homeserver_input);

    // 3. Build Matrix Client with Automatic Well-Known Discovery & SQLite Store for E2EE
    let clean_domain = homeserver_input
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    let client = if let Ok(server_name) = ServerName::parse(clean_domain) {
        info!("Performing Server / Well-Known Discovery for: {}", server_name);
        match Client::builder()
            .server_name(&server_name)
            .sqlite_store("bot_store", None)
            .build()
            .await
        {
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

    // 4. Restore existing session or log in with fixed device ID
    if client.matrix_auth().session().is_none() {
        info!("Logging in as user: {}", username);

        // Support either full MXID (@user:domain) or plain localpart (user)
        let user_id = if username.starts_with('@') {
            UserId::parse(&username)
                .context("Failed to parse MATRIX_USERNAME as a valid Matrix User ID (e.g., @user:matrix.org)")?
        } else {
            let raw_id = format!("@{}:{}", username, clean_domain);
            UserId::parse(&raw_id).context("Failed to construct valid Matrix User ID")?
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
    } else {
        info!("Restored existing session from store for user: {}", client.user_id().unwrap());
    }

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
        let todo_store = todo_store.clone();

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
            let localpart = own_id.localpart();
            let full_mxid = own_id.to_string();
            let short_mention = format!("@{}", localpart);

            let body_lower = body.to_lowercase();
            let localpart_lower = localpart.to_lowercase();

            // Check if HTML formatted_body mentions the bot user ID
            let is_html_mentioned = text_content
                .formatted
                .as_ref()
                .map(|f| f.body.contains(&full_mxid) || f.body.contains(&short_mention))
                .unwrap_or(false);

            // Check plain text mention forms: @bot:domain, @bot, bot:, bot,, @bot:, or starting with bot
            let is_plain_mentioned = body.contains(&full_mxid)
                || body_lower.contains(&short_mention.to_lowercase())
                || body_lower.starts_with(&format!("{}:", localpart_lower))
                || body_lower.starts_with(&format!("{},", localpart_lower))
                || body_lower.starts_with(&format!("@{}", localpart_lower))
                || body_lower == localpart_lower;

            let is_mentioned = is_plain_mentioned || is_html_mentioned;

            // Clean out all mention prefixes to extract the underlying command
            let mut clean_body = body
                .replace(&full_mxid, "")
                .replace(&short_mention, "")
                .trim()
                .to_string();

            let clean_lower = clean_body.to_lowercase();
            if clean_lower.starts_with(&format!("{}:", localpart_lower))
                || clean_lower.starts_with(&format!("{},", localpart_lower))
            {
                clean_body = clean_body[localpart.len() + 1..].trim().to_string();
            } else if clean_lower.starts_with(&localpart_lower) {
                clean_body = clean_body[localpart.len()..].trim().to_string();
            }

            // Trim leading colons, commas, punctuation or spaces leftover from pill formatting
            clean_body = clean_body
                .trim_start_matches(|c: char| c == ':' || c == ',' || c == '@' || c == ' ')
                .trim()
                .to_string();

            info!(
                "Message received in room [{}] from [{}]: {} (Mentioned: {}, Clean: '{}')",
                room.room_id(),
                ev.sender,
                body,
                is_mentioned,
                clean_body
            );

            let clean_lower = clean_body.to_lowercase();

            // Command handling logic (supports !command, @bot command, pill mentions like bot: hello)
            let response = if body.starts_with('!') || is_mentioned {
                // A. Reminder Commands
                if clean_lower.starts_with("remind ")
                    || clean_lower.starts_with("!remind ")
                    || clean_lower.starts_with("remindme ")
                    || clean_lower.starts_with("!remindme ")
                {
                    let raw_args = clean_body
                        .trim_start_matches("!remindme ")
                        .trim_start_matches("remindme ")
                        .trim_start_matches("!remind ")
                        .trim_start_matches("remind ")
                        .trim();

                    if let Some((dur, msg)) = parse_reminder_args(raw_args) {
                        let room_clone = room.clone();
                        let sender_name = ev.sender.localpart().to_string();
                        let msg_clone = msg.clone();

                        // Spawn background async task to automatically post reminder when timer expires
                        tokio::spawn(async move {
                            tokio::time::sleep(dur).await;
                            let alert_html = format!(
                                "⏰ <b>REMINDER for {}!</b><br/>{}",
                                html_escape(&sender_name),
                                html_escape(&msg_clone)
                            );
                            let alert_content =
                                RoomMessageEventContent::text_html(&alert_html, &alert_html);
                            if let Err(err) = room_clone.send(alert_content).await {
                                error!("Failed to send reminder notification to room: {:?}", err);
                            } else {
                                info!("Successfully delivered reminder notification to room!");
                            }
                        });

                        Some(format!(
                            "⏱️ <b>Reminder set!</b> I will remind you in <b>{}</b> for: <i>\"{}\"</i>",
                            format_duration(dur),
                            html_escape(&msg)
                        ))
                    } else {
                        Some(
                            "⚠️ <b>Invalid reminder format.</b><br/>\
                             Usage: <code>!remind &lt;duration&gt; &lt;message&gt;</code><br/>\
                             Examples: <code>!remind 10s check oven</code>, <code>@bot remind 5m meeting</code>"
                                .to_string(),
                        )
                    }
                }
                // B. To-Do Commands
                else if clean_lower.starts_with("todo add ") || clean_lower.starts_with("!todo add ") {
                    let task_text = clean_body
                        .trim_start_matches("!todo add ")
                        .trim_start_matches("todo add ")
                        .trim();

                    if task_text.is_empty() {
                        Some("⚠️ Please specify a task description! Example: <code>!todo add Buy groceries</code>".to_string())
                    } else {
                        let item = todo_store
                            .add(
                                room.room_id().as_str(),
                                ev.sender.as_str(),
                                ev.sender.localpart(),
                                task_text,
                            )
                            .await;
                        Some(format!(
                            "✅ Added Task <b>#{}</b> for {}: <i>\"{}\"</i>",
                            item.id,
                            html_escape(&item.sender_name),
                            html_escape(&item.text)
                        ))
                    }
                } else if clean_lower.starts_with("todo done ") || clean_lower.starts_with("!todo done ") {
                    let id_str = clean_body
                        .trim_start_matches("!todo done ")
                        .trim_start_matches("todo done ")
                        .trim();

                    if let Ok(id) = id_str.parse::<usize>() {
                        if let Some(item) = todo_store.mark_done(room.room_id().as_str(), id).await {
                            Some(format!(
                                "🎉 Marked Task <b>#{}</b> (<i>\"{}\"</i>) as completed! ✅",
                                item.id,
                                html_escape(&item.text)
                            ))
                        } else {
                            Some(format!("⚠️ Task <b>#{}</b> not found in this room.", id))
                        }
                    } else {
                        Some("⚠️ Invalid task ID. Usage: <code>!todo done &lt;id&gt;</code> (e.g. <code>!todo done 1</code>)".to_string())
                    }
                } else if clean_lower == "todo clear room" || clean_lower == "!todo clear room" || clean_lower == "todo clear all" || clean_lower == "!todo clear all" {
                    let cleared_count = todo_store.clear_completed_room(room.room_id().as_str()).await;
                    Some(format!("🗑️ Cleared <b>{}</b> completed task(s) from the room!", cleared_count))
                } else if clean_lower == "todo clear" || clean_lower == "!todo clear" {
                    let cleared_count = todo_store.clear_completed_user(room.room_id().as_str(), ev.sender.as_str()).await;
                    Some(format!("🗑️ Cleared <b>{}</b> of your completed task(s)!", cleared_count))
                } else if clean_lower == "todo room" || clean_lower == "!todo room" || clean_lower == "todo all" || clean_lower == "!todo all" {
                    let items = todo_store.list_room(room.room_id().as_str()).await;
                    if items.is_empty() {
                        Some("📋 <b>Room To-Do List:</b><br/>No active tasks in this room! Add one using <code>!todo add &lt;task&gt;</code>.".to_string())
                    } else {
                        let mut list_html = String::from("📋 <b>All Tasks in Room:</b><br/>");
                        for item in items {
                            if item.completed {
                                list_html.push_str(&format!(
                                    "• [<b>#{}</b>] <del>{}</del> ✅ <i>(by {})</i><br/>",
                                    item.id,
                                    html_escape(&item.text),
                                    html_escape(&item.sender_name)
                                ));
                            } else {
                                list_html.push_str(&format!(
                                    "• [<b>#{}</b>] {} <i>(by {})</i><br/>",
                                    item.id,
                                    html_escape(&item.text),
                                    html_escape(&item.sender_name)
                                ));
                            }
                        }
                        Some(list_html)
                    }
                } else if clean_lower == "todo" || clean_lower == "!todo" || clean_lower == "todo list" || clean_lower == "!todo list" || clean_lower == "todo my" || clean_lower == "!todo my" {
                    let items = todo_store.list_user(room.room_id().as_str(), ev.sender.as_str()).await;
                    if items.is_empty() {
                        Some(format!("📋 <b>Your To-Do List ({}):</b><br/>You have no active tasks in this room! Add one using <code>!todo add &lt;task&gt;</code>.<br/><i>(Use <code>!todo room</code> to view everyone's tasks)</i>", html_escape(ev.sender.localpart())))
                    } else {
                        let mut list_html = format!("📋 <b>Your To-Do List ({}):</b><br/>", html_escape(ev.sender.localpart()));
                        for item in items {
                            if item.completed {
                                list_html.push_str(&format!(
                                    "• [<b>#{}</b>] <del>{}</del> ✅<br/>",
                                    item.id,
                                    html_escape(&item.text)
                                ));
                            } else {
                                list_html.push_str(&format!(
                                    "• [<b>#{}</b>] {}<br/>",
                                    item.id,
                                    html_escape(&item.text)
                                ));
                            }
                        }
                        list_html.push_str("<i>(Tip: Use <code>!todo room</code> to view tasks from everyone)</i>");
                        Some(list_html)
                    }
                }
                // C. Basic Greeting & Help Commands
                else {
                    match clean_lower.as_str() {
                        "" | "hello" | "hi" | "!hello" | "!hi" => Some(format!(
                            "👋 Hello, {}! Greetings from <b>matrix-hello-bot</b> written in Rust! 🦀",
                            ev.sender.localpart()
                        )),
                        "ping" | "!ping" => Some(
                            "🏓 <b>Pong!</b> Matrix Hello Bot is active and running!".to_string(),
                        ),
                        "help" | "!help" => Some(
                            "🤖 <b>Matrix Hello Bot Commands:</b><br/>\
                             • <code>!hello</code> or <code>@bot hello</code> - Get a friendly greeting<br/>\
                             • <code>!ping</code> - Check bot availability<br/>\
                             • <code>!remind &lt;time&gt; &lt;msg&gt;</code> - Set a timer (e.g. <code>!remind 10s check oven</code>)<br/>\
                             • <code>!todo add &lt;task&gt;</code> - Add task to your list<br/>\
                             • <code>!todo</code> or <code>!todo my</code> - View your personal tasks<br/>\
                             • <code>!todo room</code> - View all tasks in this room<br/>\
                             • <code>!todo done &lt;id&gt;</code> - Mark task #id as completed<br/>\
                             • <code>!todo clear</code> - Remove your completed tasks".to_string(),
                        ),
                        _ if clean_lower.starts_with("hello ") || clean_lower.starts_with("!hello ") => {
                            let target_name = clean_body
                                .trim_start_matches("!hello ")
                                .trim_start_matches("hello ")
                                .trim();
                            Some(format!(
                                "👋 Hello, <b>{}</b>! Greetings from Rust! 🦀",
                                target_name
                            ))
                        }
                        _ if is_mentioned => Some(format!(
                            "👋 Hello, {}! You mentioned me! Greetings from <b>matrix-hello-bot</b> written in Rust! 🦀",
                            ev.sender.localpart()
                        )),
                        _ => None,
                    }
                }
            } else {
                match body.to_lowercase().as_str() {
                    "hello" | "hi" => Some(format!(
                        "👋 Hello, {}! Greetings from <b>matrix-hello-bot</b> written in Rust! 🦀",
                        ev.sender.localpart()
                    )),
                    _ => None,
                }
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
