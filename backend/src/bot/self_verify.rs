//! Making the bot's own session verified, without a human.
//!
//! Element shows a session as *Verified* when the account's self-signing key
//! has signed that device and the signature is on the homeserver. A password
//! login mints unsigned device keys, so signing is the step that has to follow
//! it — which is why a bot that logs in with the same password as a verified
//! Element session still shows up unverified.
//!
//! The private self-signing key reaches the bot through secret storage
//! (`MATRIX_RECOVERY_KEY`), which `bot/client.rs` unlocks before calling into
//! here. This module then puts that key to work on our own device.
//!
//! Signing another *user* into trust is a different problem with a different
//! answer — see [`super::verification`].

use std::time::Duration;

use matrix_sdk::Client;
use tracing::{debug, info, warn};

/// How long to keep trying to sign this device, and how long between tries.
///
/// The wait is for our own device keys to reach the server: they upload on the
/// first sync, and until the homeserver hands them back there is nothing to
/// attach a signature to.
const SIGNING_ATTEMPTS: u32 = 6;
const SIGNING_RETRY_DELAY: Duration = Duration::from_secs(5);

/// What the current trust facts call for.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// A signature by our own self-signing key is already on this device.
    Done,
    /// We hold the self-signing key and the device is unsigned — sign it.
    Sign,
    /// The device has not reached the crypto store yet; look again shortly.
    Wait,
    /// The self-signing key is not available, so nothing here can sign us.
    Unsignable,
}

/// Decides the next move from the three facts that determine it.
fn next_step(device_known: bool, cross_signed: bool, has_self_signing: bool) -> Step {
    match (device_known, cross_signed, has_self_signing) {
        (false, _, _) => Step::Wait,
        (true, true, _) => Step::Done,
        (true, false, true) => Step::Sign,
        (true, false, false) => Step::Unsignable,
    }
}

/// Signs this device with our own self-signing key, so the session shows as
/// verified without anyone confirming emoji.
///
/// The SDK already attempts this inside `Recovery::recover`, but only as a
/// conditional step — it signs when secret storage carried the self-signing key
/// *and* our device is already in the crypto store, and `recover` returns `Ok`
/// either way. Right after a fresh login the device keys are still uploading on
/// the first sync, so that attempt can find nothing to sign and the session
/// stays unverified with no error anywhere. Retrying here is what closes that
/// window.
pub async fn ensure_self_verified(client: &Client) {
    for attempt in 1..=SIGNING_ATTEMPTS {
        let has_self_signing = client
            .encryption()
            .cross_signing_status()
            .await
            .is_some_and(|status| status.has_self_signing);

        let own_device = match client.encryption().get_own_device().await {
            Ok(device) => device,
            Err(err) => {
                warn!("Could not read this device from the crypto store: {:?}", err);
                None
            }
        };

        let cross_signed = own_device.as_ref().is_some_and(|d| d.is_cross_signed_by_owner());

        debug!(
            "Self-verification attempt {}/{}: device_known={} cross_signed={} has_self_signing={}",
            attempt,
            SIGNING_ATTEMPTS,
            own_device.is_some(),
            cross_signed,
            has_self_signing
        );

        match next_step(own_device.is_some(), cross_signed, has_self_signing) {
            Step::Done => {
                info!("This device is cross-signed — Element shows the session as verified 🎉");
                return;
            }
            Step::Sign => {
                // `verify` signs the device and uploads the signature, so an
                // `Ok` here means the homeserver has it and every client that
                // trusts our identity now trusts this session.
                //
                // The outcome is taken from that upload rather than from a
                // re-read, because the local copy of our own device does not
                // pick up its fresh cross-signature: measured across restarts,
                // `is_cross_signed_by_owner` still reports false a second after
                // the signature is uploaded and re-queried, even though
                // `/keys/query` shows it. So `Done` is the rare path and this
                // one runs most boots — which is harmless, as the signature is
                // deterministic and re-uploading it is a no-op for the server.
                match own_device.expect("Sign implies a known device").verify().await {
                    Ok(()) => {
                        info!("Signed this device with our own self-signing key — the session is now verified 🎉");
                        return;
                    }
                    Err(err) => warn!(
                        "Could not sign this device (attempt {}/{}): {:?}",
                        attempt, SIGNING_ATTEMPTS, err
                    ),
                }
            }
            Step::Wait => {
                debug!(
                    "This device is not in the crypto store yet (attempt {}/{}); waiting for the first sync",
                    attempt, SIGNING_ATTEMPTS
                );
            }
            Step::Unsignable => {
                // Retrying cannot conjure the key: the only automatic source is
                // secret storage, and that import has already run by now.
                warn!(
                    "No cross-signing self-signing key is available, so nothing can sign this \
                     device and Element will show the session as Unverified. Set \
                     MATRIX_RECOVERY_KEY to the account's recovery key, and if it is already set, \
                     re-set recovery in Element (Settings → Encryption) so the current \
                     cross-signing keys are exported into secret storage."
                );
                return;
            }
        }

        tokio::time::sleep(SIGNING_RETRY_DELAY).await;
    }

    warn!(
        "Gave up signing this device after {} attempts; the session stays unverified",
        SIGNING_ATTEMPTS
    );
}

