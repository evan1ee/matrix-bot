use anyhow::{Context, Result};
use matrix_sdk::{
    ruma::{OwnedUserId, ServerName, UserId},
    Client,
};
use std::time::Duration;
use tracing::{info, warn};

use crate::bot::self_verify::ensure_self_verified;
use crate::bot::verification::request_verification_from_allowlist;
use crate::config::Config;

/// Where the full Matrix session is persisted between runs.
///
/// The sqlite store keeps crypto state but not the session, so without this the
/// bot password-logs-in on every boot. Every such login mints a brand new
/// device (see `delete_stale_device`), so skipping this on an ordinary restart
/// is what keeps the account from accumulating one device per boot.
const SESSION_PATH: &str = "data/matrix_session.json";

/// Where the matrix-sdk sqlite stores (crypto + room state) live.
const STORE_PATH: &str = "data/bot_store";

pub struct BotClient;

/// Builds a client against the configured homeserver, backed by the sqlite
/// store. Tries `.well-known` discovery first, then the bare URL.
async fn build_client(homeserver_input: &str, clean_domain: &str) -> Result<Client> {
    if let Ok(server_name) = ServerName::parse(clean_domain) {
        if let Ok(c) = Client::builder()
            .server_name(&server_name)
            .sqlite_store(STORE_PATH, None)
            .build()
            .await
        {
            return Ok(c);
        }
    }

    let fallback_url = if homeserver_input.starts_with("http") {
        homeserver_input.to_string()
    } else {
        format!("https://{}", clean_domain)
    };

    Client::builder()
        .homeserver_url(&fallback_url)
        .sqlite_store(STORE_PATH, None)
        .build()
        .await
        .context("Failed to build Matrix client")
}

/// Writes `contents` to `path`, owner-readable only where the OS supports it.
fn write_private(path: &str, contents: &[u8]) {
    std::fs::create_dir_all("data").ok();
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        if let Ok(mut f) = options.open(path) {
            let _ = f.write_all(contents);
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents).ok();
    }
}

/// Reads a previously persisted session, if one is on disk and still parses.
fn load_session() -> Option<matrix_sdk::matrix_auth::MatrixSession> {
    let raw = std::fs::read_to_string(SESSION_PATH).ok()?;
    match serde_json::from_str(&raw) {
        Ok(session) => Some(session),
        Err(err) => {
            warn!("Ignoring unreadable session at {}: {}", SESSION_PATH, err);
            None
        }
    }
}

/// Moves the sqlite stores aside, returning the archive path if there was one.
///
/// The crypto store holds the Olm account — the device's identity keys — and is
/// only meaningful for the device it was created with. Once that device is gone
/// from the homeserver (the user deleted the session, or the token was revoked)
/// the keys in it are orphaned: a fresh login re-creates the device ID but the
/// crypto machine still believes it has already published its keys, so it never
/// uploads them again. Everything the bot then encrypts arrives at other
/// clients signed by a device the homeserver knows nothing about, which Element
/// reports as "Encrypted by an unknown or deleted device".
///
/// The store is archived rather than deleted because it may hold room keys that
/// were never uploaded to key backup, and those are the only copy.
fn archive_store(suffix: &str) -> Option<String> {
    if !std::path::Path::new(STORE_PATH).exists() {
        return None;
    }

    let archived = format!("{}.orphaned-{}", STORE_PATH, suffix);
    match std::fs::rename(STORE_PATH, &archived) {
        Ok(()) => Some(archived),
        Err(err) => {
            warn!("Could not move the stale crypto store aside: {}", err);
            None
        }
    }
}

