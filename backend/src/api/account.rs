//! Account management: passkeys, linked Matrix identities, sessions, recovery.
//!
//! Almost everything here takes [`VerifiedUser`] rather than [`AuthUser`].
//! That is the step-up boundary: a session hours old can still add a todo, but
//! anything that changes *how the account can be reached* needs a passkey
//! assertion from the last few minutes. Putting the extractor in the signature
//! is the whole opt-in, and `test_every_sensitive_route_demands_step_up` fails
//! if a new route here forgets it.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::accounts::security_events::SecurityEvent;
use crate::accounts::{
    credentials::DeleteOutcome, linking::LINK_CODE_TTL_MINUTES, AccountRepository,
    CredentialRepository, LinkRepository, MergePreview, MergeRepository,
    RecoveryRepository, SecurityEventKind, SecurityEventRepository, SessionRepository,
};
use crate::api::auth::extractor::VerifiedUser;
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;

#[derive(Debug, Serialize, ToSchema)]
pub struct PasskeySummary {
    pub id: String,
    pub label: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionSummary {
    /// The session's storage key. Safe to expose — it is a hash, not the
    /// cookie, so it cannot be replayed.
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub user_agent: Option<String>,
    /// Whether this is the session making the request.
    pub current: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkCodeResponse {
    pub code: String,
    pub expires_in_seconds: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnlinkRequest {
    pub mxid: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PendingMerge {
    pub from_account: String,
    pub preview: MergePreview,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfirmMergeRequest {
    pub from_account: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    /// Shown exactly once. The server keeps only hashes.
    pub codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenamePasskeyRequest {
    pub label: String,
}

// ---------------------------------------------------------------- passkeys --

#[utoipa::path(
    get,
    path = "/api/v1/account/passkeys",
    responses((status = 200, body = Vec<PasskeySummary>)),
    security(("session_cookie" = []))
)]
pub async fn list_passkeys(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<PasskeySummary>>, ApiError> {
    let stored = CredentialRepository::list_for_account(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(
        stored
            .into_iter()
            .map(|c| PasskeySummary {
                id: c.id,
                label: c.label,
                created_at: c.created_at,
                last_used_at: c.last_used_at,
            })
            .collect(),
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/account/passkeys/{id}",
    request_body = RenamePasskeyRequest,
    responses((status = 204, description = "Renamed")),
    security(("session_cookie" = []))
)]
pub async fn rename_passkey(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Path(id): Path<String>,
    Json(payload): Json<RenamePasskeyRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let label = payload.label.trim();
    if label.is_empty() || label.len() > 64 {
        return Err(ApiError::bad_request("A passkey name must be 1-64 characters"));
    }

    let renamed = CredentialRepository::rename(&state.db.pool, &user.account_id, &id, label)
        .await
        .map_err(ApiError::from)?;

    if !renamed {
        return Err(ApiError::not_found("No such passkey"));
    }

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::PasskeyRenamed,
        Some(label),
    )
    .await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/v1/account/passkeys/{id}",
    responses(
        (status = 204, description = "Removed"),
        (status = 409, description = "Would lock the account out", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_passkey(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    match CredentialRepository::delete(&state.db.pool, &user.account_id, &id)
        .await
        .map_err(ApiError::from)?
    {
        DeleteOutcome::Removed => {
            SecurityEventRepository::record(
                &state.db.pool,
                &user.account_id,
                SecurityEventKind::PasskeyRemoved,
                None,
            )
            .await;
            Ok(axum::http::StatusCode::NO_CONTENT)
        }
        DeleteOutcome::NotFound => Err(ApiError::not_found("No such passkey")),
        DeleteOutcome::WouldLockOut => Err(ApiError::conflict(
            "That is your only passkey. Add another before removing this one.",
        )),
    }
}

// ------------------------------------------------------------ matrix links --

#[utoipa::path(
    get,
    path = "/api/v1/account/matrix",
    responses((status = 200, body = Vec<String>)),
    security(("session_cookie" = []))
)]
pub async fn list_links(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(
        AccountRepository::linked_mxids(&state.db.pool, &user.account_id)
            .await
            .map_err(ApiError::from)?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/account/matrix/link",
    responses((status = 200, body = LinkCodeResponse)),
    security(("session_cookie" = []))
)]
pub async fn start_link(
    State(state): State<ApiState>,
    user: VerifiedUser,
) -> Result<Json<LinkCodeResponse>, ApiError> {
    let code = LinkRepository::issue(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(LinkCodeResponse {
        code,
        expires_in_seconds: LINK_CODE_TTL_MINUTES * 60,
    }))
}

/// Whether a merge is waiting on this account's confirmation.
///
/// Set when `!link` found the MXID already attached to an unclaimed account
/// holding data — the user is told to come back here, where the counts can
/// actually be shown before they agree to move anything.
#[utoipa::path(
    get,
    path = "/api/v1/account/matrix/pending-merge",
    responses((status = 200, body = Option<PendingMerge>)),
    security(("session_cookie" = []))
)]
pub async fn pending_merge(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Option<PendingMerge>>, ApiError> {
    let pending = state.pending_merges.read().await.get(&user.account_id).cloned();

    let Some(from_account) = pending else {
        return Ok(Json(None));
    };

    let preview = MergeRepository::preview(&state.db.pool, &from_account)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(Some(PendingMerge { from_account, preview })))
}

#[utoipa::path(
    post,
    path = "/api/v1/account/matrix/confirm-merge",
    request_body = ConfirmMergeRequest,
    responses((status = 200, body = MergePreview)),
    security(("session_cookie" = []))
)]
pub async fn confirm_merge(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Json(payload): Json<ConfirmMergeRequest>,
) -> Result<Json<MergePreview>, ApiError> {
    // The pending entry is the authorisation. Without this check any account
    // could name any other account id and absorb it.
    let authorised = state
        .pending_merges
        .read()
        .await
        .get(&user.account_id)
        .is_some_and(|from| *from == payload.from_account);

    if !authorised {
        return Err(ApiError::not_found("No merge is waiting for confirmation"));
    }

    let outcome = MergeRepository::merge(&state.db.pool, &payload.from_account, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    state.pending_merges.write().await.remove(&user.account_id);

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::AccountsMerged,
        Some(&format!("{} rows", outcome.moved.total())),
    )
    .await;

    Ok(Json(outcome.moved))
}

#[utoipa::path(
    post,
    path = "/api/v1/account/matrix/unlink",
    request_body = UnlinkRequest,
    responses((status = 204, description = "Unlinked")),
    security(("session_cookie" = []))
)]
pub async fn unlink(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Json(payload): Json<UnlinkRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let removed = LinkRepository::unlink(&state.db.pool, &user.account_id, &payload.mxid)
        .await
        .map_err(ApiError::from)?;

    if !removed {
        return Err(ApiError::not_found("That Matrix account is not linked here"));
    }

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::MatrixUnlinked,
        Some(&payload.mxid),
    )
    .await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- sessions --

