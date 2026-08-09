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

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::{
    encryption::verification::{SasState, SasVerification, Verification, VerificationRequestState},
    ruma::events::{
        key::verification::request::ToDeviceKeyVerificationRequestEvent,
        room::message::{MessageType, OriginalSyncRoomMessageEvent},
    },
    Client,
};
use tracing::{error, info, warn};

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
}
