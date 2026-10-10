//! Account management: passkeys, linked Matrix identities, sessions, recovery.
//!
//! Almost every handler here takes [`VerifiedUser`] rather than [`AuthUser`].
//! That is the step-up boundary: a session hours old can still add a todo, but
//! anything that changes *how the account can be reached* needs a passkey
//! assertion from the last few minutes. Putting the extractor in the signature
//! is the whole opt-in, and `api::step_up_tests` — which drives the real router
//! — fails if a route in here forgets it.

pub mod audit;
pub mod passkeys;
pub mod recovery;
pub mod sessions;
pub mod types;

pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{hash_token, AccountId, STATUS_ACTIVE};
    use crate::interface::http::account::{audit::*, passkeys::*, recovery::*, sessions::*};
    use crate::interface::http::auth::extractor::VerifiedUser;
    use crate::interface::http::auth::types::{ApiState, AuthUser};
    use axum::extract::{Path, State};

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        )
    }

    /// A caller whose session is genuinely in the database, so `session_id_hash`
    /// matches a real row.
    async fn signed_in(state: &ApiState, name: &str) -> AuthUser {
        let account = state
            .services
            .accounts
            .create(name, STATUS_ACTIVE)
            .await
            .unwrap();
        let token = state
            .services
            .accounts
            .create_session(&account, Some("Test/1.0"), None, None)
            .await
            .unwrap();

        AuthUser {
            account_id: account,
            display_name: name.to_string(),
            session_id_hash: hash_token(&token),
            last_verified_at: chrono::Utc::now(),
            restricted_until: None,
            role: crate::domain::accounts::AccountRole::User,
        }
    }

    /// The security-events list is a public contract: the web UI reads exactly
    /// these keys.
    #[test]
    fn security_event_json_shape_is_unchanged() {
        let event = crate::domain::accounts::SecurityEvent {
            id: 7,
            account_id: AccountId::from_seed("shape"),
            kind: "signed_in".to_string(),
            detail: Some("from a phone".to_string()),
            created_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
        };

        let json = serde_json::to_value(SecurityEvent::from(&event)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": 7,
                "kind": "signed_in",
                "detail": "from a phone",
                "created_at": "2026-01-02T03:04:05Z",
            })
        );
    }

    #[tokio::test]
    async fn test_the_last_passkey_cannot_be_removed_through_the_api() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;

        state
            .services
            .accounts
            .add_passkey(&user.account_id, "only", "{}", "Only key")
            .await
            .unwrap();

        let err = delete_passkey(
            State(state.clone()),
            VerifiedUser(user),
            Path("only".to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_sessions_list_marks_the_current_one() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;
        state
            .services
            .accounts
            .create_session(&user.account_id, Some("Other/1.0"), None, None)
            .await
            .unwrap();

        let sessions = list_sessions(State(state), user.clone()).await.unwrap();
        assert_eq!(sessions.0.len(), 2);

        let current: Vec<_> = sessions.0.iter().filter(|s| s.current).collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].id, user.session_id_hash);
    }

    #[tokio::test]
    async fn test_revoking_others_leaves_this_session_alone() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;
        let other = state
            .services
            .accounts
            .create_session(&user.account_id, None, None, None)
            .await
            .unwrap();

        let revoked = revoke_other_sessions(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();
        assert_eq!(revoked.0, 1);

        assert!(state
            .services
            .accounts
            .resolve_session(&other)
            .await
            .unwrap()
            .is_none());

        let left = list_sessions(State(state), user).await.unwrap();
        assert_eq!(left.0.len(), 1);
        assert!(left.0[0].current);
    }

    #[tokio::test]
    async fn test_revoking_single_session_works() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;
        let other = state
            .services
            .accounts
            .create_session(&user.account_id, None, None, None)
            .await
            .unwrap();
        let other_hash = hash_token(&other);

        let res = revoke_session(
            State(state.clone()),
            Path(other_hash),
            VerifiedUser(user.clone()),
        )
        .await
        .unwrap();
        assert!(res.0);

        assert!(state
            .services
            .accounts
            .resolve_session(&other)
            .await
            .unwrap()
            .is_none());

        let left = list_sessions(State(state), user).await.unwrap();
        assert_eq!(left.0.len(), 1);
        assert!(left.0[0].current);
    }

    #[tokio::test]
    async fn test_recovery_codes_are_returned_once_and_counted_after() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;

        let issued = regenerate_recovery_codes(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();
        assert_eq!(issued.0.codes.len(), 10);

        let status = recovery_status(State(state), user).await.unwrap();
        assert_eq!(status.0["codes_remaining"], 10);
    }

    /// Cancelling must work from an ordinary session. The person doing it may
    /// be racing a 24-hour clock, and demanding a passkey to stop a recovery
    /// they did not ask for would be exactly backwards.
    #[tokio::test]
    async fn test_cancelling_recovery_needs_no_step_up() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;

        state
            .services
            .accounts
            .request_recovery(&user.account_id, None)
            .await
            .unwrap();
        assert!(state
            .services
            .accounts
            .pending_recovery(&user.account_id)
            .await
            .unwrap()
            .is_some());

        // Deliberately a stale session: it could not pass `VerifiedUser`.
        let stale = AuthUser {
            last_verified_at: chrono::Utc::now() - chrono::Duration::hours(6),
            ..user.clone()
        };
        assert!(!stale.may_perform_sensitive_action(chrono::Utc::now()));

        let cancelled = cancel_recovery(State(state.clone()), stale).await.unwrap();
        assert_eq!(cancelled.0, 1);
        assert!(state
            .services
            .accounts
            .pending_recovery(&user.account_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_sensitive_actions_are_written_to_the_audit_log() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;

        let _ = regenerate_recovery_codes(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();
        let _ = revoke_other_sessions(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();

        let events = list_security_events(State(state), user).await.unwrap();
        let kinds: Vec<_> = events.0.iter().map(|e| e.kind.as_str()).collect();
        assert!(kinds.contains(&"recovery_codes_issued"));
        assert!(kinds.contains(&"sessions_revoked"));
    }

    /// One account must never see another's passkeys, sessions or audit log.
    #[tokio::test]
    async fn test_account_management_is_scoped_to_its_own_account() {
        let state = state().await;
        let mine = signed_in(&state, "mine").await;
        let theirs = signed_in(&state, "theirs").await;

        state
            .services
            .accounts
            .add_passkey(&theirs.account_id, "theirs", "{}", "Their key")
            .await
            .unwrap();

        state
            .services
            .accounts
            .record_security_event(
                &theirs.account_id,
                crate::domain::accounts::SecurityEventKind::SignedIn,
                None,
            )
            .await;

        assert!(list_passkeys(State(state.clone()), mine.clone())
            .await
            .unwrap()
            .0
            .is_empty());
        assert!(list_security_events(State(state.clone()), mine.clone())
            .await
            .unwrap()
            .0
            .is_empty());

        let sessions = list_sessions(State(state), mine.clone()).await.unwrap();
        assert_eq!(sessions.0.len(), 1, "only this account's own session");
    }
}
