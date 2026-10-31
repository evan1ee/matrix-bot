//! REST handlers for note wiki-links and the knowledge graph.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::application::knowledge::{KnowledgeGraph, LinkedNote, NoteLinks};
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

/// One end of a link. `note_id` is null for a link to a title with no note yet.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct NoteLinkDto {
    pub note_id: Option<i64>,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NoteLinksResponse {
    /// Notes (or not-yet-existing titles) this note links to.
    pub outgoing: Vec<NoteLinkDto>,
    /// Notes that link to this note.
    pub backlinks: Vec<NoteLinkDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphNode {
    pub id: i64,
    pub title: String,
    pub note_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GraphEdge {
    pub source: i64,
    pub target: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KnowledgeGraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl From<LinkedNote> for NoteLinkDto {
    fn from(link: LinkedNote) -> Self {
        Self {
            note_id: link.note_id,
            title: link.title,
        }
    }
}

impl From<NoteLinks> for NoteLinksResponse {
    fn from(links: NoteLinks) -> Self {
        Self {
            outgoing: links.outgoing.into_iter().map(Into::into).collect(),
            backlinks: links.backlinks.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<KnowledgeGraph> for KnowledgeGraphResponse {
    fn from(graph: KnowledgeGraph) -> Self {
        Self {
            nodes: graph
                .nodes
                .into_iter()
                .map(|n| GraphNode {
                    id: n.id,
                    title: n.title,
                    note_type: n.note_type.as_str().to_string(),
                })
                .collect(),
            edges: graph
                .edges
                .into_iter()
                .map(|(source, target)| GraphEdge { source, target })
                .collect(),
        }
    }
}

#[utoipa::path(
    get,
    tag = "knowledge",
    path = "/api/v1/notes/{id}/links",
    params(("id" = i64, Path, description = "Note ID")),
    responses(
        (status = 200, body = NoteLinksResponse),
        (status = 404, description = "Note not found")
    ),
    security(("session_cookie" = []))
)]
pub async fn get_note_links(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<NoteLinksResponse>, ApiError> {
    let links = state
        .services
        .notes
        .links(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(links.into()))
}

#[utoipa::path(
    get,
    tag = "knowledge",
    path = "/api/v1/notes/graph",
    responses((status = 200, body = KnowledgeGraphResponse)),
    security(("session_cookie" = []))
)]
pub async fn get_knowledge_graph(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<KnowledgeGraphResponse>, ApiError> {
    let graph = state
        .services
        .notes
        .graph(&user.account_id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(graph.into()))
}