/// Timestamp suffix for archived stores, unique to the second.
fn archive_suffix() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Removes `device_id` from the account via password-authenticated UIA.
///
/// Called after a fresh login, on the account's *previous* device — the one
/// whose crypto store was just archived because its session no longer
/// restored. Every peer who already talked to that device still has it
/// cross-signed and holds an Olm session keyed to its identity keys. Simply
/// logging in again and leaving that device on the server would let a new
/// device silently inherit its old device ID with different keys underneath,
/// which every peer's client then encrypts to as if nothing changed — the
/// homeserver signals the key change, but a client only rebuilds a peer's Olm
/// session by *receiving* a decrypt failure against it, and the peer who is
/// still encrypting *to* the swapped device never gets one to react to. It
/// silently produces ciphertext the new device can't read, and there is no
/// event that would prompt the peer's client to notice on its own.
///
/// Deleting the old device instead makes the replacement an unambiguous *new*
/// device: every peer sees it appear fresh, verifies it through the normal new
/// device flow, and establishes a new Olm session from scratch — the well-worn
/// path, rather than the in-place key swap most clients don't handle.
///
/// Best-effort: this is cleanup, not a precondition for the bot to run, so a
/// failure here is logged and otherwise ignored.
async fn delete_stale_device(client: &Client, config: &Config, device_id: &matrix_sdk::ruma::OwnedDeviceId) {
    use matrix_sdk::ruma::api::client::uiaa;

    let devices = [device_id.clone()];
    let first_attempt = client.delete_devices(&devices, None).await;

    let Err(err) = first_attempt else {
        info!("Deleted the previous device {} from the account", device_id);
        return;
    };

    let Some(uiaa_info) = err.as_uiaa_response() else {
        warn!("Could not delete the previous device {}: {:?}", device_id, err);
        return;
    };

    let mut password = uiaa::Password::new(
        uiaa::UserIdentifier::UserIdOrLocalpart(config.username.clone()),
        config.password.clone(),
    );
    password.session = uiaa_info.session.clone();

    match client.delete_devices(&devices, Some(uiaa::AuthData::Password(password))).await {
        Ok(_) => info!("Deleted the previous device {} from the account", device_id),
        Err(err) => warn!(
            "Could not delete the previous device {} even with password auth: {:?}. It will \
             stay on the account; peers who talked to it will need to re-verify the new device.",
            device_id, err
        ),
    }
}

/// The MXID to log in as, whether `username` is a full ID or a localpart.
fn resolve_user_id(username: &str, clean_domain: &str) -> Result<OwnedUserId> {
    if username.starts_with('@') {
        UserId::parse(username).context("Failed to parse MATRIX_USERNAME as a valid Matrix User ID")
    } else {
        let raw_id = format!("@{}:{}", username, clean_domain);
        UserId::parse(&raw_id).context("Failed to construct valid Matrix User ID")
    }
}

/// Restores the persisted session onto `client` and proves it is still live.
///
/// `restore_session` installs the session even when its token is dead, so the
/// client then *looks* logged in; only a round trip to the homeserver tells the
/// two apart. A session that fails here is deleted, because the caller's next
/// step is a fresh login that will replace it.
async fn restore_previous_session(client: &Client) -> bool {
    let Some(session) = load_session() else {
        return false;
    };

    if let Err(err) = client.matrix_auth().restore_session(session).await {
        warn!("Could not restore the stored session ({}); logging in again", err);
        let _ = std::fs::remove_file(SESSION_PATH);
        return false;
    }

    match client.whoami().await {
        Ok(who) => {
            info!(
                "Reusing stored Matrix session for {} on device {}",
                who.user_id,
                who.device_id.map(|d| d.to_string()).unwrap_or_default()
            );
            true
        }
        Err(err) => {
            warn!("Stored session was rejected by the homeserver ({}); discarding it and logging in again", err);
            let _ = std::fs::remove_file(SESSION_PATH);
            false
        }
    }
}

