//! Wiki-link maintenance and reads for notes.
//!
//! Links are derived from the Markdown, so they are rewritten inside the same
//! transaction as every note write that can change them; nothing else writes
//! `knowledge_note_links`.

use super::notes::NoteService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::knowledge::links::extract_links;
use crate::infrastructure::postgres::repositories::knowledge::links::{self as repo, GraphNodeRow};
pub use repo::LinkedNote;

/// Most notes the graph returns, newest edits first; the force layout is
/// unreadable (and the payload large) far beyond this.
pub const MAX_GRAPH_NODES: i64 = 500;

/// A note's links in both directions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLinks {
    pub outgoing: Vec<LinkedNote>,
    pub backlinks: Vec<LinkedNote>,
}

/// The owner's active notes and the resolved links between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNodeRow>,
    /// `(source, target)` note ids.
    pub edges: Vec<(i64, i64)>,
}

/// Brings the link table in line with a note that was just created or edited.
/// `renamed` is true when the title changed (never true for a create, whose
/// dangling links are resolved by `created`).
pub(super) async fn sync_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: &AccountId,
    note_id: i64,
    title: &str,
    markdown: &str,
    previous_title: Option<&str>,
) -> Result<(), AppError> {
    repo::lock_owner(tx, owner).await?;
    if previous_title.is_some_and(|old| old != title) {
        repo::re_resolve_stale(&mut **tx, owner, note_id, title).await?;
    }
    if previous_title != Some(title) {
        repo::resolve_dangling(&mut **tx, owner, note_id, title).await?;
    }
    let titles: Vec<String> = extract_links(markdown, title)
        .into_iter()
        .map(|l| l.target_title)
        .collect();
    repo::replace_outgoing(tx, owner, note_id, &titles).await?;
    Ok(())
}

/// After a note was deleted: links that pointed at it may have another note
/// with the same title to land on.
pub(super) async fn after_delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: &AccountId,
) -> Result<(), AppError> {
    repo::lock_owner(tx, owner).await?;
    repo::resolve_all_dangling(&mut **tx, owner).await?;
    Ok(())
}

impl NoteService {
    /// Outgoing links and backlinks of one note.
    pub async fn links(&self, owner: &AccountId, id: i64) -> Result<NoteLinks, AppError> {
        let exists = crate::infrastructure::postgres::repositories::knowledge::get_note_by_id(
            &self.pool, id, owner,
        )
        .await?
        .is_some();
        if !exists {
            return Err(AppError::NotFound("Note not found".to_string()));
        }
        Ok(NoteLinks {
            outgoing: repo::list_outgoing(&self.pool, owner, id).await?,
            backlinks: repo::list_backlinks(&self.pool, owner, id).await?,
        })
    }

