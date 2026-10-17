use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use tracing::error;

use crate::infrastructure::postgres::repositories::llm as llm_repo;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
use crate::llm::error::AiError;
use crate::llm::models::{
    AiChatRequest, AiChatResponse, AiConversation, AiMessage, AiUserConfig, UpdateAiConfigInput,
    VerifyAiKeyInput, VerifyAiKeyResponse,
};
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::llm::retrospective::{generate_weekly_retrospective, WeeklyRetrospective};
use crate::llm::{run_turn, LlmTurnContext};

/// It used to be a bare `String`, so this always answered 400 — a provider
/// outage and a bad prompt looked identical to the caller. Each variant now
/// gets the status it actually deserves.
impl From<AiError> for ApiError {
    fn from(err: AiError) -> Self {
        match err {
            AiError::Validation(msg) => ApiError::bad_request(msg),
            AiError::Provider(msg) => ApiError::service_unavailable(msg),
            AiError::Timeout => {
                ApiError::gateway_timeout("The AI provider did not respond in time")
            }
            AiError::Db(msg) => ApiError::internal(msg),
        }
    }
}

#[utoipa::path(
    post,
    tag = "ai",
    path = "/api/v1/ai/chat",
    request_body = AiChatRequest,
    responses((status = 200, body = AiChatResponse)),
    security(("session_cookie" = []))
)]
pub async fn chat(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<AiChatRequest>,
) -> Result<Json<AiChatResponse>, ApiError> {
    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return Err(ApiError::bad_request("Prompt cannot be empty"));
    }
    if prompt.len() > 4000 {
        return Err(ApiError::bad_request(
            "Prompt exceeds maximum length of 4000 characters",
        ));
    }

    let owner_id = &user.account_id;
    let room_id = state
        .services
        .rooms
        .resolve_target_room(owner_id, payload.room_id.as_deref())
        .await;

    let turn_ctx = LlmTurnContext::new(
        &state.services,
        &state.db.pool,
        &state.ai,
        owner_id,
        &room_id,
        None,
    );

    let turn_result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        run_turn(&turn_ctx, prompt, payload.conversation_id.as_deref()),
    )
    .await
    .map_err(|_| {
        error!("AI chat execution timed out after 180s for {}", owner_id);
        ApiError::gateway_timeout("AI response timed out after 180 seconds. Please try again.")
    })?
    .map_err(|e| {
        error!("AI chat execution error for {}: {}", owner_id, e);
        ApiError::from(e)
    })?;

    Ok(Json(AiChatResponse {
        conversation_id: turn_result.conversation_id,
        message: turn_result.message,
        model_used: turn_result.model_used,
        tool_calls_executed: turn_result.tool_calls_executed,
    }))
}

#[utoipa::path(
    get,
    tag = "ai",
    path = "/api/v1/ai/conversations",
    responses((status = 200, body = Vec<AiConversation>)),
    security(("session_cookie" = []))
)]
pub async fn list_conversations(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<AiConversation>>, ApiError> {
    let convs = llm_repo::list_conversations(&state.db.pool, &user.account_id, 20)
        .await
        .map_err(ApiError::from)?;
    let dtos: Vec<AiConversation> = convs.into_iter().map(Into::into).collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    get,
    tag = "ai",
    path = "/api/v1/ai/conversations/{id}/messages",
    params(("id" = String, Path, description = "Conversation ID")),
    responses((status = 200, body = Vec<AiMessage>)),
    security(("session_cookie" = []))
)]
pub async fn get_conversation_messages(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<AiMessage>>, ApiError> {
    let _conv = llm_repo::get_conversation(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("Conversation not found"))?;

    let messages = llm_repo::get_recent_messages(&state.db.pool, &id, 100)
        .await
        .map_err(ApiError::from)?;
    let dtos: Vec<AiMessage> = messages.into_iter().map(Into::into).collect();
    Ok(Json(dtos))
}

#[utoipa::path(
    delete,
    tag = "ai",
    path = "/api/v1/ai/conversations/{id}",
    params(("id" = String, Path, description = "Conversation ID")),
    responses((status = 204)),
    security(("session_cookie" = []))
)]
pub async fn delete_conversation(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .services
        .llm
        .delete_conversation(&id, &user.account_id)
        .await?;

    if !deleted {
        return Err(ApiError::not_found("Conversation not found"));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    tag = "ai",
    path = "/api/v1/account/ai-config",
    responses((status = 200, body = AiUserConfig)),
    security(("session_cookie" = []))
)]
pub async fn get_ai_config(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<AiUserConfig>, ApiError> {
    let resolved = ResolvedAiConfig::resolve_effective(&state.services, &state.ai, None).await;
    let settings = state
        .services
        .settings
        .get_or_default(&user.account_id)
        .await;
    let config = llm_repo::get_user_config(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?
        .map(|c| {
            let mut redacted: AiUserConfig = c.into();
            redacted = redacted.redacted();
            if redacted
                .api_base_url
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                redacted.api_base_url = Some(resolved.api_base_url.clone());
            }
            if redacted
                .model_name
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                redacted.model_name = Some(resolved.model_name.clone());
            }
            if !redacted.has_api_key && resolved.api_key.is_some() {
                redacted.has_api_key = true;
            }
            redacted.max_history_turns = Some(settings.max_history_turns);
            redacted
        })
        .unwrap_or_else(|| AiUserConfig {
            owner_id: user.account_id.to_string(),
            api_base_url: Some(resolved.api_base_url),
            api_key: None,
            model_name: Some(resolved.model_name),
            system_prompt: None,
            temperature: Some(resolved.temperature),
            max_tokens: Some(resolved.max_tokens),
            max_history_turns: Some(settings.max_history_turns),
            updated_at: String::new(),
            has_api_key: resolved.api_key.is_some(),
        });

    Ok(Json(config))
}