/// Pulls this account's room keys out of the server-side key backup.
///
/// The recovery key unlocks the backup, but the SDK does not fetch from it on
/// its own — so after a fresh crypto store the bot holds no Megolm sessions and
/// silently fails to decrypt every message. Downloading them is what turns a
/// working recovery key into a bot that can actually read its rooms.
async fn restore_room_keys_from_backup(client: &Client) {
    // Recovery has only just imported the backup key, and rooms are not known
    // until the first sync lands. Give both a chance rather than giving up.
    for attempt in 0..12u32 {
        tokio::time::sleep(Duration::from_secs(5)).await;

        if !client.encryption().backups().are_enabled().await {
            continue;
        }

        let rooms = client.joined_rooms();
        if rooms.is_empty() {
            continue;
        }

        let mut restored = 0usize;
        let mut failed = 0usize;
        for room in rooms {
            match client
                .encryption()
                .backups()
                .download_room_keys_for_room(room.room_id())
                .await
            {
                Ok(()) => restored += 1,
                Err(err) => {
                    failed += 1;
                    warn!("Could not restore keys for {}: {:?}", room.room_id(), err);
                }
            }
        }

        info!(
            "Restored room keys from backup for {} room(s) ({} failed) after {} attempt(s)",
            restored,
            failed,
            attempt + 1
        );
        return;
    }

    warn!("Key backup never became usable; the bot may be unable to decrypt existing rooms");
}

fn save_session(client: &Client) {
    if let Some(session) = client.matrix_auth().session() {
        match serde_json::to_vec(&session) {
            Ok(bytes) => write_private(SESSION_PATH, &bytes),
            Err(err) => warn!("Could not serialise the Matrix session: {}", err),
        }
    }
}

