//! Chat-initiated account linking, web half (spec §7.3).
//!
//! The token arrives in a request body, never a URL, and is not echoed or
//! logged here: it is a bearer secret for ten minutes.

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use utoipa::ToSchema;
use webauthn_rs::prelude::RegisterPublicKeyCredential;

use crate::application::identity::{LinkOutcome, LinkPreview};
use crate::domain::accounts::{hash_token, AccountId};
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::recovery::extract_client_ip;
use crate::interface::http::auth::types::{ApiState, AuthUser, Ceremony};
use crate::interface::http::auth::webauthn::{issue_session, passkey_label, CeremonyStart};
use crate::interface::http::error::ApiError;
use crate::interface::http::rate_limit::check_link_signup_limit;

/// Longest string a request may carry (AGENTS.md §5.1).
const MAX_FIELD: usize = 500;
/// The WebAuthn display name budget (same as `/auth/passkey/register/start`).
const MAX_DISPLAY_NAME: usize = 64;

// Deliberately no `Debug`: a derived impl would print the token.
#[derive(Deserialize, ToSchema)]
pub struct LinkTokenRequest {
    pub token: String,
}

#[derive(Deserialize, ToSchema)]
pub struct LinkSignupFinishRequest {
    /// From `/link/signup/start`.
    pub ceremony_id: String,
    pub token: String,
    /// The authenticator's response, verbatim.
    #[schema(value_type = Object)]
    pub credential: RegisterPublicKeyCredential,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkPreviewState {
    Bind,
    AlreadyYours,
    NeedsMerge,
    OwnedElsewhere,
    SignupAllowed,
    SignInRequired,
    Expired,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkPreviewDto {
    pub state: LinkPreviewState,
    pub channel: Option<String>,
    pub user_id: Option<String>,
    /// Rows a merge would move; only for `needs_merge`.
    pub merge_item_count: Option<i64>,
}

impl From<&LinkPreview> for LinkPreviewDto {
    fn from(preview: &LinkPreview) -> Self {
        use LinkPreviewState as S;
        let (state, identity, merge_item_count) = match preview {
            LinkPreview::Bind { identity } => (S::Bind, Some(identity), None),
            LinkPreview::AlreadyYours => (S::AlreadyYours, None, None),
            LinkPreview::NeedsMerge { identity, preview } => {
                (S::NeedsMerge, Some(identity), Some(preview.total()))
            }
            LinkPreview::OwnedElsewhere => (S::OwnedElsewhere, None, None),
            LinkPreview::SignupAllowed { identity } => (S::SignupAllowed, Some(identity), None),
            LinkPreview::SignInRequired { identity } => (S::SignInRequired, Some(identity), None),
            LinkPreview::Expired => (S::Expired, None, None),
        };
        Self {
            state,
            channel: identity.map(|i| i.channel.as_str().to_string()),
            user_id: identity.map(|i| i.user_id.clone()),
            merge_item_count,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkClaimOutcome {
    Linked,
    AlreadyYours,
    Merged,
    OwnedElsewhere,
    Expired,
}

impl From<&LinkOutcome> for LinkClaimOutcome {
    fn from(outcome: &LinkOutcome) -> Self {
        match outcome {
            LinkOutcome::Linked { .. } => Self::Linked,
            LinkOutcome::AlreadyYours => Self::AlreadyYours,
            LinkOutcome::Merged { .. } => Self::Merged,
            LinkOutcome::OwnedElsewhere => Self::OwnedElsewhere,
            LinkOutcome::Expired => Self::Expired,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkClaimResponse {
    pub outcome: LinkClaimOutcome,
}

fn checked_token(token: &str) -> Result<&str, ApiError> {
    let token = token.trim();
    if token.is_empty() || token.len() > MAX_FIELD {
        return Err(ApiError::bad_request("That link is not valid"));
    }
    Ok(token)
}

fn rate_limited(
    state: &ApiState,
    connect_info: Option<&ConnectInfo<SocketAddr>>,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let ip = extract_client_ip(connect_info, headers, state.trust_forwarded_for);
    if check_link_signup_limit(&ip) {
        Ok(())
    } else {
        Err(ApiError::too_many_requests(
            "Too many sign-up attempts. Please wait a minute.",
        ))
    }
}

#[utoipa::path(
    post,
    tag = "link",
    path = "/api/v1/link/inspect",
    request_body = LinkTokenRequest,
    responses(
        (status = 200, description = "What claiming the token would do for this viewer", body = LinkPreviewDto),
        (status = 400, description = "Malformed token", body = ApiErrorResponse)
    )
)]
pub async fn inspect(
    State(state): State<ApiState>,
    viewer: Option<AuthUser>,
    Json(payload): Json<LinkTokenRequest>,
) -> Result<Json<LinkPreviewDto>, ApiError> {
    let token = checked_token(&payload.token)?;
    let preview = state
        .services
        .link
        .inspect(token, viewer.as_ref().map(|v| &v.account_id))
        .await?;
    Ok(Json(LinkPreviewDto::from(&preview)))
}

#[utoipa::path(
    post,
    tag = "link",
    path = "/api/v1/link/claim",
    request_body = LinkTokenRequest,
    responses(
        (status = 200, description = "Outcome of the claim", body = LinkClaimResponse),
        (status = 400, description = "Malformed token", body = ApiErrorResponse),
        (status = 403, description = "Passkey confirmation required", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn claim(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Json(payload): Json<LinkTokenRequest>,
) -> Result<Json<LinkClaimResponse>, ApiError> {
    let token = checked_token(&payload.token)?;
    let outcome = state.services.link.claim(token, &user.account_id).await?;
    Ok(Json(LinkClaimResponse {
        outcome: LinkClaimOutcome::from(&outcome),
    }))
}

/// The refusal for a token whose identity may not register here. The same
/// words whatever the reason, so the endpoint does not reveal the allowlist.
fn signup_refused() -> ApiError {
    ApiError::forbidden(
        "This account may not register here. Sign in to an existing account instead.",
    )
}

#[utoipa::path(
    post,
    tag = "link",
    path = "/api/v1/link/signup/start",
    request_body = LinkTokenRequest,
    responses(
        (status = 200, description = "Challenge to pass to navigator.credentials.create", body = CeremonyStart),
        (status = 403, description = "This identity may not register an account", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    )
)]
pub async fn signup_start(
    State(state): State<ApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<LinkTokenRequest>,
) -> Result<Json<CeremonyStart>, ApiError> {
    rate_limited(&state, connect_info.as_ref(), &headers)?;
    let token = checked_token(&payload.token)?;
    let LinkPreview::SignupAllowed { identity } = state.services.link.inspect(token, None).await?
    else {
        return Err(signup_refused());
    };

    let display_name: String = identity
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("{} {}", identity.channel.label(), identity.user_id))
        .chars()
        .take(MAX_DISPLAY_NAME)
        .collect();

    // As in `/auth/passkey/register/start`: the id is the WebAuthn user handle,
    // but no row exists until the ceremony finishes.
    let account_id = AccountId::generate();
    let (options, reg_state) = state
        .webauthn
        .start_passkey_registration(account_id.to_handle(), &display_name, &display_name, None)
        .map_err(|err| {
            tracing::error!("Could not start passkey registration: {:?}", err);
            ApiError::internal("Could not start passkey registration")
        })?;
    let ceremony_id = state
        .stash_ceremony(Ceremony::LinkSignup {
            account_id,
            display_name,
            token_hash: hash_token(token),
            state: reg_state,
        })
        .await?;

    Ok(Json(CeremonyStart {
        ceremony_id,
        options: serde_json::to_value(&options)
            .map_err(|err| ApiError::internal(err.to_string()))?,
    }))
}

#[utoipa::path(
    post,
    tag = "link",
    path = "/api/v1/link/signup/finish",
    request_body = LinkSignupFinishRequest,
    responses(
        (status = 200, description = "Account created, identity linked and signed in", body = SessionResponse),
        (status = 400, description = "Ceremony expired, credential rejected or link no longer valid", body = ApiErrorResponse),
        (status = 403, description = "This identity may not register an account", body = ApiErrorResponse),
        (status = 409, description = "Someone else linked this identity first", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    )
)]
pub async fn signup_finish(
    State(state): State<ApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<LinkSignupFinishRequest>,
) -> Result<Response, ApiError> {
    rate_limited(&state, connect_info.as_ref(), &headers)?;
    if payload.ceremony_id.len() > MAX_FIELD
        || payload
            .label
            .as_deref()
            .is_some_and(|l| l.len() > MAX_FIELD)
    {
        return Err(ApiError::bad_request("That request is too long"));
    }
    let token = checked_token(&payload.token)?;

    // Taken first: a challenge is single-use whatever happens next.
    let Some(Ceremony::LinkSignup {
        account_id,
        display_name,
        token_hash,
        state: reg_state,
    }) = state.take_ceremony(&payload.ceremony_id).await
    else {
        return Err(ApiError::bad_request(
            "That registration expired. Start again.",
        ));
    };
    if token_hash != hash_token(token) {
        return Err(ApiError::bad_request(
            "That registration expired. Start again.",
        ));
    }
    // Re-checked here (not only at start): the allowlist gates account
    // creation, so no credential work happens for an identity it refuses.
    if !matches!(
        state.services.link.inspect(token, None).await?,
        LinkPreview::SignupAllowed { .. }
    ) {
        return Err(signup_refused());
    }

    let passkey = state
        .webauthn
        .finish_passkey_registration(&payload.credential, &reg_state)
        .map_err(|err| {
            tracing::warn!("Passkey registration rejected: {:?}", err);
            ApiError::bad_request("That passkey could not be verified")
        })?;

    let outcome = state
        .services
        .link
        .sign_up_with_passkey(
            token,
            &account_id,
            &display_name,
            &passkey,
            passkey_label(payload.label.as_deref()),
        )
        .await?;
    match outcome {
        LinkOutcome::Linked { .. } | LinkOutcome::Merged { .. } => {
            issue_session(&state, &account_id, &display_name, &headers).await
        }
        LinkOutcome::Expired | LinkOutcome::AlreadyYours => Err(ApiError::bad_request(
            "That link expired. Ask the bot for a new one.",
        )),
        LinkOutcome::OwnedElsewhere => Err(ApiError::conflict(
            "That account was linked by someone else. Sign in instead.",
        )),
    }
}