#[utoipa::path(
    put,
    tag = "ai",
    path = "/api/v1/account/ai-config",
    request_body = UpdateAiConfigInput,
    responses((status = 200, body = AiUserConfig)),
    security(("session_cookie" = []))
)]
pub async fn update_ai_config(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(input): Json<UpdateAiConfigInput>,
) -> Result<Json<AiUserConfig>, ApiError> {
    input
        .validate(state.ai.allow_private_base_url)
        .map_err(ApiError::bad_request)?;

    if let Some(turns) = input.max_history_turns {
        let clamped = turns.clamp(1, 100);
        if let Err(err) = state
            .services
            .settings
            .modify(&user.account_id, |current| {
                current.max_history_turns = clamped;
                Ok(())
            })
            .await
        {
            tracing::warn!(
                owner_id = %user.account_id,
                error = %err,
                "Failed to update max_history_turns alongside the AI config"
            );
        }
    }

    let domain_input: crate::domain::llm::UpdateAiConfigInput = input.into();
    let updated = state
        .services
        .llm
        .update_user_config(&user.account_id, &domain_input)
        .await?;

    let settings = state
        .services
        .settings
        .get_or_default(&user.account_id)
        .await;
    let dto_updated: AiUserConfig = updated.into();
    let mut redacted = dto_updated.redacted();
    redacted.max_history_turns = Some(settings.max_history_turns);

    Ok(Json(redacted))
}