impl BotClient {
    pub async fn build_and_login_with_token_handles(
        config: &Config,
        token_lock: Option<std::sync::Arc<tokio::sync::RwLock<Option<String>>>>,
        client_lock: Option<std::sync::Arc<tokio::sync::RwLock<Option<Client>>>>,
    ) -> Result<Client> {
        let homeserver_input = &config.homeserver;
        let clean_domain = homeserver_input
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        // Ensure data directory exists for matrix-sdk sqlite store
        tokio::fs::create_dir_all("data").await.ok();

        // Captured before the restore attempt, which deletes the session file
        // on failure — this is the only chance to learn which device is about
        // to be orphaned, so it can be cleaned up once the new one is live.
        let stale_device_id = load_session().map(|s| s.meta.device_id);

        let existing = build_client(homeserver_input, clean_domain).await?;
        let client = if restore_previous_session(&existing).await {
            existing
        } else {
            // No live session means the device this store was built for is no
            // longer usable, so its crypto identity goes with it. Drop the
            // client first: the sqlite files must not be moved while it holds
            // them open.
            drop(existing);

            let user_id = resolve_user_id(&config.username, clean_domain)?;
            match archive_store(&archive_suffix()) {
                Some(path) => warn!(
                    "Logging in fresh, so the crypto store from the previous device is orphaned; moved it to {}",
                    path
                ),
                None => info!("No previous crypto store; starting a new one"),
            }
            if config.recovery_key.is_none() {
                warn!("MATRIX_RECOVERY_KEY is not set, so the new device starts with no room keys and cannot decrypt older messages");
            }

            info!("No reusable Matrix session. Logging in as user: {}", user_id);
            let client = build_client(homeserver_input, clean_domain).await?;
            client
                .matrix_auth()
                .login_username(&user_id, &config.password)
                // No pinned device ID: reusing one across a genuine key change
                // is exactly what leaves every peer's Olm session silently
                // stale — see `delete_stale_device`. Letting the homeserver
                // assign a fresh ID makes this an unambiguous new device that
                // every peer verifies and re-sessions through the normal path.
                .initial_device_display_name("Rust Matrix Bot")
                .send()
                .await
                .context("Failed to log in to Matrix server")?;

            info!("Successfully logged in as {}", user_id);
            save_session(&client);

            if let Some(old_device_id) = stale_device_id {
                let client_clone = client.clone();
                let config_clone = config.clone();
                tokio::spawn(async move {
                    delete_stale_device(&client_clone, &config_clone, &old_device_id).await;
                });
            }

            client
        };

        if let Some(ref lock) = client_lock {
            *lock.write().await = Some(client.clone());
        }

        if let Some(tok) = client.access_token() {
            write_private("data/matrix_token.json", tok.as_bytes());

            if let Some(ref lock) = token_lock {
                *lock.write().await = Some(tok);
            }
        }

        // Secret storage comes first because it is where the cross-signing keys
        // come from on a fresh store, but signing runs either way: a restored
        // store already holds them, and then no recovery key is needed to make
        // the session verified.
        let recovery_key = config
            .recovery_key
            .as_ref()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        let allowlist = config.verification_allowlist.clone();
        let client_clone = client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;

            let mut secrets_imported = false;
            if let Some(key) = recovery_key {
                info!("Opening secret storage with MATRIX_RECOVERY_KEY...");
                match client_clone.encryption().recovery().recover(&key).await {
                    Ok(_) => secrets_imported = true,
                    Err(err) => warn!(
                        "MATRIX_RECOVERY_KEY did not open secret storage: {:?}. No room keys can \
                         be restored, and a fresh crypto store has no cross-signing keys to sign \
                         this device with.",
                        err
                    ),
                }
            }

            ensure_self_verified(&client_clone).await;

            // Signing our own device covers trust inside this account only. An
            // allowlisted user has to sign the bot's identity from their side,
            // and their client will refuse to send the `!verify` that would
            // start it, so the bot offers first.
            request_verification_from_allowlist(&client_clone, &allowlist).await;

            if secrets_imported {
                restore_room_keys_from_backup(&client_clone).await;
            }
        });

        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_localpart_username_gets_the_homeserver_domain() {
        let id = resolve_user_id("bot", "mbm.one").unwrap();
        assert_eq!(id.as_str(), "@bot:mbm.one");
    }

    /// A full MXID must be used verbatim: its server part may differ from the
    /// homeserver the bot connects to.
    #[test]
    fn test_full_mxid_username_is_used_as_is() {
        let id = resolve_user_id("@bot:other.example", "mbm.one").unwrap();
        assert_eq!(id.as_str(), "@bot:other.example");
    }

    #[test]
    fn test_unparseable_username_is_an_error() {
        assert!(resolve_user_id("@no-domain", "mbm.one").is_err());
    }

    #[test]
    fn test_archive_suffix_is_filename_safe() {
        let suffix = archive_suffix();
        assert!(suffix.chars().all(|c| c.is_ascii_digit() || c == '-'), "{}", suffix);
    }

    /// Archiving must never destroy the old store: it can hold the only copy of
    /// room keys that were never backed up.
    #[test]
    fn test_archive_store_moves_the_store_aside() {
        let _guard = STORE_LOCK.lock().unwrap();
        let _cwd = TempCwd::new();

        std::fs::create_dir_all(STORE_PATH).unwrap();
        std::fs::write(format!("{}/matrix-sdk-crypto.sqlite3", STORE_PATH), b"keys").unwrap();

        let archived = archive_store("testsuffix").expect("store should have been archived");

        assert_eq!(archived, format!("{}.orphaned-testsuffix", STORE_PATH));
        assert!(!std::path::Path::new(STORE_PATH).exists());
        assert_eq!(
            std::fs::read(format!("{}/matrix-sdk-crypto.sqlite3", archived)).unwrap(),
            b"keys"
        );
    }

    #[test]
    fn test_archive_store_is_a_no_op_without_a_store() {
        let _guard = STORE_LOCK.lock().unwrap();
        let _cwd = TempCwd::new();

        assert!(archive_store("testsuffix").is_none());
    }

    /// The store paths are relative, so these tests change the process-wide
    /// working directory and must not run concurrently.
    static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A scratch working directory, restored on drop.
    struct TempCwd {
        previous: std::path::PathBuf,
        dir: std::path::PathBuf,
    }

    impl TempCwd {
        fn new() -> Self {
            let previous = std::env::current_dir().unwrap();
            let dir = std::env::temp_dir().join(format!(
                "matrix-bot-store-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_current_dir(&dir).unwrap();
            Self { previous, dir }
        }
    }

    impl Drop for TempCwd {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous).unwrap();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}
