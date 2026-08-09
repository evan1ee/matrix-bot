//! Interactive device verification.
//!
//! A bot's session shows as *Unverified* to another user until that user's
//! user-signing key signs the bot's master key. Only the other user's client
//! holds that private key, so the bot cannot sign itself into their trust —
//! it has to complete an interactive verification with them once.
//!
//! This module answers verification requests and auto-confirms the SAS emoji,
//! because a headless bot has nobody to compare emoji with. That auto-confirm
//! is precisely why the allowlist below is not optional: completing a
//! verification makes the bot sign the other party's identity, so accepting
//! from anyone would let a stranger become trusted and receive room keys.

use std::{collections::HashSet, path::Path, sync::Arc, time::Duration};

use futures_util::StreamExt;
use matrix_sdk::{
    encryption::verification::{SasState, SasVerification, Verification, VerificationRequestState},
    ruma::events::{
        key::verification::request::ToDeviceKeyVerificationRequestEvent,
        room::message::{MessageType, OriginalSyncRoomMessageEvent},
    },
    Client,
};
use tracing::{debug, error, info, warn};

/// Who the bot will verify with.
#[derive(Clone, Debug)]
pub struct VerificationPolicy {
    /// The bot's own user, always permitted — verifying your own other sessions
    /// is how a bot joins its own account's trust.
    pub own_user: String,
    /// Additional users permitted to verify with the bot.
    pub allowed: Vec<String>,
}

impl VerificationPolicy {
    pub fn new(own_user: impl Into<String>, allowed: Vec<String>) -> Self {
        Self { own_user: own_user.into(), allowed }
    }

    /// Whether a verification request from `sender` should be answered.
    ///
    /// Matching is exact: an MXID is a stable identifier, and a prefix or
    /// substring match would let `@evan-evil:host` inherit `@evan:host`'s trust.
    pub fn permits(&self, sender: &str) -> bool {
        sender == self.own_user || self.allowed.iter().any(|u| u == sender)
    }
}

/// Registers handlers for both verification transports.
///
/// Requests arrive either as to-device events (Element's "verify session" on
/// your own account) or as a message in a room (verifying another user), so
/// both have to be answered or the request silently hangs.
pub fn register_verification_handlers(client: &Client, policy: VerificationPolicy) {
    let policy = Arc::new(policy);

    let to_device_policy = policy.clone();
    client.add_event_handler(
        move |ev: ToDeviceKeyVerificationRequestEvent, client: Client| {
            let policy = to_device_policy.clone();
            async move {
                handle_request(
                    client,
                    policy,
                    ev.sender.to_string(),
                    ev.content.transaction_id.to_string(),
                )
                .await;
            }
        },
    );

    let in_room_policy = policy.clone();
    client.add_event_handler(
        move |ev: OriginalSyncRoomMessageEvent, client: Client| {
            let policy = in_room_policy.clone();
            async move {
                // Our own outgoing request lands back in this handler too — a
                // room message is echoed to every member including the sender.
                // Without this check the bot answers itself and logs a
                // confusing "vanished before it could be answered" warning for
                // a request nobody was ever meant to receive.
                if client.user_id() == Some(ev.sender.as_ref()) {
                    return;
                }

                if let MessageType::VerificationRequest(_) = &ev.content.msgtype {
                    handle_request(
                        client,
                        policy,
                        ev.sender.to_string(),
                        ev.event_id.to_string(),
                    )
                    .await;
                }
            }
        },
    );

    // `!verify` lets an allowlisted user ask the bot to start the flow. Element
    // has no obvious Verify affordance for a bot you have never DMed, so the
    // bot offering it is the practical path.
    let command_policy = policy.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: matrix_sdk::room::Room, client: Client| {
        let policy = command_policy.clone();
        async move {
            let MessageType::Text(ref text) = ev.content.msgtype else { return };
            if text.body.trim().to_lowercase() != "!verify" {
                return;
            }

            let sender = ev.sender.to_string();
            let reply = if !policy.permits(&sender) {
                format!(
                    "⚠️ {} is not in <code>MATRIX_VERIFICATION_ALLOWLIST</code>, so I will not verify with you.",
                    crate::utils::html_escape(&sender)
                )
            } else {
                match request_verification_with(&client, &sender).await {
                    Ok(()) => "🔐 Verification request sent — accept it in your client and I will confirm automatically.".to_string(),
                    Err(err) => format!("❌ {}", crate::utils::html_escape(&err)),
                }
            };

            let content = matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_html(
                crate::utils::strip_html(&reply),
                reply.clone(),
            );
            if let Err(err) = room.send(content).await {
                error!("Failed to reply to !verify: {:?}", err);
            }
        }
    });

    info!(
        "Verification handlers registered (allowed: {} + own user)",
        policy.allowed.len()
    );
}

