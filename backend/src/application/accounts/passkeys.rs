//! Passkeys and passkey enrollment.
//!
//! A row holds a `webauthn_rs` [`Passkey`] serialised whole: the crate owns that
//! shape (public key, algorithm, signature counter, backup flags), so it is
//! stored as one blob rather than mirrored into columns. Nothing secret lives
//! there; a passkey's private half never leaves the authenticator.
//!
//! Every add, rename and removal is recorded as a security event here and only
//! here, so an interface that stores a passkey cannot forget the audit row, and
//! cannot write it a second time.

use sqlx::PgConnection;
use webauthn_rs::prelude::Passkey;

use crate::application::AppError;
use crate::domain::accounts::{AccountId, DeleteOutcome, SecurityEventKind, StoredCredential};
use crate::infrastructure::postgres::repositories::accounts::{core, credentials, security_events};

use super::AccountService;

impl AccountService {
    /// Every registered passkey row, including ones that will not deserialise,
    /// so the settings screen can show a broken row and offer its removal.
    pub async fn list_passkeys(
        &self,
        account_id: &AccountId,
    ) -> Result<Vec<StoredCredential>, AppError> {
        Ok(credentials::list_for_account(&self.pool, account_id).await?)
    }

    /// Every passkey the account can authenticate with.
    ///
    /// A row that will not deserialise is logged and skipped: one unreadable
    /// credential must not lock a user out of an account whose other passkeys
    /// are fine.
    pub async fn passkeys_for(&self, account_id: &AccountId) -> Result<Vec<Passkey>, AppError> {
        Ok(self
            .list_passkeys(account_id)
            .await?
            .iter()
            .filter_map(Self::parse_passkey)
            .collect())
    }

    /// Deserialises a passkey from its stored JSON representation.
    pub fn parse_passkey(credential: &StoredCredential) -> Option<Passkey> {
        match serde_json::from_str(&credential.passkey_json) {
            Ok(passkey) => Some(passkey),
            Err(err) => {
                tracing::error!(
                    "Credential {} will not deserialise: {:?}",
                    credential.id,
                    err
                );
                None
            }
        }
    }

    /// The stored row for the credential id an authenticator presented.
    pub async fn find_passkey_by_raw_id(
        &self,
        raw: &[u8],
    ) -> Result<Option<StoredCredential>, AppError> {
        Ok(credentials::find(&self.pool, &credentials::credential_id(raw)).await?)
    }

    /// Stores a verified passkey and records `PasskeyAdded(label)` once.
    /// Returns the credential id it was stored under.
    pub async fn register_passkey(
        &self,
        account_id: &AccountId,
        passkey: &Passkey,
        label: &str,
    ) -> Result<String, AppError> {
        let id = credentials::credential_id(passkey.cred_id().as_ref());
        let json = serialise(passkey)?;
        self.add_passkey(account_id, &id, &json, label).await?;
        Ok(id)
    }

