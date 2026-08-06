use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::todo::repository::TodoRepository;

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct TodoQuery {
    pub room_id: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateTodoRequest {
    pub room_id: Option<String>,
    pub text: String,
    pub priority: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateTodoRequest {
    pub text: Option<String>,
    pub priority: Option<String>,
    pub completed: Option<bool>,
    pub due_at: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TodoListResponse {
    pub items: Vec<TodoItem>,
    pub page: i64,
    pub limit: i64,
    /// Total rows matching the query, so clients can render page counts.
    pub total: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/todos",
    params(TodoQuery),
    responses(
        (status = 200, description = "List todo items", body = TodoListResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_todos(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<TodoQuery>,
) -> Result<Json<TodoListResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * limit;
    let owner_id = user.owner_id();

    let (items, total) = if let Some(room) = &query.room_id {
        let items = sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE room_id = ? AND sender_mxid = ? ORDER BY completed ASC, id DESC LIMIT ? OFFSET ?",
        )
        .bind(room)
        .bind(&owner_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM todos WHERE room_id = ? AND sender_mxid = ?",
        )
        .bind(room)
        .bind(&owner_id)
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        (items, total)
    } else {
        let items = TodoRepository::list_all_user(&state.db.pool, &owner_id, limit, offset)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let total = TodoRepository::count_all_user(&state.db.pool, &owner_id)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        (items, total)
    };

    Ok(Json(TodoListResponse { items, page, limit, total }))
}

#[utoipa::path(
    post,
    path = "/api/v1/todos",
    request_body = CreateTodoRequest,
    responses(
        (status = 200, description = "Todo created", body = TodoItem),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateTodoRequest>,
) -> Result<Json<TodoItem>, ApiError> {
    if payload.text.trim().is_empty() {
        return Err(ApiError::bad_request("Task text required"));
    }

    if payload.text.len() > 500 {
        return Err(ApiError::bad_request("Task text exceeds 500 character limit"));
    }

    let priority = payload
        .priority
        .as_deref()
        .map(TodoPriority::from_str)
        .unwrap_or(TodoPriority::Medium);

    let room_id = payload.room_id.unwrap_or_else(|| "web".to_string());

    let item = TodoRepository::add(
        &state.db.pool,
        &room_id,
        &user.owner_id(),
        &user.display_name(),
        &payload.text,
        priority,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::TodoCreated(item.clone()));

    Ok(Json(item))
}

#[utoipa::path(
    patch,
    path = "/api/v1/todos/{id}",
    params(("id" = i64, Path, description = "Todo item ID")),
    request_body = UpdateTodoRequest,
    responses(
        (status = 200, description = "Todo updated", body = TodoItem),
        (status = 404, description = "Todo not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateTodoRequest>,
) -> Result<Json<TodoItem>, ApiError> {
    let mut item = sqlx::query_as::<_, TodoItem>(
        "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ? AND sender_mxid = ?",
    )
    .bind(id)
    .bind(user.owner_id())
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Task not found or access denied"))?;

    if let Some(text) = payload.text {
        TodoRepository::edit_text(&state.db.pool, id, &text)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        item.text = text;
    }

    if let Some(prio) = payload.priority {
        let priority_obj = TodoPriority::from_str(&prio);
        TodoRepository::set_priority(&state.db.pool, id, priority_obj)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        item.priority = prio;
    }

    if let Some(completed) = payload.completed {
        TodoRepository::set_completed(&state.db.pool, id, completed)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        item.completed = completed;
    }

    if let Some(due) = payload.due_at {
        TodoRepository::set_due_date(&state.db.pool, id, &due)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        item.due_at = Some(due);
    }

    state.broadcaster.send(SystemEvent::TodoUpdated(item.clone()));

    Ok(Json(item))
}

#[utoipa::path(
    delete,
    path = "/api/v1/todos/{id}",
    params(("id" = i64, Path, description = "Todo item ID")),
    responses(
        (status = 204, description = "Todo deleted"),
        (status = 404, description = "Todo not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let owner_id = user.owner_id();
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT room_id FROM todos WHERE id = ? AND sender_mxid = ?",
    )
    .bind(id)
    .bind(&owner_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let (room_id,) = match row {
        Some(r) => r,
        None => return Err(ApiError::not_found("Task not found or access denied")),
    };

    sqlx::query("DELETE FROM todos WHERE id = ?")
        .bind(id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::TodoDeleted { id, room_id, owner_id });

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_todo_api_crud_and_isolation() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::local("alice");
        let bob = AuthUser::local("bob");

        // Alice creates todo
        let req = CreateTodoRequest {
            room_id: Some("!room:1".to_string()),
            text: "Alice task".to_string(),
            priority: Some("high".to_string()),
        };
        let created = create_todo(State(state.clone()), alice.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(created.text, "Alice task");

        // Bob tries to list Alice's tasks -> gets empty list
        let query = TodoQuery {
            room_id: None,
            page: None,
            limit: None,
        };
        let bob_list = list_todos(State(state.clone()), bob.clone(), Query(query))
            .await
            .unwrap();
        assert_eq!(bob_list.items.len(), 0);

        // Bob tries to delete Alice's task -> gets 404/access denied error
        let err = delete_todo(State(state.clone()), bob.clone(), Path(created.id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        // Alice updates her own task -> succeeds
        let update_req = UpdateTodoRequest {
            text: Some("Alice task updated".to_string()),
            priority: None,
            completed: Some(true),
            due_at: None,
        };
        let updated = update_todo(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(update_req),
        )
        .await
        .unwrap();
        assert_eq!(updated.text, "Alice task updated");

        // Alice deletes her task -> succeeds
        let status = delete_todo(State(state), alice, Path(created.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// `completed` is a boolean on the wire, so it has to work in both
    /// directions — `false` used to be accepted and silently ignored.
    #[tokio::test]
    async fn test_completion_can_be_toggled_back_off() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::local("alice");

        let created = create_todo(
            State(state.clone()),
            alice.clone(),
            Json(CreateTodoRequest {
                room_id: None,
                text: "Water the plants".to_string(),
                priority: None,
            }),
        )
        .await
        .unwrap();
        assert!(!created.completed);

        let set = |flag: bool| UpdateTodoRequest {
            text: None,
            priority: None,
            completed: Some(flag),
            due_at: None,
        };

        let done = update_todo(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(set(true)),
        )
        .await
        .unwrap();
        assert!(done.completed);

        let undone = update_todo(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(set(false)),
        )
        .await
        .unwrap();
        assert!(!undone.completed, "response must reflect the un-completion");

        // And it must have actually persisted, not just been echoed back.
        let listed = list_todos(
            State(state),
            alice,
            Query(TodoQuery { room_id: None, page: None, limit: None }),
        )
        .await
        .unwrap();
        assert_eq!(listed.items.len(), 1);
        assert!(!listed.items[0].completed);
    }

    /// A local account and a Matrix user sharing a name are different people.
    ///
    /// Ownership used to also match on `sender_name`, so registering the local
    /// account `evan` was enough to read and delete `@evan:mbm.one`'s tasks.
    #[tokio::test]
    async fn test_local_account_cannot_reach_matrix_user_data() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let matrix_evan = AuthUser::matrix("@evan:mbm.one");
        let impostor = AuthUser::local("evan");

        // Both derive the same display name — that must not grant access.
        assert_eq!(matrix_evan.display_name(), impostor.display_name());

        let created = create_todo(
            State(state.clone()),
            matrix_evan.clone(),
            Json(CreateTodoRequest {
                room_id: Some("!room:mbm.one".to_string()),
                text: "Matrix-owned task".to_string(),
                priority: None,
            }),
        )
        .await
        .unwrap();

        // Cross-room listing
        let listed = list_todos(
            State(state.clone()),
            impostor.clone(),
            Query(TodoQuery { room_id: None, page: None, limit: None }),
        )
        .await
        .unwrap();
        assert_eq!(listed.items.len(), 0, "local account must not see Matrix user's tasks");

        // Room-scoped listing takes a different code path — check it too.
        let listed_in_room = list_todos(
            State(state.clone()),
            impostor.clone(),
            Query(TodoQuery {
                room_id: Some("!room:mbm.one".to_string()),
                page: None,
                limit: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(listed_in_room.items.len(), 0);

        // Mutation and deletion must both be refused.
        let err = update_todo(
            State(state.clone()),
            impostor.clone(),
            Path(created.id),
            Json(UpdateTodoRequest {
                text: Some("hijacked".to_string()),
                priority: None,
                completed: None,
                due_at: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        let err = delete_todo(State(state.clone()), impostor, Path(created.id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        // The real owner still has full access.
        let owner_view = list_todos(
            State(state),
            matrix_evan,
            Query(TodoQuery { room_id: None, page: None, limit: None }),
        )
        .await
        .unwrap();
        assert_eq!(owner_view.items.len(), 1);
        assert_eq!(owner_view.items[0].text, "Matrix-owned task");
    }
}