#[utoipa::path(
    get,
    path = "/api/v1/account/sessions",
    responses((status = 200, body = Vec<SessionSummary>)),
    security(("session_cookie" = []))
)]
pub async fn list_sessions(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<SessionSummary>>, ApiError> {
    let sessions = SessionRepository::list_for_account(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionSummary {
                current: s.id_hash == user.session_id_hash,
                id: s.id_hash,
                created_at: s.created_at,
                last_seen_at: s.last_seen_at,
                user_agent: s.user_agent,
            })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/account/sessions/revoke-others",
    responses((status = 200, description = "How many were signed out")),
    security(("session_cookie" = []))
)]
pub async fn revoke_other_sessions(
    State(state): State<ApiState>,
    user: VerifiedUser,
) -> Result<Json<u64>, ApiError> {
    let revoked = SessionRepository::revoke_all_for_account(
        &state.db.pool,
        &user.account_id,
        Some(&user.session_id_hash),
    )
    .await
    .map_err(ApiError::from)?;

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::SessionsRevoked,
        Some(&format!("{revoked} session(s)")),
    )
    .await;

    Ok(Json(revoked))
}

// ---------------------------------------------------------------- recovery --

#[utoipa::path(
    post,
    path = "/api/v1/account/recovery-codes",
    responses((status = 200, body = RecoveryCodesResponse)),
    security(("session_cookie" = []))
)]
pub async fn regenerate_recovery_codes(
    State(state): State<ApiState>,
    user: VerifiedUser,
) -> Result<Json<RecoveryCodesResponse>, ApiError> {
    let codes = RecoveryRepository::issue_codes(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::RecoveryCodesIssued,
        None,
    )
    .await;

    Ok(Json(RecoveryCodesResponse { codes }))
}