    /// Activates (or creates) the account a registration ceremony minted an id
    /// for and stores its first passkey, in one transaction: a credential the
    /// database refuses leaves no active account that nobody can sign in to.
    pub async fn register_with_passkey(
        &self,
        account_id: &AccountId,
        display_name: &str,
        passkey: &Passkey,
        label: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        register_account_in(&mut tx, account_id, display_name, passkey, label).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stores an already-serialised passkey row and records `PasskeyAdded`.
    pub(crate) async fn add_passkey(
        &self,
        account_id: &AccountId,
        credential_id: &str,
        passkey_json: &str,
        label: &str,
    ) -> Result<(), AppError> {
        credentials::add(&self.pool, account_id, credential_id, passkey_json, label).await?;
        self.record_security_event(account_id, SecurityEventKind::PasskeyAdded, Some(label))
            .await;
        Ok(())
    }

    /// Writes back a passkey whose signature counter moved and stamps its last
    /// use. The counter is the authenticator's clone-detection signal, so a
    /// successful assertion that advanced it must be persisted.
    pub async fn record_passkey_use_of(&self, passkey: &Passkey) -> Result<(), AppError> {
        let id = credentials::credential_id(passkey.cred_id().as_ref());
        let json = serialise(passkey)?;
        Ok(credentials::record_use(&self.pool, &id, &json).await?)
    }

    pub async fn rename_passkey(
        &self,
        account_id: &AccountId,
        credential_id: &str,
        label: &str,
    ) -> Result<bool, AppError> {
        let ok = credentials::rename(&self.pool, account_id, credential_id, label).await?;
        if ok {
            self.record_security_event(account_id, SecurityEventKind::PasskeyRenamed, Some(label))
                .await;
        }
        Ok(ok)
    }

    /// Removes a passkey, refusing to remove the last one of an account with no
    /// linked Matrix identity.
    pub async fn delete_passkey(
        &self,
        account_id: &AccountId,
        credential_id: &str,
    ) -> Result<DeleteOutcome, AppError> {
        let outcome = credentials::delete(&self.pool, account_id, credential_id).await?;
        if outcome == DeleteOutcome::Removed {
            self.record_security_event(
                account_id,
                SecurityEventKind::PasskeyRemoved,
                Some(credential_id),
            )
            .await;
        }
        Ok(outcome)
    }

    pub async fn count_passkeys(&self, account_id: &AccountId) -> Result<i64, AppError> {
        Ok(credentials::count_for_account(&self.pool, account_id).await?)
    }

    /// Issues a single-use, time-limited token to enroll a first passkey.
    pub async fn issue_enrollment_token(
        &self,
        account_id: &AccountId,
        ttl_minutes: i64,
    ) -> Result<String, AppError> {
        Ok(credentials::issue_enrollment_token(&self.pool, account_id, ttl_minutes).await?)
    }

    /// Consumes an enrollment token, returning the account it was issued for.
    /// `None` = unknown, expired, already used, or the account has a passkey.
    pub async fn validate_and_consume_enrollment_token(
        &self,
        token: &str,
    ) -> Result<Option<AccountId>, AppError> {
        Ok(credentials::validate_and_consume_enrollment_token(&self.pool, token).await?)
    }
}

/// A serialisation failure is our own fault, not an outside dependency's, so it
/// is reported as `Database` (protocol error), the variant `app_error.rs` maps
/// to an opaque 500 with the detail logged; the client never sees it.
/// [`AccountService::register_with_passkey`] on a connection the caller owns,
/// so a flow that must also do something else atomically (the link-signup
/// finish binds the identity too) can share the steps. Records
/// `PasskeyAdded(label)` once, like `register_passkey`.
pub(crate) async fn register_account_in(
    conn: &mut PgConnection,
    account_id: &AccountId,
    display_name: &str,
    passkey: &Passkey,
    label: &str,
) -> Result<(), AppError> {
    let id = credentials::credential_id(passkey.cred_id().as_ref());
    let json = serialise(passkey)?;
    core::activate_or_create(&mut *conn, account_id, display_name).await?;
    credentials::add(&mut *conn, account_id, &id, &json, label).await?;
    security_events::record(
        &mut *conn,
        account_id,
        SecurityEventKind::PasskeyAdded,
        Some(label),
    )
    .await;
    Ok(())
}

fn serialise(passkey: &Passkey) -> Result<String, AppError> {
    serde_json::to_string(passkey).map_err(|err| {
        AppError::Database(sqlx::Error::Protocol(format!(
            "could not serialise passkey: {err}"
        )))
    })
}

/// A structurally valid passkey. Building one through a ceremony needs an
/// authenticator; the service only cares about its id and that it
/// round-trips through JSON.
#[cfg(test)]
pub(crate) fn test_passkey(raw_id: &[u8], counter: u32) -> Passkey {
    use base64::Engine;
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    serde_json::from_value(serde_json::json!({
        "cred": {
            "cred_id": b64(raw_id),
            "cred": {
                "type_": "ES256",
                "key": { "EC_EC2": { "curve": "SECP256R1", "x": b64(&[1; 32]), "y": b64(&[2; 32]) } }
            },
            "counter": counter,
            "transports": null,
            "user_verified": true,
            "backup_eligible": false,
            "backup_state": false,
            "registration_policy": "required",
            "extensions": {
                "cred_protect": "NotRequested",
                "hmac_create_secret": "NotRequested",
                "appid": "NotRequested",
                "cred_props": "Ignored"
            },
            "attestation": { "data": "None", "metadata": "None" },
            "attestation_format": "none"
        }
    }))
    .expect("the fixture must be a valid passkey")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{SecurityEvent, STATUS_ACTIVE};

    async fn service() -> AccountService {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        AccountService::new(pool)
    }

    use super::test_passkey as passkey;

    /// The account's audit trail, oldest first, as `(kind, detail)`.
    async fn audit(service: &AccountService, account: &AccountId) -> Vec<(String, Option<String>)> {
        let mut events: Vec<SecurityEvent> =
            service.list_security_events(account, 50).await.unwrap();
        events.reverse();
        events.into_iter().map(|e| (e.kind, e.detail)).collect()
    }

    fn row(kind: &str, detail: &str) -> (String, Option<String>) {
        (kind.to_string(), Some(detail.to_string()))
    }

    #[tokio::test]
    async fn registering_a_passkey_records_passkey_added_once() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();

        let id = service
            .register_passkey(&account, &passkey(&[7, 7, 7], 0), "MacBook")
            .await
            .unwrap();

        assert_eq!(id, credentials::credential_id(&[7, 7, 7]));
        assert_eq!(
            audit(&service, &account).await,
            vec![row("passkey_added", "MacBook")]
        );
    }