#[utoipa::path(
    post,
    tag = "ai",
    path = "/api/v1/ai/verify",
    request_body = VerifyAiKeyInput,
    responses((status = 200, body = VerifyAiKeyResponse)),
    security(("session_cookie" = []))
)]
pub async fn verify_ai_config(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(input): Json<VerifyAiKeyInput>,
) -> Result<Json<VerifyAiKeyResponse>, ApiError> {
    input
        .validate(state.ai.allow_private_base_url)
        .map_err(ApiError::bad_request)?;

    let user_config = llm_repo::get_user_config(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    // Resolved together, so the instance key never goes to a host the caller
    // typed into the form.
    let target = ResolvedAiConfig::for_verification(
        &state.ai,
        user_config.as_ref(),
        input.api_base_url.as_deref(),
        input.api_key.as_deref(),
        input.model_name.as_deref(),
    );

    match crate::infrastructure::inference::llm::shared_client(state.ai.proxy.as_deref())
        .verify_api_key(
            &target.api_base_url,
            target.api_key.as_deref(),
            Some(&target.model_name),
        )
        .await
    {
        Ok(()) => Ok(Json(VerifyAiKeyResponse {
            valid: true,
            message: "Connection successful and API key verified!".to_string(),
        })),
        Err(err) => Ok(Json(VerifyAiKeyResponse {
            valid: false,
            message: err,
        })),
    }
}

#[utoipa::path(
    get,
    tag = "ai",
    path = "/api/v1/ai/retrospective",
    responses((status = 200, body = WeeklyRetrospective)),
    security(("session_cookie" = []))
)]
pub async fn get_retrospective(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<WeeklyRetrospective>, ApiError> {
    let retro =
        generate_weekly_retrospective(&state.db.pool, &state.services.settings, &user.account_id)
            .await
            .map_err(ApiError::from)?;
    Ok(Json(retro))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::domain::accounts::AccountId;

    async fn setup_state() -> (ApiState, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = crate::application::AppServices::new(&pool)
            .identity
            .provision_matrix_for_test("@apitest:mbm.one")
            .await;
        let state = ApiState::new(
            Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        (state, owner)
    }

    #[tokio::test]
    async fn test_ai_config_api_redacts_api_key() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        let update_input = UpdateAiConfigInput {
            api_base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-very-secret-token".to_string()),
            model_name: Some("gpt-4o".to_string()),
            system_prompt: None,
            temperature: Some(0.7),
            max_tokens: Some(1024),
            max_history_turns: Some(15),
        };

        let put_res = update_ai_config(State(state.clone()), user.clone(), Json(update_input))
            .await
            .unwrap();
        assert!(put_res.has_api_key);
        assert_eq!(put_res.api_key, None);
        assert_eq!(put_res.max_history_turns, Some(15));

        let get_res = get_ai_config(State(state.clone()), user.clone())
            .await
            .unwrap();
        assert!(get_res.has_api_key);
        assert_eq!(get_res.api_key, None);
        assert_eq!(get_res.model_name.as_deref(), Some("gpt-4o"));
        assert_eq!(get_res.max_history_turns, Some(15));
    }

    #[tokio::test]
    async fn test_ai_config_api_validates_input_lengths() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        let too_long_model = UpdateAiConfigInput {
            api_base_url: None,
            api_key: None,
            model_name: Some("a".repeat(501)),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            max_history_turns: None,
        };

        let err = update_ai_config(State(state.clone()), user.clone(), Json(too_long_model))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let invalid_history = UpdateAiConfigInput {
            api_base_url: None,
            api_key: None,
            model_name: None,
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            max_history_turns: Some(150),
        };
        let err2 = update_ai_config(State(state), user, Json(invalid_history))
            .await
            .unwrap_err();
        assert_eq!(err2.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_retrospective() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        let retro_res = get_retrospective(State(state), user).await.unwrap();
        assert_eq!(retro_res.currency, "USD");
        assert_eq!(retro_res.total_spent_minor, 0);
        assert!(retro_res
            .formatted_markdown
            .contains("Weekly Executive Retrospective"));
    }

    /// The bug this whole type exists to fix: a provider outage used to reach
    /// the browser as 400 Bad Request, indistinguishable from a prompt the
    /// caller should not have sent. An unreachable base URL (the same
    /// `http://127.0.0.1:1` pattern `ai::handler`'s own tests use) gives a
    /// real `AiError::Provider` without depending on the network.
    #[tokio::test]
    async fn test_a_provider_failure_is_not_reported_as_a_bad_request() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        let input = UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1:1".to_string()),
            api_key: Some("invalid-key".to_string()),
            model_name: Some("test-model".to_string()),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            max_history_turns: None,
        };
        let domain_input: crate::domain::llm::UpdateAiConfigInput = input.into();
        state
            .services
            .llm
            .update_user_config(&owner, &domain_input)
            .await
            .unwrap();

        let err = chat(
            State(state),
            user,
            Json(AiChatRequest {
                room_id: None,
                conversation_id: None,
                prompt: "hello".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert_ne!(
            err.status,
            StatusCode::BAD_REQUEST,
            "a provider outage must not look like a bad request"
        );
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Covers the four-way mapping directly, since the test above only
    /// exercises the `Provider` arm end to end.
    #[test]
    fn test_ai_error_maps_to_the_status_it_deserves() {
        assert_eq!(
            ApiError::from(AiError::Validation("bad prompt".to_string())).status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::from(AiError::Provider("provider down".to_string())).status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            ApiError::from(AiError::Timeout).status,
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            ApiError::from(AiError::Db("db exploded".to_string())).status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
