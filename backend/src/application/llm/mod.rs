use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::llm::{AiConversation, AiMessage, AiUserConfig, UpdateAiConfigInput};
use crate::events::EventBroadcaster;
use crate::infrastructure::postgres::repositories::llm as llm_repo;

#[derive(Clone)]
pub struct LlmService {
    pool: PgPool,
    broadcaster: EventBroadcaster,
}

impl LlmService {
    pub fn new(pool: PgPool, broadcaster: EventBroadcaster) -> Self {
        Self { pool, broadcaster }
    }

    pub fn broadcaster(&self) -> &EventBroadcaster {
        &self.broadcaster
    }

    pub async fn get_user_config(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<AiUserConfig>, AppError> {
        llm_repo::get_user_config(&self.pool, account_id.as_str())
            .await
            .map_err(AppError::Database)
    }

    pub async fn update_user_config(
        &self,
        account_id: &AccountId,
        input: &UpdateAiConfigInput,
    ) -> Result<AiUserConfig, AppError> {
        let config = llm_repo::upsert_user_config(&self.pool, account_id.as_str(), input)
            .await
            .map_err(AppError::Database)?;
        self.broadcaster
            .send(crate::events::SystemEvent::AiConfigUpdated {
                owner_id: account_id.to_string(),
            });
        Ok(config)
    }

    pub async fn get_conversation(
        &self,
        id: &str,
        account_id: &AccountId,
    ) -> Result<Option<AiConversation>, AppError> {
        llm_repo::get_conversation(&self.pool, id, account_id.as_str())
            .await
            .map_err(AppError::Database)
    }

    pub async fn list_conversations(
        &self,
        account_id: &AccountId,
        limit: i64,
    ) -> Result<Vec<AiConversation>, AppError> {
        llm_repo::list_conversations(&self.pool, account_id.as_str(), limit)
            .await
            .map_err(AppError::Database)
    }

    pub async fn delete_conversation(
        &self,
        id: &str,
        account_id: &AccountId,
    ) -> Result<bool, AppError> {
        let deleted = llm_repo::delete_conversation(&self.pool, id, account_id.as_str())
            .await
            .map_err(AppError::Database)?;
        if deleted {
            self.broadcaster
                .send(crate::events::SystemEvent::AiConversationDeleted {
                    id: id.to_string(),
                    owner_id: account_id.to_string(),
                });
        }
        Ok(deleted)
    }

    pub async fn get_recent_messages(
        &self,
        conversation_id: &str,
        limit: i64,
    ) -> Result<Vec<AiMessage>, AppError> {
        llm_repo::get_recent_messages(&self.pool, conversation_id, limit)
            .await
            .map_err(AppError::Database)
    }
}
