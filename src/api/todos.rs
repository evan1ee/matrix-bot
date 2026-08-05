use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::auth::{ApiState, AuthUser};
use crate::events::SystemEvent;
use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::todo::repository::TodoRepository;

#[derive(Deserialize)]
pub struct TodoQuery {
    pub room_id: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateTodoRequest {
    pub room_id: String,
    pub text: String,
    pub priority: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateTodoRequest {
    pub text: Option<String>,
    pub priority: Option<String>,
    pub completed: Option<bool>,
    pub due_at: Option<String>,
}

#[derive(Serialize)]
pub struct TodoListResponse {
    pub items: Vec<TodoItem>,
    pub page: i64,
    pub limit: i64,
}

pub async fn list_todos(
    State(state): State<ApiState>,
    _user: AuthUser,
    Query(query): Query<TodoQuery>,
) -> Result<Json<TodoListResponse>, (StatusCode, String)> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * limit;

    let items = if let Some(room) = &query.room_id {
        TodoRepository::list_room(&state.db.pool, room, limit, offset)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos ORDER BY completed ASC, id DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    Ok(Json(TodoListResponse { items, page, limit }))
}

pub async fn create_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateTodoRequest>,
) -> Result<Json<TodoItem>, (StatusCode, String)> {
    if payload.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Task text required".to_string()));
    }

    let priority = payload
        .priority
        .as_deref()
        .map(TodoPriority::from_str)
        .unwrap_or(TodoPriority::Medium);

    let sender_mxid = format!("@web-{}:api", user.username);

    let item = TodoRepository::add(
        &state.db.pool,
        &payload.room_id,
        &sender_mxid,
        &user.username,
        &payload.text,
        priority,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.broadcaster.send(SystemEvent::TodoCreated(item.clone()));

    Ok(Json(item))
}

pub async fn update_todo(
    State(state): State<ApiState>,
    _user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateTodoRequest>,
) -> Result<Json<TodoItem>, (StatusCode, String)> {
    let mut item = sqlx::query_as::<_, TodoItem>(
        "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or((StatusCode::NOT_FOUND, "Task not found".to_string()))?;

    if let Some(text) = payload.text {
        TodoRepository::edit_text(&state.db.pool, &item.room_id, id, &text)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        item.text = text;
    }

    if let Some(prio) = payload.priority {
        let priority_obj = TodoPriority::from_str(&prio);
        TodoRepository::set_priority(&state.db.pool, &item.room_id, id, priority_obj)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        item.priority = prio;
    }

    if let Some(completed) = payload.completed {
        if completed {
            TodoRepository::mark_done(&state.db.pool, &item.room_id, id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            item.completed = true;
        }
    }

    if let Some(due) = payload.due_at {
        TodoRepository::set_due_date(&state.db.pool, &item.room_id, id, &due)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        item.due_at = Some(due);
    }

    state.broadcaster.send(SystemEvent::TodoUpdated(item.clone()));

    Ok(Json(item))
}

pub async fn delete_todo(
    State(state): State<ApiState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    let row: Option<(String,)> = sqlx::query_as("SELECT room_id FROM todos WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (room_id,) = match row {
        Some(r) => r,
        None => return Err((StatusCode::NOT_FOUND, "Task not found".to_string())),
    };

    sqlx::query("DELETE FROM todos WHERE id = ?")
        .bind(id)
        .execute(&state.db.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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
        let state = ApiState {
            db: crate::db::Database { pool },
            broadcaster: EventBroadcaster::default(),
            jwt_secret: "secret".to_string(),
        };
        let user = AuthUser {
            username: "alice".to_string(),
        };

        // Create
        let req = CreateTodoRequest {
            room_id: "!room:1".to_string(),
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