    /// The owner's knowledge graph (active notes only).
    pub async fn graph(&self, owner: &AccountId) -> Result<KnowledgeGraph, AppError> {
        let nodes = repo::graph_nodes(&self.pool, owner, MAX_GRAPH_NODES).await?;
        let ids: Vec<i64> = nodes.iter().map(|n| n.id).collect();
        let edges = repo::graph_edges(&self.pool, owner, &ids).await?;
        Ok(KnowledgeGraph { nodes, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::domain::knowledge::note::{NewNote, NotePatch};

    async fn setup() -> (NoteService, AccountId, sqlx::PgPool) {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let owner = AccountId::generate();
        seed_account(&pool, &owner).await;
        (NoteService::new(pool.clone()), owner, pool)
    }

    async fn note(svc: &NoteService, owner: &AccountId, title: &str, body: &str) -> i64 {
        svc.create(owner, NewNote::new(title, body))
            .await
            .unwrap()
            .id
            .0
    }

    fn link(note_id: Option<i64>, title: &str) -> LinkedNote {
        LinkedNote {
            note_id,
            title: title.to_string(),
        }
    }

    #[tokio::test]
    async fn a_link_resolves_and_shows_as_a_backlink() {
        let (svc, alice, _pool) = setup().await;
        let a = note(&svc, &alice, "Note A", "base").await;
        let b = note(
            &svc,
            &alice,
            "Note B",
            "points at [[note a]] and `[[Code]]`",
        )
        .await;

        let from_b = svc.links(&alice, b).await.unwrap();
        assert_eq!(from_b.outgoing, vec![link(Some(a), "Note A")]);
        let to_a = svc.links(&alice, a).await.unwrap();
        assert_eq!(to_a.backlinks, vec![link(Some(b), "Note B")]);
        assert!(to_a.outgoing.is_empty());
    }

    #[tokio::test]
    async fn a_dangling_link_resolves_when_the_target_is_created_or_renamed_into_place() {
        let (svc, alice, _pool) = setup().await;
        let src = note(&svc, &alice, "Source", "[[Later]] [[Renamed]]").await;
        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(None, "Later"), link(None, "Renamed")]
        );

        let later = note(&svc, &alice, "Later", "x").await;
        let other = note(&svc, &alice, "Other", "x").await;
        svc.update(
            &alice,
            other,
            NotePatch {
                title: Some("renamed".to_string()),
                ..NotePatch::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(Some(later), "Later"), link(Some(other), "renamed")]
        );
    }

    #[tokio::test]
    async fn renaming_a_target_away_unresolves_links_but_keeps_the_written_title() {
        let (svc, alice, _pool) = setup().await;
        let target = note(&svc, &alice, "Old name", "x").await;
        let src = note(&svc, &alice, "Source", "[[Old name]]").await;
        svc.update(
            &alice,
            target,
            NotePatch {
                title: Some("New name".to_string()),
                ..NotePatch::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(None, "Old name")]
        );
        assert!(svc
            .links(&alice, target)
            .await
            .unwrap()
            .backlinks
            .is_empty());
    }

    #[tokio::test]
    async fn deleting_a_target_repoints_links_to_a_note_with_the_same_title() {
        let (svc, alice, _pool) = setup().await;
        let first = note(&svc, &alice, "Twin", "x").await;
        let second = note(&svc, &alice, "twin", "y").await;
        let src = note(&svc, &alice, "Source", "[[Twin]]").await;
        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(Some(first), "Twin")]
        );

        svc.delete_note(&alice, first).await.unwrap();

        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(Some(second), "twin")]
        );
    }

    #[tokio::test]
    async fn deleting_a_target_dangles_and_deleting_a_source_removes_its_links() {
        let (svc, alice, pool) = setup().await;
        let target = note(&svc, &alice, "Target", "x").await;
        let src = note(&svc, &alice, "Source", "[[Target]]").await;

        svc.delete_note(&alice, target).await.unwrap();
        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(None, "Target")]
        );

        svc.delete_note(&alice, src).await.unwrap();
        let rows = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM knowledge_note_links"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn editing_the_body_replaces_the_links_and_self_links_are_dropped() {
        let (svc, alice, _pool) = setup().await;
        let a = note(&svc, &alice, "A", "x").await;
        let b = note(&svc, &alice, "B", "x").await;
        let src = note(&svc, &alice, "Src", "[[A]] [[Src]]").await;
        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(Some(a), "A")]
        );

        svc.update(
            &alice,
            src,
            NotePatch {
                markdown: Some("now [[B]]".to_string()),
                ..NotePatch::default()
            },
        )
        .await
        .unwrap();
        svc.append(&alice, src, "and [[A]] again").await.unwrap();

        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(Some(b), "B"), link(Some(a), "A")]
        );
    }

    #[tokio::test]
    async fn titles_never_resolve_across_owners() {
        let (svc, alice, pool) = setup().await;
        let bob = AccountId::generate();
        seed_account(&pool, &bob).await;
        let bobs = note(&svc, &bob, "Shared title", "x").await;
        let src = note(&svc, &alice, "Mine", "[[Shared title]]").await;

        assert_eq!(
            svc.links(&alice, src).await.unwrap().outgoing,
            vec![link(None, "Shared title")]
        );
        assert!(svc.links(&bob, bobs).await.unwrap().backlinks.is_empty());
        assert!(matches!(
            svc.links(&bob, src).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn the_graph_has_active_notes_and_their_resolved_edges_only() {
        let (svc, alice, _pool) = setup().await;
        let a = note(&svc, &alice, "A", "[[B]] [[Archived]] [[Ghost]]").await;
        let b = note(&svc, &alice, "B", "[[A]]").await;
        let archived = note(&svc, &alice, "Archived", "x").await;
        svc.archive_note(&alice, archived, true).await.unwrap();

        let graph = svc.graph(&alice).await.unwrap();
        let mut ids: Vec<i64> = graph.nodes.iter().map(|n| n.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![a, b]);
        let mut edges = graph.edges;
        edges.sort_unstable();
        assert_eq!(edges, vec![(a, b), (b, a)]);
    }
}
