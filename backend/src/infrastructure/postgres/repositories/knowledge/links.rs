//! PostgreSQL repository for note wiki-links (`knowledge_note_links`).
//!
//! Every statement is scoped by `owner_id`, so a title can only ever resolve to
//! one of the same owner's notes.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::NoteType;

/// One end of a link as the UI shows it. `note_id` is `None` for a link whose
/// target note does not exist (yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedNote {
    pub note_id: Option<i64>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNodeRow {
    pub id: i64,
    pub title: String,
    pub note_type: NoteType,
}

/// Replaces `source`'s outgoing links with `titles` (already deduplicated
/// case-insensitively by the caller), resolving each to the lowest-id note of
/// the owner with that title, never `source` itself.
pub async fn replace_outgoing(
    conn: &mut sqlx::PgConnection,
    owner: &AccountId,
    source: i64,
    titles: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM knowledge_note_links WHERE owner_id = $1 AND source_note_id = $2",
        owner.as_uuid(),
        source
    )
    .execute(&mut *conn)
    .await?;
    if titles.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        r#"
        INSERT INTO knowledge_note_links (owner_id, source_note_id, target_note_id, target_title)
        SELECT $1, $2,
               (SELECT n.id FROM notes n
                 WHERE n.owner_id = $1 AND LOWER(n.title) = LOWER(t.title) AND n.id <> $2
                 ORDER BY n.id LIMIT 1),
               t.title
        FROM unnest($3::TEXT[]) AS t(title)
        ON CONFLICT DO NOTHING
        "#,
        owner.as_uuid(),
        source,
        titles
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Points dangling links whose title matches `note`'s current title at it.
/// Run after a note is created or renamed.
pub async fn resolve_dangling(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    note: i64,
    title: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        UPDATE knowledge_note_links
        SET target_note_id = $2
        WHERE owner_id = $1 AND target_note_id IS NULL
          AND LOWER(target_title) = LOWER($3) AND source_note_id <> $2
        "#,
        owner.as_uuid(),
        note,
        title
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// After `note` was renamed to `new_title`: links that pointed at it under the
/// old title are re-resolved against the owner's notes (another note with that
/// title, else dangling).
pub async fn re_resolve_stale(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    note: i64,
    new_title: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        UPDATE knowledge_note_links l
        SET target_note_id = (
            SELECT n.id FROM notes n
             WHERE n.owner_id = l.owner_id AND LOWER(n.title) = LOWER(l.target_title)
               AND n.id <> l.source_note_id
             ORDER BY n.id LIMIT 1)
        WHERE l.owner_id = $1 AND l.target_note_id = $2
          AND LOWER(l.target_title) <> LOWER($3)
        "#,
        owner.as_uuid(),
        note,
        new_title
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Outgoing links of `source`, oldest first.
pub async fn list_outgoing(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    source: i64,
) -> Result<Vec<LinkedNote>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT l.target_note_id, COALESCE(t.title, l.target_title) AS "title!"
        FROM knowledge_note_links l
        LEFT JOIN notes t ON t.id = l.target_note_id
        WHERE l.owner_id = $1 AND l.source_note_id = $2
        ORDER BY l.id
        "#,
        owner.as_uuid(),
        source
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LinkedNote {
            note_id: r.target_note_id,
            title: r.title,
        })
        .collect())
}

/// Notes that link to `target`, most recently edited first.
pub async fn list_backlinks(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    target: i64,
) -> Result<Vec<LinkedNote>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT s.id, s.title
        FROM knowledge_note_links l
        JOIN notes s ON s.id = l.source_note_id
        WHERE l.owner_id = $1 AND l.target_note_id = $2
        ORDER BY s.updated_at DESC, s.id
        "#,
        owner.as_uuid(),
        target
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LinkedNote {
            note_id: Some(r.id),
            title: r.title,
        })
        .collect())
}

/// Active (non-archived) notes, newest edit first, at most `limit`.
pub async fn graph_nodes(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    limit: i64,
) -> Result<Vec<GraphNodeRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, title, note_type
        FROM notes
        WHERE owner_id = $1 AND is_archived = FALSE
        ORDER BY updated_at DESC, id
        LIMIT $2
        "#,
        owner.as_uuid(),
        limit
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(GraphNodeRow {
                id: r.id,
                title: r.title,
                note_type: r.note_type.parse().map_err(
                    |e: crate::domain::knowledge::KnowledgeError| sqlx::Error::Decode(Box::new(e)),
                )?,
            })
        })
        .collect()
}

/// Resolved links between the given notes, as `(source, target)` pairs.
pub async fn graph_edges(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    node_ids: &[i64],
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT source_note_id, target_note_id AS "target_note_id!"
        FROM knowledge_note_links
        WHERE owner_id = $1 AND target_note_id IS NOT NULL
          AND source_note_id = ANY($2) AND target_note_id = ANY($2)
        ORDER BY id
        "#,
        owner.as_uuid(),
        node_ids
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.source_note_id, r.target_note_id))
        .collect())
}