/// Whether `/keys/query` shows `device_id` signed by `user`'s self-signing key.
///
/// This is the homeserver's own answer, and the exact rule Element applies to
/// decide between a green shield and *Unverified*: the device's signature list
/// must carry an entry keyed by the public self-signing key from the same
/// response. Checking it against the server rather than the local crypto store
/// is deliberate — the store lags the signature upload, so it can still say
/// "unsigned" about a device the whole account already trusts.
pub fn device_is_cross_signed(
    keys_query: &serde_json::Value,
    user_id: &str,
    device_id: &str,
) -> bool {
    let Some(self_signing_key) = keys_query["self_signing_keys"][user_id]["keys"]
        .as_object()
        .and_then(|keys| keys.values().next())
        .and_then(|key| key.as_str())
    else {
        return false;
    };

    let Some(signatures) = keys_query["device_keys"][user_id][device_id]["signatures"][user_id]
        .as_object()
    else {
        return false;
    };

    signatures
        .keys()
        .filter_map(|signer| signer.split_once(':'))
        .any(|(algorithm, key)| algorithm == "ed25519" && key == self_signing_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_an_already_signed_device_needs_nothing() {
        assert_eq!(next_step(true, true, true), Step::Done);
    }

    /// The case that leaves a bot unverified in practice: the login succeeded,
    /// the self-signing key is in hand, and the device is simply unsigned.
    #[test]
    fn test_an_unsigned_device_gets_signed() {
        assert_eq!(next_step(true, false, true), Step::Sign);
    }

    /// Right after a fresh login our device keys are still uploading, and a
    /// device we cannot see yet must not be mistaken for an unsignable one.
    #[test]
    fn test_an_unknown_device_is_waited_for() {
        assert_eq!(next_step(false, false, false), Step::Wait);
        assert_eq!(next_step(false, false, true), Step::Wait);
    }

    /// Without the self-signing key there is no automatic path, so the loop
    /// must stop instead of retrying something that cannot change.
    #[test]
    fn test_no_self_signing_key_is_unsignable() {
        assert_eq!(next_step(true, false, false), Step::Unsignable);
    }

    /// A `/keys/query` response shaped like the real one, where `BOT` carries
    /// `signatures` naming `SELFSIGN` — the same key the account publishes as
    /// its self-signing key.
    fn keys_query(device_signatures: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "self_signing_keys": {
                "@bot:mbm.one": {
                    "usage": ["self_signing"],
                    "keys": { "ed25519:SELFSIGN": "SELFSIGN" }
                }
            },
            "device_keys": {
                "@bot:mbm.one": {
                    "BOT": { "signatures": { "@bot:mbm.one": device_signatures } }
                }
            }
        })
    }

    #[test]
    fn test_a_device_signed_by_the_self_signing_key_is_cross_signed() {
        let response = keys_query(serde_json::json!({
            "ed25519:BOT": "sig",
            "ed25519:SELFSIGN": "sig"
        }));
        assert!(device_is_cross_signed(&response, "@bot:mbm.one", "BOT"));
    }

    /// A freshly logged-in device signs itself and nothing else. That self
    /// signature is present on every device, verified or not, so it must not be
    /// mistaken for cross-signing.
    #[test]
    fn test_a_self_signature_alone_is_not_cross_signing() {
        let response = keys_query(serde_json::json!({ "ed25519:BOT": "sig" }));
        assert!(!device_is_cross_signed(&response, "@bot:mbm.one", "BOT"));
    }

    /// A signature from some other key — a stale self-signing key from before a
    /// cross-signing reset, say — does not make the device trusted today.
    #[test]
    fn test_a_signature_by_another_key_is_not_cross_signing() {
        let response = keys_query(serde_json::json!({
            "ed25519:BOT": "sig",
            "ed25519:OLDSELFSIGN": "sig"
        }));
        assert!(!device_is_cross_signed(&response, "@bot:mbm.one", "BOT"));
    }

    #[test]
    fn test_an_unknown_device_is_not_cross_signed() {
        let response = keys_query(serde_json::json!({ "ed25519:SELFSIGN": "sig" }));
        assert!(!device_is_cross_signed(&response, "@bot:mbm.one", "OTHER"));
    }

    #[test]
    fn test_an_account_without_cross_signing_is_not_cross_signed() {
        let response = serde_json::json!({ "device_keys": {}, "self_signing_keys": {} });
        assert!(!device_is_cross_signed(&response, "@bot:mbm.one", "BOT"));
    }

    /// Where the bot leaves the access token of the session it is running as.
    ///
    /// Resolved from the manifest rather than the working directory: `cargo
    /// test` runs from the package root, one level below the `data/` the bot
    /// writes from the repo root.
    fn token_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/matrix_token.json")
    }

    async fn resolved_homeserver(configured: &str) -> String {
        let domain = configured
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        if let Ok(response) = reqwest::get(format!("https://{}/.well-known/matrix/client", domain)).await
        {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                if let Some(base_url) = body["m.homeserver"]["base_url"].as_str() {
                    return base_url.trim_end_matches('/').to_string();
                }
            }
        }

        format!("https://{}", domain)
    }

    /// Asserts that the session the bot is *actually running as* is verified.
    ///
    /// This is the end-to-end check the unit tests above cannot make: it asks
    /// the homeserver about the live session and applies Element's own rule to
    /// the answer. Nothing here logs in — it reads the token the running bot
    /// saved — because logging in again on the pinned device ID would revoke
    /// that session's token out from under it.
    ///
    /// Ignored by default: it needs credentials, a network, and a bot that has
    /// been started at least once. Run it with
    /// `cargo test --package matrix-bot self_verify -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a live homeserver and a bot session on disk"]
    async fn test_the_running_bot_session_is_verified_on_the_homeserver() {
        dotenvy::dotenv().ok();

        let token = std::fs::read_to_string(token_path())
            .expect("no data/matrix_token.json — start the bot once before running this test");
        let token = token.trim().trim_matches('"');

        let configured = std::env::var("MATRIX_HOMESERVER").expect("MATRIX_HOMESERVER must be set");
        let homeserver = resolved_homeserver(&configured).await;
        let http = reqwest::Client::new();

        // Ask who this token belongs to rather than deriving it from config, so
        // the assertion lands on the session that is really running.
        let whoami: serde_json::Value = http
            .get(format!("{}/_matrix/client/v3/account/whoami", homeserver))
            .bearer_auth(token)
            .send()
            .await
            .expect("whoami request failed")
            .json()
            .await
            .expect("whoami returned no JSON");

        let user_id = whoami["user_id"]
            .as_str()
            .unwrap_or_else(|| panic!("the saved token is not valid: {}", whoami));
        let device_id = whoami["device_id"].as_str().expect("whoami returned no device_id");

        let keys: serde_json::Value = http
            .post(format!("{}/_matrix/client/v3/keys/query", homeserver))
            .bearer_auth(token)
            .json(&serde_json::json!({ "device_keys": { user_id: [] } }))
            .send()
            .await
            .expect("keys/query request failed")
            .json()
            .await
            .expect("keys/query returned no JSON");

        assert!(
            device_is_cross_signed(&keys, user_id, device_id),
            "device {} of {} is not signed by the account's self-signing key, so Element shows \
             this session as Unverified",
            device_id,
            user_id
        );
    }
}