#[utoipa::path(
    get,
    path = "/api/v1/account/recovery-status",
    responses((status = 200, description = "Codes left, and any recovery counting down")),
    security(("session_cookie" = []))
)]
pub async fn recovery_status(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    let remaining = RecoveryRepository::remaining(&state.db.pool, &user.account_id)
        .await
        .unwrap_or(0);
    let pending = RecoveryRepository::pending(&state.db.pool, &user.account_id)
        .await
        .unwrap_or(None);

    Ok(Json(serde_json::json!({
        "codes_remaining": remaining,
        "recovery_valid_after": pending,
    })))
}

/// Stops a recovery someone started from a linked Matrix account.
///
/// Deliberately takes [`AuthUser`], not [`VerifiedUser`]. Everything else here
/// is gated on a fresh passkey, but this is the *defence* against a recovery
/// nobody asked for — demanding a passkey to cancel would be exactly backwards
/// when the person cancelling may be racing a 24-hour clock on their phone.
#[utoipa::path(
    post,
    path = "/api/v1/account/recovery/cancel",
    responses((status = 200, description = "How many requests were cancelled")),
    security(("session_cookie" = []))
)]
pub async fn cancel_recovery(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<u64>, ApiError> {
    let cancelled = RecoveryRepository::cancel_all(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    if cancelled > 0 {
        SecurityEventRepository::record(
            &state.db.pool,
            &user.account_id,
            SecurityEventKind::RecoveryCancelled,
            None,
        )
        .await;
    }

    Ok(Json(cancelled))
}

// ------------------------------------------------------------------- audit --

#[utoipa::path(
    get,
    path = "/api/v1/account/security-events",
    responses((status = 200, body = Vec<SecurityEvent>)),
    security(("session_cookie" = []))
)]
pub async fn list_security_events(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<SecurityEvent>>, ApiError> {
    Ok(Json(
        SecurityEventRepository::list(&state.db.pool, &user.account_id, 50)
            .await
            .map_err(ApiError::from)?,
    ))
}

/// Records that a `!link` needs the web UI to confirm a merge.
///
/// Held in memory rather than a table: it is a handoff between two halves of
/// one flow, valid only until the user answers, and a stale entry should
/// evaporate on restart rather than linger as a standing authorisation to
/// absorb an account.
pub async fn note_pending_merge(state: &ApiState, into: &str, from: &str) {
    state
        .pending_merges
        .write()
        .await
        .insert(into.to_string(), from.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{session::hash_token, LinkOutcome, STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    async fn state() -> ApiState {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
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
        let account = AccountRepository::create(&state.db.pool, name, STATUS_ACTIVE)
            .await
            .unwrap();
        let token = SessionRepository::create(&state.db.pool, &account, Some("Test/1.0"), None, None)
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

        let outcome = LinkRepository::claim(&state.db.pool, &res.code, "@evan:mbm.one", true)
            .await
            .unwrap();
        assert_eq!(outcome, LinkOutcome::Linked { account_id: user.account_id.clone() });

        let links = list_links(State(state), user).await.unwrap();
        assert_eq!(links.0, vec!["@evan:mbm.one".to_string()]);
    }

    #[tokio::test]
    async fn test_unlink_removes_only_your_own_link() {
        let state = state().await;
        let mine = signed_in(&state, "mine").await;
        let theirs = signed_in(&state, "theirs").await;

        sqlx::query("INSERT INTO matrix_links (mxid, account_id) VALUES ('@evan:mbm.one', ?)")
            .bind(&theirs.account_id)
            .execute(&state.db.pool)
            .await
            .unwrap();

        let err = unlink(
            State(state.clone()),
            VerifiedUser(mine),
            Json(UnlinkRequest { mxid: "@evan:mbm.one".to_string() }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);

        // Still linked to its real owner.
        assert_eq!(
            AccountRepository::find_by_mxid(&state.db.pool, "@evan:mbm.one").await.unwrap(),
            Some(theirs.account_id)
        );
    }

    /// Naming another account's id must not be enough to absorb it. The pending
    /// entry — written only by a completed `!link` — is the authorisation.
    #[tokio::test]
    async fn test_a_merge_cannot_be_confirmed_without_a_pending_link() {
        let state = state().await;
        let attacker = signed_in(&state, "attacker").await;
        let victim = AccountRepository::create(&state.db.pool, "victim", STATUS_UNCLAIMED)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', ?, 'victim', 'private', 'medium', 0)",
        )
        .bind(&victim)
        .execute(&state.db.pool)
        .await
        .unwrap();

        let err = confirm_merge(
            State(state.clone()),
            VerifiedUser(attacker.clone()),
            Json(ConfirmMergeRequest { from_account: victim.clone() }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);

        let owner: String = sqlx::query_scalar("SELECT owner_id FROM todos WHERE text = 'private'")
            .fetch_one(&state.db.pool)
            .await
            .unwrap();
        assert_eq!(owner, victim, "the victim's row must not have moved");
    }

    #[tokio::test]
    async fn test_a_pending_merge_can_be_previewed_then_confirmed() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;
        let old = AccountRepository::resolve_or_provision(&state.db.pool, "@evan:mbm.one")
            .await
            .unwrap();

        for text in ["one", "two"] {
            sqlx::query(
                "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
                 VALUES ('!r:x', ?, 'evan', ?, 'medium', 0)",
            )
            .bind(&old)
            .bind(text)
            .execute(&state.db.pool)
            .await
            .unwrap();
        }

        note_pending_merge(&state, &user.account_id, &old).await;

        let pending = pending_merge(State(state.clone()), user.clone()).await.unwrap();
        let pending = pending.0.expect("a merge should be waiting");
        assert_eq!(pending.preview.todos, 2);

        let moved = confirm_merge(
            State(state.clone()),
            VerifiedUser(user.clone()),
            Json(ConfirmMergeRequest { from_account: old }),
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

        sqlx::query(
            "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
             VALUES ('only', ?, '{}', 'Only key')",
        )
        .bind(&user.account_id)
        .execute(&state.db.pool)
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
        SessionRepository::create(&state.db.pool, &user.account_id, Some("Other/1.0"), None, None)
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
        let other = SessionRepository::create(&state.db.pool, &user.account_id, None, None, None)
            .await
            .unwrap();

        let revoked = revoke_other_sessions(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();
        assert_eq!(revoked.0, 1);

        assert!(SessionRepository::resolve(&state.db.pool, &other).await.unwrap().is_none());

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

        RecoveryRepository::request(&state.db.pool, &user.account_id).await.unwrap();
        assert!(RecoveryRepository::pending(&state.db.pool, &user.account_id)
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
        assert!(RecoveryRepository::pending(&state.db.pool, &user.account_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_sensitive_actions_are_written_to_the_audit_log() {
        let state = state().await;
        let user = signed_in(&state, "evan").await;

        regenerate_recovery_codes(State(state.clone()), VerifiedUser(user.clone()))
            .await
            .unwrap();
        revoke_other_sessions(State(state.clone()), VerifiedUser(user.clone()))
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

        sqlx::query(
            "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
             VALUES ('theirs', ?, '{}', 'Their key')",
        )
        .bind(&theirs.account_id)
        .execute(&state.db.pool)
        .await
        .unwrap();
        SecurityEventRepository::record(
            &state.db.pool,
            &theirs.account_id,
            SecurityEventKind::SignedIn,
            None,
        )
        .await;

        assert!(list_passkeys(State(state.clone()), mine.clone()).await.unwrap().0.is_empty());
        assert!(list_security_events(State(state.clone()), mine.clone())
            .await
            .unwrap()
            .0
            .is_empty());

        let sessions = list_sessions(State(state), mine.clone()).await.unwrap();
        assert_eq!(sessions.0.len(), 1, "only this account's own session");
    }
}
