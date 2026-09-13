use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use tracing::error;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::ai::client::ResolvedAiConfig;
use crate::modules::ai::handler::{get_ai_client, run_ai_turn};
use crate::modules::ai::models::{
    AiChatRequest, AiChatResponse, AiConversation, AiMessage, AiUserConfig, UpdateAiConfigInput,
    VerifyAiKeyInput, VerifyAiKeyResponse,
};
use crate::modules::ai::repository::AiRepository;
use crate::modules::ai::retrospective::{generate_weekly_retrospective, WeeklyRetrospective};

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
    let room_id = crate::api::rooms::resolve_target_room(
        &state.db.pool,
        owner_id,
        payload.room_id.as_deref(),
    )
    .await;

    let turn_result = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        run_ai_turn(
            &state.db.pool,
            Some(&state.broadcaster),
            owner_id,
            &room_id,
            payload.conversation_id.as_deref(),
            prompt,
        ),
    )
    .await
    .map_err(|_| {
        error!("AI chat execution timed out after 90s for {}", owner_id);
        ApiError::gateway_timeout("AI response timed out after 90 seconds. Please try again.")
    })?
    .map_err(|e| {
        error!("AI chat execution error for {}: {}", owner_id, e);
        ApiError::bad_request(e)
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
    let convs = AiRepository::list_conversations(&state.db.pool, &user.account_id, 20)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(convs))
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
    let _conv = AiRepository::get_conversation(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("Conversation not found"))?;

    let messages = AiRepository::get_recent_messages(&state.db.pool, &id, 100)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(messages))
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
    let deleted = AiRepository::delete_conversation(&state.db.pool, &id, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    if !deleted {
        return Err(ApiError::not_found("Conversation not found"));
    }

    state.broadcaster.send(SystemEvent::AiConversationDeleted {
        id,
        owner_id: user.account_id.clone(),
    });

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
    let resolved = ResolvedAiConfig::from_user_config(None);
    let settings = crate::modules::settings::repository::SettingsRepository::get_or_default(
        &state.db.pool,
        &user.account_id,
    )
    .await;
    let config = AiRepository::get_user_config(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?
        .map(|c| {
            let mut redacted = c.redacted();
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
            owner_id: user.account_id.clone(),
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
    input.validate().map_err(ApiError::bad_request)?;

    if let Some(turns) = input.max_history_turns {
        let mut settings =
            crate::modules::settings::repository::SettingsRepository::get_or_default(
                &state.db.pool,
                &user.account_id,
            )
            .await;
        settings.max_history_turns = turns.clamp(1, 100);
        let _ = crate::modules::settings::repository::SettingsRepository::upsert(
            &state.db.pool,
            &user.account_id,
            &settings,
        )
        .await;
        state.broadcaster.send(SystemEvent::SettingsUpdated {
            owner_id: user.account_id.clone(),
            settings,
        });
    }

    let updated = AiRepository::upsert_user_config(&state.db.pool, &user.account_id, &input)
        .await
        .map_err(ApiError::from)?;

    state.broadcaster.send(SystemEvent::AiConfigUpdated {
        owner_id: user.account_id.clone(),
    });

    let settings = crate::modules::settings::repository::SettingsRepository::get_or_default(
        &state.db.pool,
        &user.account_id,
    )
    .await;
    let mut redacted = updated.redacted();
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
    input.validate().map_err(ApiError::bad_request)?;

    let user_config = AiRepository::get_user_config(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    let resolved = ResolvedAiConfig::from_user_config(user_config.as_ref());

    let target_base_url = input
        .api_base_url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or(resolved.api_base_url);

    let target_key = input
        .api_key
        .filter(|k| !k.trim().is_empty())
        .or_else(|| user_config.as_ref().and_then(|c| c.api_key.clone()))
        .or(resolved.api_key);

    let target_model = input
        .model_name
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(resolved.model_name);

    match get_ai_client()
        .verify_api_key(&target_base_url, target_key.as_deref(), Some(&target_model))
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
    let retro = generate_weekly_retrospective(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(retro))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    async fn setup_state() -> (ApiState, String) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let owner =
            crate::accounts::AccountRepository::resolve_or_provision(&pool, "@apitest:mbm.one")
                .await
                .unwrap();
        let state = ApiState::new(
            Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        (state, owner)
    }

    #[tokio::test]
    async fn test_ai_config_api_redacts_api_key() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(&owner);

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
        let user = AuthUser::for_account(&owner);

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
        let user = AuthUser::for_account(&owner);

        let retro_res = get_retrospective(State(state), user).await.unwrap();
        assert_eq!(retro_res.currency, "USD");
        assert_eq!(retro_res.total_spent_minor, 0);
        assert!(retro_res
            .formatted_markdown
            .contains("Weekly Executive Retrospective"));
    }
}
