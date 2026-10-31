use std::sync::Arc;

use sqlx::PgPool;

pub mod authentic_replies;
pub use authentic_replies::AiReplyTracker;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::llm::{
    title_from_prompt, AiConversation, AiMessage, AiUserConfig, ConversationTitle,
    UpdateAiConfigInput,
};
use crate::infrastructure::postgres::repositories::llm as llm_repo;
use crate::infrastructure::postgres::repositories::outbox;

#[derive(Clone)]
pub struct LlmService {
    pool: PgPool,
    pub authentic_replies: Arc<AiReplyTracker>,
}

impl LlmService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            authentic_replies: Arc::new(AiReplyTracker::default()),
        }
    }

    pub fn with_authentic_replies(mut self, tracker: Arc<AiReplyTracker>) -> Self {
        self.authentic_replies = tracker;
        self
    }

    /// Records an authentic assistant reply event ID to prevent quote taint.
    pub fn record_authentic_reply(&self, room_key: &str, event_id: &str, owner_id: &AccountId) {
        self.authentic_replies.record(room_key, event_id, *owner_id);
    }

    /// Checks if a quoted event ID corresponds to an authentic assistant reply.
    pub fn is_authentic_reply(&self, room_key: &str, event_id: &str, owner_id: &AccountId) -> bool {
        self.authentic_replies
            .is_authentic(room_key, event_id, owner_id)
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

    /// Mints an empty conversation. A turn that names no conversation resumes
    /// the room's latest one for an hour, so "new chat" has to create one.
    pub async fn create_conversation(
        &self,
        owner: &AccountId,
        room_id: &str,
    ) -> Result<AiConversation, AppError> {
        let mut tx = self.pool.begin().await?;
        let conv = llm_repo::create_conversation(&mut *tx, owner, room_id)
            .await
            .map_err(AppError::Database)?;
        self.publish_updated(&mut tx, owner, &conv.id).await?;
        tx.commit().await?;
        Ok(conv)
    }

    /// Sets the user's title for a conversation, replacing any earlier one.
    pub async fn rename_conversation(
        &self,
        owner: &AccountId,
        id: &str,
        title: &str,
    ) -> Result<AiConversation, AppError> {
        let title =
            ConversationTitle::parse(title).map_err(|e| AppError::Validation(e.to_string()))?;
        let mut tx = self.pool.begin().await?;
        if !llm_repo::set_conversation_title(&mut *tx, id, owner, title.as_str(), false)
            .await
            .map_err(AppError::Database)?
        {
            return Err(AppError::NotFound("Conversation not found".to_string()));
        }
        self.publish_updated(&mut tx, owner, id).await?;
        let conv = llm_repo::get_conversation(&mut *tx, id, owner)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| AppError::NotFound("Conversation not found".to_string()))?;
        tx.commit().await?;
        Ok(conv)
    }

    /// Names an untitled conversation from its first prompt. Does nothing
    /// once it has a title, so a rename is never overwritten.
    pub async fn auto_title(
        &self,
        owner: &AccountId,
        conversation_id: &str,
        prompt: &str,
    ) -> Result<(), AppError> {
        let Some(title) = title_from_prompt(prompt) else {
            return Ok(());
        };
        let mut tx = self.pool.begin().await?;
        if llm_repo::set_conversation_title(&mut *tx, conversation_id, owner, title.as_str(), true)
            .await
            .map_err(AppError::Database)?
        {
            self.publish_updated(&mut tx, owner, conversation_id)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Removes a message and everything after it, so an edited prompt can be
    /// sent from that point. The conversation itself stays.
    pub async fn truncate_from(
        &self,
        owner: &AccountId,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        llm_repo::truncate_messages_from(&mut *tx, conversation_id, owner, message_id)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;
        self.publish_updated(&mut tx, owner, conversation_id)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn publish_updated(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        owner: &AccountId,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        outbox::publish(
            tx,
            &SystemEvent::AiConversationUpdated {
                id: conversation_id.to_string(),
                owner_id: *owner,
            },
        )
        .await?;
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

    #[tokio::test]
    async fn test_llm_service_authentic_replies_delegation() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/dummy").unwrap();
        let service = LlmService::new(pool);
        let owner = AccountId::generate();

        assert!(!service.is_authentic_reply("!r:x", "$ev1", &owner));
        service.record_authentic_reply("!r:x", "$ev1", &owner);
        assert!(service.is_authentic_reply("!r:x", "$ev1", &owner));
        assert!(!service.is_authentic_reply("!r:x", "$ev2", &owner));
    }

    /// Given two accounts, When one creates, renames, auto-titles and
    /// truncates, Then each publishes AiConversationUpdated, bad titles are
    /// rejected, a rename survives later auto-titles, and strangers get NotFound.
    #[tokio::test]
    async fn thread_operations_publish_and_respect_ownership() {
        let (service, owner) = setup("llm_threads").await;
        let stranger = AccountId::from_seed("llm_threads_stranger");
        crate::db::pool::seed_account(&service.pool, &stranger).await;

        let conv = service.create_conversation(&owner, "web").await.unwrap();
        assert_eq!(conv.title, None);

        service
            .auto_title(&owner, &conv.id, "Plan my week")
            .await
            .unwrap();
        let titled = llm_repo::get_conversation(&service.pool, &conv.id, &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(titled.title.as_deref(), Some("Plan my week"));

        for bad in [
            "",
            "   ",
            "two
lines",
            &"x".repeat(101),
        ] {
            assert!(matches!(
                service.rename_conversation(&owner, &conv.id, bad).await,
                Err(AppError::Validation(_))
            ));
        }
        let renamed = service
            .rename_conversation(&owner, &conv.id, "  My plan ")
            .await
            .unwrap();
        assert_eq!(renamed.title.as_deref(), Some("My plan"));
        service
            .auto_title(&owner, &conv.id, "Something else")
            .await
            .unwrap();
        let after = llm_repo::get_conversation(&service.pool, &conv.id, &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.title.as_deref(), Some("My plan"));

        assert!(matches!(
            service
                .rename_conversation(&stranger, &conv.id, "Mine")
                .await,
            Err(AppError::NotFound(_))
        ));
        let first = llm_repo::add_message(&service.pool, &conv.id, "user", "hi", None, None)
            .await
            .unwrap();
        assert!(matches!(
            service.truncate_from(&stranger, &conv.id, &first.id).await,
            Err(AppError::NotFound(_))
        ));
        service
            .truncate_from(&owner, &conv.id, &first.id)
            .await
            .unwrap();
        assert!(service
            .get_recent_messages(&conv.id, 10)
            .await
            .unwrap()
            .is_empty());

        // create, auto_title, rename, truncate: one event each.
        assert_eq!(
            outbox::event_types_for(&service.pool, &owner)
                .await
                .unwrap(),
            vec!["AiConversationUpdated"; 4]
        );
    }
}
