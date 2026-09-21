use sqlx::SqlitePool;
use uuid::Uuid;

use super::models::{AiConversation, AiMessage, AiUserConfig, UpdateAiConfigInput};

pub struct AiRepository;

impl AiRepository {
    pub async fn get_user_config(
        pool: &SqlitePool,
        owner_id: &str,
    ) -> Result<Option<AiUserConfig>, sqlx::Error> {
        let mut config = sqlx::query_as::<_, AiUserConfig>(
            r#"
            SELECT owner_id, api_base_url, api_key, model_name, system_prompt, temperature, max_tokens, updated_at
            FROM ai_user_configs
            WHERE owner_id = ?
            "#,
        )
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        if let Some(ref mut cfg) = config {
            cfg.has_api_key = cfg
                .api_key
                .as_ref()
                .map(|k| !k.trim().is_empty())
                .unwrap_or(false);
        }

        Ok(config)
    }

    pub async fn upsert_user_config(
        pool: &SqlitePool,
        owner_id: &str,
        input: &UpdateAiConfigInput,
    ) -> Result<AiUserConfig, sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO ai_user_configs (
                owner_id, api_base_url, api_key, model_name, system_prompt, temperature, max_tokens, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(owner_id) DO UPDATE SET
                api_base_url = COALESCE(excluded.api_base_url, ai_user_configs.api_base_url),
                api_key = CASE
                    WHEN excluded.api_key = '' THEN NULL
                    WHEN excluded.api_key IS NOT NULL THEN excluded.api_key
                    ELSE ai_user_configs.api_key
                END,
                model_name = COALESCE(excluded.model_name, ai_user_configs.model_name),
                system_prompt = COALESCE(excluded.system_prompt, ai_user_configs.system_prompt),
                temperature = COALESCE(excluded.temperature, ai_user_configs.temperature),
                max_tokens = COALESCE(excluded.max_tokens, ai_user_configs.max_tokens),
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(owner_id)
        .bind(&input.api_base_url)
        .bind(&input.api_key)
        .bind(&input.model_name)
        .bind(&input.system_prompt)
        .bind(input.temperature)
        .bind(input.max_tokens)
        .execute(pool)
        .await?;

        Self::get_user_config(pool, owner_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn get_or_create_conversation(
        pool: &SqlitePool,
        owner_id: &str,
        room_id: &str,
        max_age_seconds: i64,
    ) -> Result<AiConversation, sqlx::Error> {
        // Look for recent conversation
        let existing = sqlx::query_as::<_, AiConversation>(
            r#"
            SELECT id, owner_id, room_id, created_at, updated_at
            FROM ai_conversations
            WHERE owner_id = ? AND room_id = ?
              AND (strftime('%s', 'now') - strftime('%s', updated_at)) <= ?
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(owner_id)
        .bind(room_id)
        .bind(max_age_seconds)
        .fetch_optional(pool)
        .await?;

        if let Some(conv) = existing {
            return Ok(conv);
        }

        // Otherwise create new conversation
        let id = format!("conv_{}", Uuid::new_v4().simple());
        sqlx::query(
            r#"
            INSERT INTO ai_conversations (id, owner_id, room_id, created_at, updated_at)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(&id)
        .bind(owner_id)
        .bind(room_id)
        .execute(pool)
        .await?;

        Self::get_conversation(pool, &id, owner_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn get_conversation(
        pool: &SqlitePool,
        id: &str,
        owner_id: &str,
    ) -> Result<Option<AiConversation>, sqlx::Error> {
        sqlx::query_as::<_, AiConversation>(
            r#"
            SELECT id, owner_id, room_id, created_at, updated_at
            FROM ai_conversations
            WHERE id = ? AND owner_id = ?
            "#,
        )
        .bind(id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_conversations(
        pool: &SqlitePool,
        owner_id: &str,
        limit: i64,
    ) -> Result<Vec<AiConversation>, sqlx::Error> {
        sqlx::query_as::<_, AiConversation>(
            r#"
            SELECT id, owner_id, room_id, created_at, updated_at
            FROM ai_conversations
            WHERE owner_id = ?
            ORDER BY updated_at DESC
            LIMIT ?
            "#,
        )
        .bind(owner_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn delete_conversation(
        pool: &SqlitePool,
        id: &str,
        owner_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            DELETE FROM ai_conversations
            WHERE id = ? AND owner_id = ?
            "#,
        )
        .bind(id)
        .bind(owner_id)
        .execute(pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn touch_conversation(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE ai_conversations
            SET updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn add_message(
        pool: &SqlitePool,
        conversation_id: &str,
        role: &str,
        content: &str,
        tool_call_id: Option<&str>,
        tool_name: Option<&str>,
    ) -> Result<AiMessage, sqlx::Error> {
        let id = format!("msg_{}", Uuid::new_v4().simple());
        sqlx::query(
            r#"
            INSERT INTO ai_messages (id, conversation_id, role, content, tool_call_id, tool_name, created_at)
            VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(&id)
        .bind(conversation_id)
        .bind(role)
        .bind(content)
        .bind(tool_call_id)
        .bind(tool_name)
        .execute(pool)
        .await?;

        Self::touch_conversation(pool, conversation_id).await?;

        sqlx::query_as::<_, AiMessage>(
            r#"
            SELECT id, conversation_id, role, content, tool_call_id, tool_name, created_at
            FROM ai_messages
            WHERE id = ?
            "#,
        )
        .bind(&id)
        .fetch_one(pool)
        .await
    }

    pub async fn get_recent_messages(
        pool: &SqlitePool,
        conversation_id: &str,
        limit: i64,
    ) -> Result<Vec<AiMessage>, sqlx::Error> {
        let mut messages = sqlx::query_as::<_, AiMessage>(
            r#"
            SELECT id, conversation_id, role, content, tool_call_id, tool_name, created_at
            FROM ai_messages
            WHERE conversation_id = ? AND (role != 'assistant' OR TRIM(content) != '')
            ORDER BY created_at DESC
            LIMIT ?
            "#,
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        messages.reverse();
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup_db() -> (Database, String) {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();
        crate::db::schema::run_migrations(&db.pool).await.unwrap();
        let account_id = crate::accounts::AccountRepository::resolve_or_provision(
            db.sqlite_pool(),
            "@tester:mbm.one",
        )
        .await
        .unwrap();
        (db, account_id)
    }

    #[tokio::test]
    async fn test_ai_user_config_crud() {
        let (db, owner) = setup_db().await;

        let initial = AiRepository::get_user_config(db.sqlite_pool(), &owner)
            .await
            .unwrap();
        assert!(initial.is_none());

        let input = UpdateAiConfigInput {
            api_base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-test12345678".to_string()),
            model_name: Some("gpt-4o".to_string()),
            system_prompt: Some("You are a helpful bot.".to_string()),
            temperature: Some(0.5),
            max_tokens: Some(1024),
            ..Default::default()
        };

        let saved = AiRepository::upsert_user_config(db.sqlite_pool(), &owner, &input)
            .await
            .unwrap();
        assert_eq!(saved.model_name.as_deref(), Some("gpt-4o"));
        assert_eq!(saved.temperature, Some(0.5));

        let retrieved = AiRepository::get_user_config(db.sqlite_pool(), &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.model_name.as_deref(), Some("gpt-4o"));
        assert_eq!(retrieved.api_key.as_deref(), Some("sk-test12345678"));
        assert!(retrieved.has_api_key);
    }

    #[tokio::test]
    async fn test_ai_user_config_partial_update_preserves_fields() {
        let (db, owner) = setup_db().await;

        let initial_input = UpdateAiConfigInput {
            api_base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-initial-secret".to_string()),
            model_name: Some("gpt-4o".to_string()),
            system_prompt: Some("Original prompt".to_string()),
            temperature: Some(0.3),
            max_tokens: Some(512),
            ..Default::default()
        };

        AiRepository::upsert_user_config(db.sqlite_pool(), &owner, &initial_input)
            .await
            .unwrap();

        // Partial update: switch model only
        let model_update = UpdateAiConfigInput {
            model_name: Some("deepseek-chat".to_string()),
            ..Default::default()
        };

        let updated = AiRepository::upsert_user_config(db.sqlite_pool(), &owner, &model_update)
            .await
            .unwrap();

        assert_eq!(updated.model_name.as_deref(), Some("deepseek-chat"));
        assert_eq!(
            updated.api_base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(updated.api_key.as_deref(), Some("sk-initial-secret"));
        assert_eq!(updated.system_prompt.as_deref(), Some("Original prompt"));
        assert_eq!(updated.temperature, Some(0.3));
        assert_eq!(updated.max_tokens, Some(512));
        assert!(updated.has_api_key);
    }

    #[tokio::test]
    async fn test_ai_conversations_and_messages() {
        let (db, owner) = setup_db().await;
        let room = "!room:mbm.one";

        let conv = AiRepository::get_or_create_conversation(db.sqlite_pool(), &owner, room, 1800)
            .await
            .unwrap();
        assert_eq!(conv.owner_id, owner);

        let msg1 =
            AiRepository::add_message(db.sqlite_pool(), &conv.id, "user", "Hello!", None, None)
                .await
                .unwrap();
        assert_eq!(msg1.content, "Hello!");
        assert_eq!(msg1.role, "user");

        let msg2 = AiRepository::add_message(
            db.sqlite_pool(),
            &conv.id,
            "assistant",
            "Hi there!",
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(msg2.content, "Hi there!");

        let list = AiRepository::get_recent_messages(db.sqlite_pool(), &conv.id, 10)
            .await
            .unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].role, "user");
        assert_eq!(list[1].role, "assistant");

        // An empty assistant message (e.g. intermediate tool call) should be filtered out
        let _empty_msg =
            AiRepository::add_message(db.sqlite_pool(), &conv.id, "assistant", "   ", None, None)
                .await
                .unwrap();
        let filtered_list = AiRepository::get_recent_messages(db.sqlite_pool(), &conv.id, 10)
            .await
            .unwrap();
        assert_eq!(filtered_list.len(), 2);

        let deleted = AiRepository::delete_conversation(db.sqlite_pool(), &conv.id, &owner)
            .await
            .unwrap();
        assert!(deleted);

        let after_delete = AiRepository::get_conversation(db.sqlite_pool(), &conv.id, &owner)
            .await
            .unwrap();
        assert!(after_delete.is_none());
    }
}
