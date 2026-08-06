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
    _user: AuthUser,
    Query(query): Query<TodoQuery>,
) -> Result<Json<TodoListResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * limit;

    let items = if let Some(room) = &query.room_id {
        TodoRepository::list_room(&state.db.pool, room, limit, offset)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos ORDER BY completed ASC, id DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    };

    Ok(Json(TodoListResponse { items, page, limit }))
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

    let (sender_mxid, sender_name) = if user.username.starts_with('@') {
        let name = user
            .username
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or(&user.username)
            .to_string();
        (user.username.clone(), name)
    } else {
        (format!("@web-{}:api", user.username), user.username.clone())
    };

    let room_id = payload.room_id.unwrap_or_else(|| "web".to_string());

    let item = TodoRepository::add(
        &state.db.pool,
        &room_id,
        &sender_mxid,
        &sender_name,
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
    _user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateTodoRequest>,
) -> Result<Json<TodoItem>, ApiError> {
    let mut item = sqlx::query_as::<_, TodoItem>(
        "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Task not found"))?;

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
        if completed {
            TodoRepository::mark_done(&state.db.pool, id)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            item.completed = true;
        }
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
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT room_id FROM todos WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let (room_id,) = match row {
        Some(r) => r,
        None => return Err(ApiError::not_found("Task not found")),
    };

    sqlx::query("DELETE FROM todos WHERE id = ?")
        .bind(id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::TodoDeleted { id, room_id });

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_todo_api_crud() {
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
        let user = AuthUser {
            username: "alice".to_string(),
        };

        // Create
        let req = CreateTodoRequest {
            room_id: Some("!room:1".to_string()),
            text: "Buy milk".to_string(),
            priority: Some("high".to_string()),
        };
        let created = create_todo(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(created.text, "Buy milk");
        assert_eq!(created.priority, "high");

        // List
        let query = TodoQuery {
            room_id: Some("!room:1".to_string()),
            page: None,
            limit: None,
        };
        let list = list_todos(State(state.clone()), user.clone(), Query(query))
            .await
            .unwrap();
        assert_eq!(list.items.len(), 1);

        // Update
        let update_req = UpdateTodoRequest {
            text: Some("Buy oat milk".to_string()),
            priority: None,
            completed: Some(true),
            due_at: None,
        };
        let updated = update_todo(
            State(state.clone()),
            user.clone(),
            Path(created.id),
            Json(update_req),
        )
        .await
        .unwrap();
        assert_eq!(updated.text, "Buy oat milk");
        assert!(updated.completed);

        // Delete
        let status = delete_todo(State(state), user, Path(created.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
