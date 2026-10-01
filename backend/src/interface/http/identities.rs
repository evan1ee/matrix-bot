//! The identities linked to the caller's account, across every channel.

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::channels::{Channel, LinkedIdentity};
use crate::error::ApiError;
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::types::{ApiState, AuthUser};

/// Longest string a request may carry (AGENTS.md §5.1).
const MAX_FIELD: usize = 500;

#[derive(Debug, Serialize, ToSchema)]
pub struct IdentityDto {
    /// `matrix`, `discord`, `telegram` or `email`.
    pub channel: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub linked_at: DateTime<Utc>,
}

impl From<&LinkedIdentity> for IdentityDto {
    fn from(identity: &LinkedIdentity) -> Self {
        Self {
            channel: identity.channel.as_str().to_string(),
            user_id: identity.user_id.clone(),
            display_name: identity.display_name.clone(),
            linked_at: identity.linked_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IdentityListResponse {
    pub identities: Vec<IdentityDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnlinkIdentityRequest {
    pub channel: String,
    pub user_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnlinkIdentityResponse {
    /// `false` when the account did not hold that identity.
    pub unlinked: bool,
}

#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/identities",
    responses((status = 200, description = "Identities linked to this account", body = IdentityListResponse)),
    security(("session_cookie" = []))
)]
pub async fn list_identities(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<IdentityListResponse>, ApiError> {
    let linked = state.services.identity.list(&user.account_id).await?;
    Ok(Json(IdentityListResponse {
        identities: linked.iter().map(IdentityDto::from).collect(),
    }))
}

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/identities/unlink",
    request_body = UnlinkIdentityRequest,
    responses(
        (status = 200, description = "Unlinked, or not linked here", body = UnlinkIdentityResponse),
        (status = 400, description = "Unknown channel or oversized field", body = ApiErrorResponse),
        (status = 403, description = "Passkey confirmation required", body = ApiErrorResponse),
        (status = 409, description = "That is the only way to sign in", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn unlink_identity(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Json(payload): Json<UnlinkIdentityRequest>,
) -> Result<Json<UnlinkIdentityResponse>, ApiError> {
    if payload.channel.len() > MAX_FIELD || payload.user_id.len() > MAX_FIELD {
        return Err(ApiError::bad_request("That identity is too long"));
    }
    let channel: Channel = payload
        .channel
        .parse()
        .map_err(|_| ApiError::bad_request("Unknown channel"))?;
    let unlinked = state
        .services
        .identity
        .unlink(&user.account_id, channel, &payload.user_id)
        .await?;
    Ok(Json(UnlinkIdentityResponse { unlinked }))
}
