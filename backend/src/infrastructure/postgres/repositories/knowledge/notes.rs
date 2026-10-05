//! PostgreSQL repository for notes CRUD operations.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::{Note, NoteFormat, NoteId};

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct NoteRow {
    pub id: i64,
    pub owner_id: uuid::Uuid,
    pub title: String,
    pub content: String,
    pub format: String,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub is_archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<NoteRow> for Note {
    fn from(row: NoteRow) -> Self {
        Self {
            id: NoteId(row.id),
            owner_id: AccountId::from_uuid(row.owner_id),
            title: row.title,
            content: row.content,
            format: row.format.parse().unwrap_or(NoteFormat::Markdown),
            source_channel: row.source_channel,
            source_ref: row.source_ref,
            tags: row.tags,
            is_archived: row.is_archived,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub async fn insert_note(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    title: &str,
    content: &str,
    format: &str,
    source_channel: &str,
    source_ref: Option<&str>,
    tags: &[String],
) -> Result<Note, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        INSERT INTO notes (
            owner_id, title, content, format, source_channel, source_ref, tags
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        "#,
        owner.as_uuid(),
        title,
        content,
        format,
        source_channel,
        source_ref,
        tags
    )
    .fetch_one(executor)
    .await?;

    Ok(row.into())
}

pub async fn update_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
    title: &str,
    content: &str,
    format: &str,
    tags: &[String],
) -> Result<Option<Note>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        UPDATE notes
        SET
            title = $3,
            content = $4,
            format = $5,
            tags = $6,
            updated_at = NOW()
        WHERE id = $1 AND owner_id = $2
        RETURNING
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        "#,
        id,
        owner.as_uuid(),
        title,
        content,
        format,
        tags
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn get_note_by_id(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Note>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        SELECT
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_notes(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    tag_filter: Option<&str>,
    include_archived: bool,
    limit: i64,
) -> Result<Vec<Note>, sqlx::Error> {
    let rows = sqlx::query_as!(
        NoteRow,
        r#"
        SELECT
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        FROM notes
        WHERE owner_id = $1
          AND ($2::TEXT IS NULL OR $2 = ANY(tags))
          AND ($3::BOOLEAN OR is_archived = FALSE)
        ORDER BY updated_at DESC
        LIMIT $4
        "#,
        owner.as_uuid(),
        tag_filter,
        include_archived,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn archive_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
    is_archived: bool,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        UPDATE notes
        SET is_archived = $3, updated_at = NOW()
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid(),
        is_archived
    )
    .execute(executor)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn delete_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        DELETE FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .execute(executor)
    .await?;

    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};

    #[tokio::test]
    async fn test_note_crud_and_isolation() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        let bob = AccountId::generate();
        seed_account(&pool, &alice).await;
        seed_account(&pool, &bob).await;

        let note = insert_note(
            &pool,
            &alice,
            "Deep Work",
            "Focus without distraction",
            "markdown",
            "web",
            None,
            &["productivity".to_string(), "focus".to_string()],
        )
        .await
        .expect("insert note");

        assert_eq!(note.title, "Deep Work");
        assert_eq!(note.tags, vec!["productivity", "focus"]);
        assert_eq!(note.owner_id, alice);

        // Bob cannot read Alice's note
        let bob_read = get_note_by_id(&pool, note.id.0, &bob)
            .await
            .expect("query note");
        assert!(bob_read.is_none());

        // Alice reads her note
        let alice_read = get_note_by_id(&pool, note.id.0, &alice)
            .await
            .expect("query note")
            .expect("note found");
        assert_eq!(alice_read.id, note.id);

        // Update note
        let updated = update_note(
            &pool,
            note.id.0,
            &alice,
            "Deep Work 2.0",
            "Updated focus tactics",
            "markdown",
            &["focus".to_string()],
        )
        .await
        .expect("update note")
        .expect("note returned");
        assert_eq!(updated.title, "Deep Work 2.0");
        assert_eq!(updated.tags, vec!["focus"]);

        // List notes with tag filter
        let list_filtered = list_notes(&pool, &alice, Some("focus"), false, 10)
            .await
            .expect("list notes");
        assert_eq!(list_filtered.len(), 1);

        let list_unmatched = list_notes(&pool, &alice, Some("nonexistent"), false, 10)
            .await
            .expect("list notes");
        assert_eq!(list_unmatched.len(), 0);

        // Archive note
        let archived = archive_note(&pool, note.id.0, &alice, true)
            .await
            .expect("archive note");
        assert!(archived);

        let list_active = list_notes(&pool, &alice, None, false, 10)
            .await
            .expect("list active");
        assert_eq!(list_active.len(), 0);

        let list_all = list_notes(&pool, &alice, None, true, 10)
            .await
            .expect("list all");
        assert_eq!(list_all.len(), 1);

        // Delete note
        let deleted = delete_note(&pool, note.id.0, &alice)
            .await
            .expect("delete note");
        assert!(deleted);

        let read_deleted = get_note_by_id(&pool, note.id.0, &alice)
            .await
            .expect("query note");
        assert!(read_deleted.is_none());
    }
}