async fn handle_request(
    client: Client,
    policy: Arc<VerificationPolicy>,
    sender: String,
    flow_id: String,
) {
    if !policy.permits(&sender) {
        warn!(
            "Ignoring verification request from {} — not in MATRIX_VERIFICATION_ALLOWLIST. \
             Add them to that list to allow it.",
            sender
        );
        return;
    }

    let Ok(user_id) = matrix_sdk::ruma::UserId::parse(&sender) else {
        warn!("Verification request from an unparseable user ID: {}", sender);
        return;
    };

    let Some(request) = client
        .encryption()
        .get_verification_request(&user_id, &flow_id)
        .await
    else {
        warn!("Verification request {} from {} vanished before it could be answered", flow_id, sender);
        return;
    };

    info!("Accepting verification request from {}", sender);
    if let Err(err) = request.accept().await {
        error!("Failed to accept verification request from {}: {:?}", sender, err);
        return;
    }

    drive_request(request, &sender).await;
}

/// Runs a verification request to completion, whichever side started it.
async fn drive_request(request: matrix_sdk::encryption::verification::VerificationRequest, sender: &str) {
    let mut changes = request.changes();
    while let Some(state) = changes.next().await {
        match state {
            VerificationRequestState::Ready { .. } => {
                // The other side is ready; start the emoji exchange rather than
                // waiting for them to pick a method.
                if let Err(err) = request.start_sas().await {
                    warn!("Could not start SAS with {}: {:?}", sender, err);
                }
            }
            VerificationRequestState::Transitioned { verification } => {
                if let Verification::SasV1(sas) = verification {
                    run_sas(sas, sender).await;
                }
                break;
            }
            VerificationRequestState::Done => {
                info!("Verification with {} completed", sender);
                break;
            }
            VerificationRequestState::Cancelled(info) => {
                warn!("Verification with {} cancelled: {:?}", sender, info.reason());
                break;
            }
            _ => {}
        }
    }
}

/// How long to wait for an allowlisted user's identity to reach the store.
const IDENTITY_ATTEMPTS: u32 = 6;
const IDENTITY_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Where users we have completed a SAS exchange with are recorded, so a
/// restart does not ask them again.
///
/// This is deliberately not `UserIdentity::is_verified()`. That flag reports
/// only whether *we* signed *them*, and it was observed `true` for a user
/// whose client was, at the same moment, still refusing to send to this bot —
/// it cannot see the signature direction that actually matters. A completed
/// `SasState::Done`, by contrast, is the cryptographic proof that both sides
/// exchanged and validated MACs; there is no stronger signal available to the
/// bot that the exchange truly went through, which is why it is the one thing
/// this file treats as durable.
fn verified_users_path() -> std::path::PathBuf {
    Path::new("data/verified_users.json").to_path_buf()
}

fn load_verified_users(path: &Path) -> HashSet<String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_verified_users(path: &Path, users: &HashSet<String>) {
    let Ok(bytes) = serde_json::to_vec(users) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = std::fs::write(path, bytes) {
        warn!("Could not persist verified users to {}: {}", path.display(), err);
    }
}

/// Records that a completed SAS exchange with `user` was observed, so future
/// boots stop asking them.
fn mark_verification_complete(user: &str) {
    let path = verified_users_path();
    let mut users = load_verified_users(&path);
    if users.insert(user.to_string()) {
        save_verified_users(&path, &users);
        info!("Recorded a completed verification with {} — will not ask again", user);
    }
}

/// Asks every allowlisted user to verify the bot, unless a completed exchange
/// with them is already on record.
///
/// Cross-user trust cannot be self-served. Signing the bot's own device makes
/// the session verified *within the bot's account*; for another user to trust
/// it, their user-signing key must sign the bot's master key, and only their
/// client holds that key. Nothing the bot does to itself can substitute.
///
/// Waiting for that user to type `!verify` deadlocks: a client set to refuse
/// unverified sessions blocks the very message that would start the flow.
/// Sending the request from this side breaks the deadlock, because an incoming
/// verification request needs no message from them — only an accept.
pub async fn request_verification_from_allowlist(client: &Client, allowlist: &[String]) {
    let already_done = load_verified_users(&verified_users_path());

    for user in allowlist {
        if already_done.contains(user) {
            debug!("A completed verification with {} is on record; not asking again", user);
            continue;
        }

        for attempt in 1..=IDENTITY_ATTEMPTS {
            // The request needs their cross-signing identity, which arrives
            // with the first sync.
            if !identity_is_known(client, user).await {
                debug!(
                    "No cross-signing identity for {} yet (attempt {}/{})",
                    user, attempt, IDENTITY_ATTEMPTS
                );
                tokio::time::sleep(IDENTITY_RETRY_DELAY).await;
                continue;
            }

            match request_verification_with(client, user).await {
                Ok(()) => {
                    info!("Asked {} to verify the bot — accept the prompt in your client", user)
                }
                Err(err) => warn!("Could not ask {} to verify the bot: {}", user, err),
            }
            break;
        }
    }
}

async fn identity_is_known(client: &Client, user: &str) -> bool {
    let Ok(user_id) = matrix_sdk::ruma::UserId::parse(user) else {
        warn!("MATRIX_VERIFICATION_ALLOWLIST holds an unparseable user ID: {}", user);
        return false;
    };

    client
        .encryption()
        .get_user_identity(&user_id)
        .await
        .is_ok_and(|identity| identity.is_some())
}

