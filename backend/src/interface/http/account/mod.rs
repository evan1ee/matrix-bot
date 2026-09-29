//! Account management: passkeys, linked Matrix identities, sessions, recovery.
//!
//! Almost every handler here takes [`VerifiedUser`] rather than [`AuthUser`].
//! That is the step-up boundary: a session hours old can still add a todo, but
//! anything that changes *how the account can be reached* needs a passkey
//! assertion from the last few minutes. Putting the extractor in the signature
//! is the whole opt-in, and `api::step_up_tests` — which drives the real router
//! — fails if a route in here forgets it.

pub mod audit;
pub mod links;
pub mod passkeys;
pub mod recovery;
pub mod sessions;
pub mod types;

pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::linking::LINK_CODE_TTL_MINUTES;
    use crate::accounts::session::hash_token;
    use crate::domain::accounts::{LinkOutcome, STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::events::EventBroadcaster;
    use crate::infrastructure::postgres::repositories::accounts as repo;
    use crate::infrastructure::postgres::repositories::todos as todo_repo;
    use crate::interface::http::account::{
        audit::*, links::*, passkeys::*, recovery::*, sessions::*,
    };
    use crate::interface::http::auth::extractor::VerifiedUser;
    use crate::interface::http::auth::types::{ApiState, AuthUser};
    use axum::extract::{Path, State};
    use axum::Json;

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
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
        }
    }

    #[tokio::test]
    async fn test_link_code_is_issued_and_usable_from_matrix() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;

        let res = start_link(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();
        assert_eq!(res.expires_in_seconds, LINK_CODE_TTL_MINUTES * 60);

        let outcome = state
            .services
            .accounts
            .claim_link(&res.code, "@evan:mbm.one", true)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            LinkOutcome::Linked {
                account_id: user.account_id
            }
        );

        let links = list_links(State(state), user).await.unwrap();
        assert_eq!(links.0, vec!["@evan:mbm.one".to_string()]);
    }

    #[tokio::test]
    async fn test_unlink_removes_only_your_own_link() {
        let state = state().await;
        let mine = signed_in(&state, "mine").await;
        let theirs = signed_in(&state, "theirs").await;

        repo::linking::link(&state.db.pool, &theirs.account_id, "@evan:mbm.one")
            .await
            .unwrap();

        let err = unlink(
            State(state.clone()),
            VerifiedUser(mine),
            Json(UnlinkRequest {
                mxid: "@evan:mbm.one".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);

        // Still linked to its real owner.
        assert_eq!(
            state
                .services
                .accounts
                .find_by_mxid("@evan:mbm.one")
                .await
                .unwrap(),
            Some(theirs.account_id)
        );
    }

    /// Naming another account's id must not be enough to absorb it. The pending
    /// entry — written only by a completed `!link` — is the authorisation.
    #[tokio::test]
    async fn test_a_merge_cannot_be_confirmed_without_a_pending_link() {
        let state = state().await;
        let attacker = signed_in(&state, "attacker").await;
        let victim = state
            .services
            .accounts
            .create("victim", STATUS_UNCLAIMED)
            .await
            .unwrap();

        let todo = todo_repo::create(
            &state.db.pool,
            &victim,
            &NewTodo {
                room_id: "web".to_string(),
                sender_name: "victim".to_string(),
                text: "private".to_string(),
                priority: TodoPriority::Medium,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            },
        )
        .await
        .unwrap();

        let err = confirm_merge(
            State(state.clone()),
            VerifiedUser(attacker.clone()),
            Json(ConfirmMergeRequest {
                from_account: victim.to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);

        let item = todo_repo::find_owned(&state.db.pool, &victim, todo.id)
            .await
            .unwrap();
        assert!(item.is_some(), "the victim's row must not have moved");
    }

    #[tokio::test]
    async fn test_a_pending_merge_can_be_previewed_then_confirmed() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;
        let old = state
            .services
            .accounts
            .resolve_or_provision("@evan:mbm.one")
            .await
            .unwrap();

        for text in ["one", "two"] {
            todo_repo::create(
                &state.db.pool,
                &old,
                &NewTodo {
                    room_id: "!r:x".to_string(),
                    sender_name: "evan".to_string(),
                    text: text.to_string(),
                    priority: TodoPriority::Medium,
                    due_at: None,
                    remind_before_minutes: None,
                    remind_room_id: None,
                },
            )
            .await
            .unwrap();
        }

        note_pending_merge(&state, &user.account_id, &old).await;

        let pending = pending_merge(State(state.clone()), user.clone())
            .await
            .unwrap();
        let pending = pending.0.expect("a merge should be waiting");
        assert_eq!(pending.preview.todos, 2);

        let moved = confirm_merge(
            State(state.clone()),
            VerifiedUser(user.clone()),
            Json(ConfirmMergeRequest {
                from_account: old.to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(moved.0.todos, 2);

        // Consumed, so it cannot be replayed.
        assert!(pending_merge(State(state), user).await.unwrap().0.is_none());
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
            .request_recovery(&user.account_id)
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
