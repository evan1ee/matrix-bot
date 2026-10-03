use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::llm::{AiConversation, AiMessage, AiUserConfig, UpdateAiConfigInput};
use crate::infrastructure::postgres::repositories::llm as llm_repo;
use crate::infrastructure::postgres::repositories::outbox;

#[derive(Clone)]
pub struct LlmService {
    pool: PgPool,
}

impl LlmService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_user_config(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<AiUserConfig>, AppError> {
        llm_repo::get_user_config(&self.pool, account_id)
            .await
            .map_err(AppError::Database)
    }

    pub async fn update_user_config(
        &self,
        account_id: &AccountId,
        input: &UpdateAiConfigInput,
    ) -> Result<AiUserConfig, AppError> {
        let mut tx = self.pool.begin().await?;
        let config = llm_repo::upsert_user_config(&mut tx, account_id, input)
            .await
            .map_err(AppError::Database)?;
        outbox::publish(
            &mut tx,
            &SystemEvent::AiConfigUpdated {
                owner_id: *account_id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(config)
    }

    pub async fn get_conversation(
        &self,
        id: &str,
        account_id: &AccountId,
    ) -> Result<Option<AiConversation>, AppError> {
        llm_repo::get_conversation(&self.pool, id, account_id)
            .await
            .map_err(AppError::Database)
    }

    pub async fn list_conversations(
        &self,
        account_id: &AccountId,
        limit: i64,
    ) -> Result<Vec<AiConversation>, AppError> {
        llm_repo::list_conversations(&self.pool, account_id, limit)
            .await
            .map_err(AppError::Database)
    }

    pub async fn delete_conversation(
        &self,
        id: &str,
        account_id: &AccountId,
    ) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = llm_repo::delete_conversation(&mut *tx, id, account_id)
            .await
            .map_err(AppError::Database)?;
        if deleted {
            outbox::publish(
                &mut tx,
                &SystemEvent::AiConversationDeleted {
                    id: id.to_string(),
                    owner_id: *account_id,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    /// The owner's conversation for `room_id` updated within `max_age_seconds`,
    /// or a fresh one.
    pub async fn get_or_create_conversation(
        &self,
        account_id: &AccountId,
        room_id: &str,
        max_age_seconds: i64,
    ) -> Result<AiConversation, AppError> {
        llm_repo::get_or_create_conversation(&self.pool, account_id, room_id, max_age_seconds)
            .await
            .map_err(AppError::Database)
    }

    /// Stores the assistant's final reply and publishes
    /// `AiConversationUpdated` in one transaction: a failed insert publishes
    /// nothing.
    pub async fn append_assistant_message(
        &self,
        owner: &AccountId,
        conversation_id: &str,
        content: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        llm_repo::insert_message(&mut tx, conversation_id, "assistant", content, None, None)
            .await
            .map_err(AppError::Database)?;
        outbox::publish(
            &mut tx,
            &SystemEvent::AiConversationUpdated {
                id: conversation_id.to_string(),
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::postgres::repositories::outbox;

    async fn setup(name: &str) -> (LlmService, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(name);
        crate::db::pool::seed_account(&pool, &owner).await;
        (LlmService::new(pool), owner)
    }

    /// Given a conversation, When it is deleted twice, Then the second call
    /// returns false and exactly one AiConversationDeleted exists.
    #[tokio::test]
    async fn delete_conversation_publishes_only_when_a_row_was_deleted() {
        let (service, owner) = setup("llm_delete").await;
        let conv = llm_repo::get_or_create_conversation(&service.pool, &owner, "!r:x", 1800)
            .await
            .unwrap();

        assert!(service.delete_conversation(&conv.id, &owner).await.unwrap());
        assert!(!service.delete_conversation(&conv.id, &owner).await.unwrap());

        assert_eq!(
            outbox::event_types_for(&service.pool, &owner)
                .await
                .unwrap(),
            vec!["AiConversationDeleted"]
        );
    }

    /// When the assistant message is appended, Then it is stored and
    /// AiConversationUpdated is published.
    #[tokio::test]
    async fn append_assistant_message_stores_and_publishes() {
        let (service, owner) = setup("llm_append").await;
        let conv = llm_repo::get_or_create_conversation(&service.pool, &owner, "!r:x", 1800)
            .await
            .unwrap();

        service
            .append_assistant_message(&owner, &conv.id, "hello there")
            .await
            .unwrap();

        let messages = service.get_recent_messages(&conv.id, 10).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].content, "hello there");
        assert_eq!(
            outbox::event_types_for(&service.pool, &owner)
                .await
                .unwrap(),
            vec!["AiConversationUpdated"]
        );
    }

    /// Given a conversation that does not exist, When a message is appended,
    /// Then it fails and no event exists.
    #[tokio::test]
    async fn append_assistant_message_is_atomic() {
        let (service, owner) = setup("llm_append_fail").await;

        let res = service
            .append_assistant_message(&owner, "conv_missing", "x")
            .await;

        assert!(res.is_err());
        assert!(outbox::event_types_for(&service.pool, &owner)
            .await
            .unwrap()
            .is_empty());
    }

    /// When the config is updated, Then the row and AiConfigUpdated exist.
    #[tokio::test]
    async fn update_user_config_publishes_in_the_same_transaction() {
        let (service, owner) = setup("llm_config").await;
        let input = UpdateAiConfigInput {
            model_name: Some("m1".to_string()),
            ..Default::default()
        };

        let cfg = service.update_user_config(&owner, &input).await.unwrap();

        assert_eq!(cfg.model_name.as_deref(), Some("m1"));
        assert_eq!(
            outbox::event_types_for(&service.pool, &owner)
                .await
                .unwrap(),
            vec!["AiConfigUpdated"]
        );
    }
}
