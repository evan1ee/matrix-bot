use uuid::Uuid;

use crate::domain::accounts::AccountId;
use crate::domain::llm::{AiConversation, AiMessage, AiUserConfig, UpdateAiConfigInput};

pub async fn get_user_config(
    conn: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
) -> Result<Option<AiUserConfig>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT owner_id, api_base_url, api_key, model_name, system_prompt, temperature, max_tokens, updated_at
        FROM ai_user_configs
        WHERE owner_id = $1
        "#,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|r| {
        let has_api_key = r
            .api_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        AiUserConfig {
            owner_id: AccountId::from_uuid(r.owner_id).to_string(),
            api_base_url: r.api_base_url,
            api_key: r.api_key,
            model_name: r.model_name,
            system_prompt: r.system_prompt,
            temperature: r.temperature,
            max_tokens: r.max_tokens,
            max_history_turns: None,
            updated_at: r.updated_at,
            has_api_key,
        }
    }))
}

pub async fn upsert_user_config(
    pool: &sqlx::PgPool,
    owner_id: &AccountId,
    input: &UpdateAiConfigInput,
) -> Result<AiUserConfig, sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO ai_user_configs (
            owner_id, api_base_url, api_key, model_name, system_prompt, temperature, max_tokens, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, CAST(CURRENT_TIMESTAMP AS TEXT))
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
            updated_at = CAST(CURRENT_TIMESTAMP AS TEXT)
        "#,
        owner_id.as_uuid(),
        input.api_base_url,
        input.api_key,
        input.model_name,
        input.system_prompt,
        input.temperature,
        input.max_tokens
    )
    .execute(pool)
    .await?;

    get_user_config(pool, owner_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_or_create_conversation(
    pool: &sqlx::PgPool,
    owner_id: &AccountId,
    room_id: &str,
    max_age_seconds: i64,
) -> Result<AiConversation, sqlx::Error> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(max_age_seconds))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    let existing = sqlx::query!(
        r#"
        SELECT id, owner_id, room_id, created_at, updated_at
        FROM ai_conversations
        WHERE owner_id = $1 AND room_id = $2 AND updated_at >= $3
        ORDER BY updated_at DESC
        LIMIT 1
        "#,
        owner_id.as_uuid(),
        room_id,
        cutoff
    )
    .fetch_optional(pool)
    .await?;

    if let Some(r) = existing {
        return Ok(AiConversation {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id).to_string(),
            room_id: r.room_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        });
    }

    let id = format!("conv_{}", Uuid::new_v4().simple());
    sqlx::query!(
        r#"
        INSERT INTO ai_conversations (id, owner_id, room_id, created_at, updated_at)
        VALUES ($1, $2, $3, CAST(CURRENT_TIMESTAMP AS TEXT), CAST(CURRENT_TIMESTAMP AS TEXT))
        "#,
        id,
        owner_id.as_uuid(),
        room_id
    )
    .execute(pool)
    .await?;

    get_conversation(pool, &id, owner_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_conversation(
    conn: impl sqlx::PgExecutor<'_>,
    id: &str,
    owner_id: &AccountId,
) -> Result<Option<AiConversation>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, owner_id, room_id, created_at, updated_at
        FROM ai_conversations
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|r| AiConversation {
        id: r.id,
        owner_id: AccountId::from_uuid(r.owner_id).to_string(),
        room_id: r.room_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

pub async fn list_conversations(
    conn: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
    limit: i64,
) -> Result<Vec<AiConversation>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, room_id, created_at, updated_at
        FROM ai_conversations
        WHERE owner_id = $1
        ORDER BY updated_at DESC
        LIMIT $2
        "#,
        owner_id.as_uuid(),
        limit
    )
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| AiConversation {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id).to_string(),
            room_id: r.room_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn delete_conversation(
    conn: impl sqlx::PgExecutor<'_>,
    id: &str,
    owner_id: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        DELETE FROM ai_conversations
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_uuid()
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn touch_conversation(
    conn: impl sqlx::PgExecutor<'_>,
    id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        UPDATE ai_conversations
        SET updated_at = CAST(CURRENT_TIMESTAMP AS TEXT)
        WHERE id = $1
        "#,
        id
    )
    .execute(conn)
    .await?;

    Ok(())
}

pub async fn add_message(
    pool: &sqlx::PgPool,
    conversation_id: &str,
    role: &str,
    content: &str,
    tool_call_id: Option<&str>,
    tool_name: Option<&str>,
) -> Result<AiMessage, sqlx::Error> {
    let id = format!("msg_{}", Uuid::new_v4().simple());
    sqlx::query!(
        r#"
        INSERT INTO ai_messages (id, conversation_id, role, content, tool_call_id, tool_name, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, CAST(CURRENT_TIMESTAMP AS TEXT))
        "#,
        id,
        conversation_id,
        role,
        content,
        tool_call_id,
        tool_name
    )
    .execute(pool)
    .await?;

    touch_conversation(pool, conversation_id).await?;

    let row = sqlx::query!(
        r#"
        SELECT id, conversation_id, role, content, tool_call_id, tool_name, created_at
        FROM ai_messages
        WHERE id = $1
        "#,
        id
    )
    .fetch_one(pool)
    .await?;

    Ok(AiMessage {
        id: row.id,
        conversation_id: row.conversation_id,
        role: row.role,
        content: row.content,
        tool_call_id: row.tool_call_id,
        tool_name: row.tool_name,
        created_at: row.created_at,
    })
}

pub async fn get_recent_messages(
    conn: impl sqlx::PgExecutor<'_>,
    conversation_id: &str,
    limit: i64,
) -> Result<Vec<AiMessage>, sqlx::Error> {
    let mut rows = sqlx::query!(
        r#"
        SELECT id, conversation_id, role, content, tool_call_id, tool_name, created_at
        FROM ai_messages
        WHERE conversation_id = $1 AND (role != 'assistant' OR TRIM(content) != '')
        ORDER BY created_at DESC
        LIMIT $2
        "#,
        conversation_id,
        limit
    )
    .fetch_all(conn)
    .await?;

    rows.reverse();

    Ok(rows
        .into_iter()
        .map(|r| AiMessage {
            id: r.id,
            conversation_id: r.conversation_id,
            role: r.role,
            content: r.content,
            tool_call_id: r.tool_call_id,
            tool_name: r.tool_name,
            created_at: r.created_at,
        })
        .collect())
}

pub struct AiRepository;

impl AiRepository {
    pub async fn get_user_config(
        pool: &sqlx::PgPool,
        owner_id: &AccountId,
    ) -> Result<Option<AiUserConfig>, sqlx::Error> {
        get_user_config(pool, owner_id).await
    }

    pub async fn upsert_user_config(
        pool: &sqlx::PgPool,
        owner_id: &AccountId,
        input: &UpdateAiConfigInput,
    ) -> Result<AiUserConfig, sqlx::Error> {
        upsert_user_config(pool, owner_id, input).await
    }

    pub async fn get_or_create_conversation(
        pool: &sqlx::PgPool,
        owner_id: &AccountId,
        room_id: &str,
        max_age_seconds: i64,
    ) -> Result<AiConversation, sqlx::Error> {
        get_or_create_conversation(pool, owner_id, room_id, max_age_seconds).await
    }

    pub async fn get_conversation(
        pool: &sqlx::PgPool,
        id: &str,
        owner_id: &AccountId,
    ) -> Result<Option<AiConversation>, sqlx::Error> {
        get_conversation(pool, id, owner_id).await
    }

    pub async fn list_conversations(
        pool: &sqlx::PgPool,
        owner_id: &AccountId,
        limit: i64,
    ) -> Result<Vec<AiConversation>, sqlx::Error> {
        list_conversations(pool, owner_id, limit).await
    }

    pub async fn delete_conversation(
        pool: &sqlx::PgPool,
        id: &str,
        owner_id: &AccountId,
    ) -> Result<bool, sqlx::Error> {
        delete_conversation(pool, id, owner_id).await
    }

    pub async fn touch_conversation(pool: &sqlx::PgPool, id: &str) -> Result<(), sqlx::Error> {
        touch_conversation(pool, id).await
    }

    pub async fn add_message(
        pool: &sqlx::PgPool,
        conversation_id: &str,
        role: &str,
        content: &str,
        tool_call_id: Option<&str>,
        tool_name: Option<&str>,
    ) -> Result<AiMessage, sqlx::Error> {
        add_message(
            pool,
            conversation_id,
            role,
            content,
            tool_call_id,
            tool_name,
        )
        .await
    }

    pub async fn get_recent_messages(
        pool: &sqlx::PgPool,
        conversation_id: &str,
        limit: i64,
    ) -> Result<Vec<AiMessage>, sqlx::Error> {
        get_recent_messages(pool, conversation_id, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ai_repository_crud() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let owner_str =
            crate::accounts::AccountRepository::resolve_or_provision(&pool, "@tester:mbm.one")
                .await
                .unwrap();
        let owner = AccountId::from(owner_str.as_str());

        let initial = get_user_config(&pool, &owner).await.unwrap();
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

        let saved = upsert_user_config(&pool, &owner, &input).await.unwrap();
        assert_eq!(saved.owner_id, owner.to_string());
        assert_eq!(saved.model_name.as_deref(), Some("gpt-4o"));
        assert!(saved.has_api_key);

        let conv = get_or_create_conversation(&pool, &owner, "!room:test", 3600)
            .await
            .unwrap();
        assert_eq!(conv.owner_id, owner.to_string());

        let msg = add_message(&pool, &conv.id, "user", "Hello bot!", None, None)
            .await
            .unwrap();
        assert_eq!(msg.content, "Hello bot!");

        let recent = get_recent_messages(&pool, &conv.id, 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].content, "Hello bot!");

        let deleted = delete_conversation(&pool, &conv.id, &owner).await.unwrap();
        assert!(deleted);
    }
}