    /// First passkey of a new account, a second added later, then one removed:
    /// the trail is exactly one row per action, in order.
    #[tokio::test]
    async fn adding_and_removing_passkeys_leaves_one_audit_row_each() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();

        let first = service
            .register_passkey(&account, &passkey(&[1], 0), "MacBook")
            .await
            .unwrap();
        let second = service
            .register_passkey(&account, &passkey(&[2], 0), "iPhone")
            .await
            .unwrap();
        assert_eq!(
            service.delete_passkey(&account, &first).await.unwrap(),
            DeleteOutcome::Removed
        );

        assert_eq!(
            audit(&service, &account).await,
            vec![
                row("passkey_added", "MacBook"),
                row("passkey_added", "iPhone"),
                row("passkey_removed", &first),
            ]
        );
        assert_eq!(service.count_passkeys(&account).await.unwrap(), 1);
        assert_eq!(service.list_passkeys(&account).await.unwrap()[0].id, second);
    }

    /// A refused removal and a missed rename change nothing, so they log nothing.
    #[tokio::test]
    async fn a_refused_removal_or_missed_rename_records_nothing() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        let only = service
            .register_passkey(&account, &passkey(&[1], 0), "MacBook")
            .await
            .unwrap();

        assert_eq!(
            service.delete_passkey(&account, &only).await.unwrap(),
            DeleteOutcome::WouldLockOut
        );
        assert!(!service
            .rename_passkey(&account, "no-such-credential", "x")
            .await
            .unwrap());

        assert_eq!(
            audit(&service, &account).await,
            vec![row("passkey_added", "MacBook")]
        );
    }

    /// The enrollment path: issuing and consuming the token writes no audit
    /// row of its own; the one row is the registration that follows.
    #[tokio::test]
    async fn first_enrollment_through_a_token_leaves_a_single_audit_row() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();

        let token = service.issue_enrollment_token(&account, 10).await.unwrap();
        assert_eq!(audit(&service, &account).await, vec![]);

        let consumed = service
            .validate_and_consume_enrollment_token(&token)
            .await
            .unwrap();
        assert_eq!(consumed, Some(account));
        assert_eq!(audit(&service, &account).await, vec![]);

        service
            .register_passkey(&account, &passkey(&[9], 0), "Passkey")
            .await
            .unwrap();
        assert_eq!(
            audit(&service, &account).await,
            vec![row("passkey_added", "Passkey")]
        );

        // Single-use, and once a passkey exists no new token redeems either.
        assert_eq!(
            service
                .validate_and_consume_enrollment_token(&token)
                .await
                .unwrap(),
            None
        );
        let late = service.issue_enrollment_token(&account, 10).await.unwrap();
        assert_eq!(
            service
                .validate_and_consume_enrollment_token(&late)
                .await
                .unwrap(),
            None,
            "a token cannot enroll a second passkey"
        );
    }

    #[tokio::test]
    async fn an_expired_enrollment_token_is_refused() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();

        let token = service.issue_enrollment_token(&account, -1).await.unwrap();

        assert_eq!(
            service
                .validate_and_consume_enrollment_token(&token)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn an_unserialisable_passkey_row_is_skipped() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        service
            .add_passkey(&account, "cred_broken", "{}", "test key")
            .await
            .unwrap();
        service
            .register_passkey(&account, &passkey(&[4], 0), "good")
            .await
            .unwrap();

        let usable = service.passkeys_for(&account).await.unwrap();
        assert_eq!(usable.len(), 1, "the unparseable row is skipped");
        assert_eq!(AsRef::<[u8]>::as_ref(usable[0].cred_id()), &[4]);

        // The row is still listed, so the UI can show it and offer removal.
        assert_eq!(service.list_passkeys(&account).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_presented_credential_id_finds_its_row_and_use_is_recorded() {
        let service = service().await;
        let account = service.create("evan", STATUS_ACTIVE).await.unwrap();
        service
            .register_passkey(&account, &passkey(&[5, 6], 0), "MacBook")
            .await
            .unwrap();

        let found = service
            .find_passkey_by_raw_id(&[5, 6])
            .await
            .unwrap()
            .expect("the registered credential id resolves");
        assert_eq!(found.account_id, account);
        assert!(found.last_used_at.is_none());
        assert!(service
            .find_passkey_by_raw_id(&[9, 9])
            .await
            .unwrap()
            .is_none());

        let moved = passkey(&[5, 6], 42);
        service.record_passkey_use_of(&moved).await.unwrap();

        let after = service
            .find_passkey_by_raw_id(&[5, 6])
            .await
            .unwrap()
            .unwrap();
        assert!(after.last_used_at.is_some());
        assert!(after.passkey_json.contains("\"counter\":42"));
        // Signing in is not part of the passkey lifecycle audit trail.
        assert_eq!(
            audit(&service, &account).await,
            vec![row("passkey_added", "MacBook")]
        );
    }

    #[tokio::test]
    async fn registering_an_account_with_its_passkey_is_one_atomic_step() {
        let service = service().await;
        let id = AccountId::generate();
        service
            .register_with_passkey(&id, "Evan", &passkey(&[3, 3], 0), "Laptop")
            .await
            .unwrap();

        let account = service.find(&id).await.unwrap().expect("account created");
        assert_eq!(account.display_name, "Evan");
        assert_eq!(service.list_passkeys(&id).await.unwrap().len(), 1);
        assert_eq!(
            audit(&service, &id).await,
            vec![row("passkey_added", "Laptop")]
        );
    }

    /// The old two-call sequence left an active account without a passkey when
    /// the second call failed; now a rejected passkey leaves nothing behind.
    #[tokio::test]
    async fn a_rejected_passkey_leaves_no_account_behind() {
        let service = service().await;
        let first = AccountId::generate();
        service
            .register_with_passkey(&first, "First", &passkey(&[4, 4], 0), "Key")
            .await
            .unwrap();

        let second = AccountId::generate();
        let err = service
            .register_with_passkey(&second, "Second", &passkey(&[4, 4], 0), "Key")
            .await;
        assert!(err.is_err(), "a credential id is stored once");
        assert!(service.find(&second).await.unwrap().is_none());
    }
}