/// Asks `user` to verify this session.
///
/// Element shows this as an incoming verification request. The bot drives its
/// own side automatically, so the user only has to accept and confirm — which
/// is far easier than hunting for a Verify button on a bot's profile.
pub async fn request_verification_with(client: &Client, user: &str) -> Result<(), String> {
    let user_id = matrix_sdk::ruma::UserId::parse(user)
        .map_err(|e| format!("`{}` is not a valid user ID: {}", user, e))?;

    let identity = client
        .encryption()
        .get_user_identity(&user_id)
        .await
        .map_err(|e| format!("could not look up {}: {}", user, e))?
        .ok_or_else(|| {
            format!("{} has no cross-signing identity the bot can see yet", user)
        })?;

    let request = identity
        .request_verification()
        .await
        .map_err(|e| format!("could not send a verification request to {}: {}", user, e))?;

    info!("Sent a verification request to {}", user);
    let sender = user.to_string();
    tokio::spawn(async move { drive_request(request, &sender).await });
    Ok(())
}

/// Runs the short-authentication-string exchange to completion.
///
/// The emoji are logged rather than compared: there is no human on this side to
/// look at them. That is safe only because the counterparty was allowlisted —
/// see the module docs.
async fn run_sas(sas: SasVerification, sender: &str) {
    if let Err(err) = sas.accept().await {
        error!("Failed to accept SAS verification from {}: {:?}", sender, err);
        return;
    }

    let mut changes = sas.changes();
    while let Some(state) = changes.next().await {
        match state {
            SasState::KeysExchanged { emojis, decimals } => {
                if let Some(emojis) = emojis {
                    let shown: Vec<&str> = emojis.emojis.iter().map(|e| e.symbol).collect();
                    info!("Verification emoji for {}: {}", sender, shown.join(" "));
                } else {
                    info!("Verification decimals for {}: {:?}", sender, decimals);
                }

                if let Err(err) = sas.confirm().await {
                    error!("Failed to confirm SAS verification with {}: {:?}", sender, err);
                    return;
                }
            }
            SasState::Done { .. } => {
                info!("Session verified with {} 🎉", sender);
                mark_verification_complete(sender);
                break;
            }
            SasState::Cancelled(info) => {
                warn!("SAS verification with {} cancelled: {:?}", sender, info.reason());
                break;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> VerificationPolicy {
        VerificationPolicy::new(
            "@bot:mbm.one",
            vec!["@evan:mbm.one".to_string(), "@ops:mbm.one".to_string()],
        )
    }

    #[test]
    fn test_own_user_is_always_permitted() {
        assert!(policy().permits("@bot:mbm.one"));
    }

    #[test]
    fn test_allowlisted_users_are_permitted() {
        assert!(policy().permits("@evan:mbm.one"));
        assert!(policy().permits("@ops:mbm.one"));
    }

    /// Completing a verification makes the bot sign the other party's identity,
    /// so anyone not named must be refused.
    #[test]
    fn test_everyone_else_is_refused() {
        let p = policy();
        assert!(!p.permits("@stranger:mbm.one"));
        assert!(!p.permits("@evan:evil.example"));
    }

    /// A substring or prefix match would let a lookalike inherit the trust of
    /// an allowlisted account.
    #[test]
    fn test_lookalike_ids_do_not_match() {
        let p = policy();
        assert!(!p.permits("@evan-evil:mbm.one"));
        assert!(!p.permits("@evan:mbm.one.evil.example"));
        assert!(!p.permits("evan:mbm.one"));
        assert!(!p.permits("@evan"));
    }

    #[test]
    fn test_empty_allowlist_still_permits_own_user_only() {
        let p = VerificationPolicy::new("@bot:mbm.one", Vec::new());
        assert!(p.permits("@bot:mbm.one"));
        assert!(!p.permits("@evan:mbm.one"));
    }

    /// A path under the OS temp dir, unique per test run so parallel tests
    /// never share a file.
    fn scratch_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "matrix_bot_verified_users_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn test_a_fresh_path_has_no_verified_users() {
        let path = scratch_path("fresh");
        let _ = std::fs::remove_file(&path);
        assert!(load_verified_users(&path).is_empty());
    }

    #[test]
    fn test_marking_a_user_persists_across_a_reload() {
        let path = scratch_path("roundtrip");
        let _ = std::fs::remove_file(&path);

        let mut users = load_verified_users(&path);
        users.insert("@evan:mbm.one".to_string());
        save_verified_users(&path, &users);

        let reloaded = load_verified_users(&path);
        assert!(reloaded.contains("@evan:mbm.one"));

        let _ = std::fs::remove_file(&path);
    }

    /// A missing or corrupt file must read as "nobody verified yet", not as an
    /// error that blocks the bot from asking — losing this file should cost an
    /// extra prompt, never a stuck bot.
    #[test]
    fn test_a_corrupt_file_reads_as_empty_rather_than_failing() {
        let path = scratch_path("corrupt");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_verified_users(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
