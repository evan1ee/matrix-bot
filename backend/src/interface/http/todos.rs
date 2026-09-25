use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::todo::repository::TodoRepository;
use crate::modules::todo::service::{self, NewTodo, TodoError, TodoPatch};

impl From<TodoError> for ApiError {
    fn from(err: TodoError) -> Self {
        match err {
            TodoError::NotFound => ApiError::not_found(err.to_string()),
            TodoError::Db(inner) => ApiError::internal(inner.to_string()),
            other => ApiError::bad_request(other.to_string()),
        }
    }
}

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct TodoQuery {
    pub room_id: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    /// Narrows the listing to one completion state. Omitted returns both, so
    /// `total` counts every task — callers wanting an outstanding-work figure
    /// must ask for `completed=false` explicitly.
    pub completed: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateTodoRequest {
    pub room_id: Option<String>,
    pub text: String,
    pub priority: Option<String>,
    /// When the task is due.
    pub due_at: Option<DateTime<Utc>>,
    /// Minutes before `due_at` to send a reminder. Requires `due_at`.
    pub remind_before_minutes: Option<i64>,
    /// Matrix room the due reminder should be delivered to. Defaults to the
    /// task's own room; required when that is the `web` sentinel.
    pub remind_room_id: Option<String>,
}

/// A PATCH body.
///
/// The nullable fields are `Option<Option<T>>` so the three cases stay
/// distinguishable on the wire: absent leaves the value alone, `null` clears it,
/// and a value sets it. A plain `Option<T>` would make "clear the due date"
/// unexpressible.
#[derive(Deserialize, ToSchema)]
pub struct UpdateTodoRequest {
    pub text: Option<String>,
    pub priority: Option<String>,
    pub completed: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub due_at: Option<Option<DateTime<Utc>>>,
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>)]
    pub remind_before_minutes: Option<Option<i64>>,
    pub remind_room_id: Option<String>,
}

/// Distinguishes an explicit JSON `null` from an absent field.
///
/// Serde collapses both to `None` for a plain `Option`; this keeps `null` as
/// `Some(None)` so a PATCH can clear a value.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
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
    security(("session_cookie" = []))
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
        let items = TodoRepository::list_room_user(
            &state.db.pool,
            room,
            &owner_id,
            query.completed,
            limit,
            offset,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let total =
            TodoRepository::count_room_user(&state.db.pool, room, &owner_id, query.completed)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;

        (items, total)
    } else {
        let items = TodoRepository::list_all_user(
            &state.db.pool,
            &owner_id,
            query.completed,
            limit,
            offset,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let total = TodoRepository::count_all_user(&state.db.pool, &owner_id, query.completed)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        (items, total)
    };

    Ok(Json(TodoListResponse {
        items,
        page,
        limit,
        total,
    }))
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
    security(("session_cookie" = []))
)]
pub async fn create_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateTodoRequest>,
) -> Result<Json<TodoItem>, ApiError> {
    let priority = payload
        .priority
        .as_deref()
        .map(TodoPriority::from_str_or_default)
        .unwrap_or(TodoPriority::Medium);

    let rooms = &state.services.rooms;
    let room_id = rooms
        .resolve_target_room(&user.owner_id(), payload.room_id.as_deref())
        .await;

    let remind_room = match payload.remind_room_id.as_deref() {
        Some(rr) => Some(rooms.resolve_target_room(&user.owner_id(), Some(rr)).await),
        None => None,
    };

    let item = service::create(
        &state.db.pool,
        Some(&state.broadcaster),
        &user.owner_id(),
        NewTodo {
            text: &payload.text,
            priority,
            room_id: &room_id,
            sender_name: &user.display_name,
            due_at: payload.due_at,
            remind_before_minutes: payload.remind_before_minutes,
            remind_room_id: remind_room.as_deref(),
        },
    )
    .await?;

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
    security(("session_cookie" = []))
)]
pub async fn update_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateTodoRequest>,
) -> Result<Json<TodoItem>, ApiError> {
    let priority = payload
        .priority
        .as_deref()
        .map(TodoPriority::from_str_or_default);

    let item = service::update(
        &state.db.pool,
        Some(&state.broadcaster),
        &user.owner_id(),
        id,
        TodoPatch {
            text: payload.text.as_deref(),
            priority,
            completed: payload.completed,
            due_at: payload.due_at,
            remind_before_minutes: payload.remind_before_minutes,
            remind_room_id: payload.remind_room_id.as_deref(),
        },
    )
    .await?;

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
    security(("session_cookie" = []))
)]
pub async fn delete_todo(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    service::delete(
        &state.db.pool,
        Some(&state.broadcaster),
        &user.owner_id(),
        id,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use crate::modules::reminder::models::ReminderSource;
    use crate::modules::reminder::repository::ReminderRepository;
    use crate::modules::todo::due_reminder;

    /// `total` must count only the requested completion state.
    ///
    /// The dashboard's outstanding-work tile reads `total` rather than the
    /// returned page, so a filter that narrowed `items` but not the count would
    /// show finished tasks as still to do.
    #[tokio::test]
    async fn test_completed_filter_narrows_both_items_and_total() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::named("alice", "acct_alice");

        let mut ids = Vec::new();
        for text in ["one", "two", "three"] {
            let created = create_todo(
                State(state.clone()),
                alice.clone(),
                Json(CreateTodoRequest {
                    room_id: None,
                    text: text.to_string(),
                    priority: None,
                    due_at: None,
                    remind_before_minutes: None,
                    remind_room_id: None,
                }),
            )
            .await
            .unwrap();
            ids.push(created.id);
        }

        // Finish one of the three.
        let _ = update_todo(
            State(state.clone()),
            alice.clone(),
            Path(ids[0]),
            Json(UpdateTodoRequest {
                text: None,
                priority: None,
                completed: Some(true),
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            }),
        )
        .await
        .unwrap();

        let list = |completed: Option<bool>| {
            let state = state.clone();
            let alice = alice.clone();
            async move {
                list_todos(
                    State(state),
                    alice,
                    Query(TodoQuery {
                        room_id: None,
                        page: None,
                        limit: None,
                        completed,
                    }),
                )
                .await
                .unwrap()
            }
        };

        let all = list(None).await;
        assert_eq!(all.total, 3, "an unfiltered query still counts every task");
        assert_eq!(all.items.len(), 3);

        let outstanding = list(Some(false)).await;
        assert_eq!(outstanding.total, 2, "only the two unfinished tasks count");
        assert_eq!(outstanding.items.len(), 2);
        assert!(outstanding.items.iter().all(|t| !t.completed));

        let done = list(Some(true)).await;
        assert_eq!(done.total, 1);
        assert!(done.items.iter().all(|t| t.completed));
    }

    #[tokio::test]
    async fn test_todo_api_crud_and_isolation() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::named("alice", "acct_alice");
        let bob = AuthUser::named("bob", "acct_bob");

        // Alice creates todo
        let req = CreateTodoRequest {
            room_id: Some("!room:1".to_string()),
            text: "Alice task".to_string(),
            priority: Some("high".to_string()),
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
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
            completed: None,
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
            remind_before_minutes: None,
            remind_room_id: None,
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let alice = AuthUser::named("alice", "acct_alice");

        let created = create_todo(
            State(state.clone()),
            alice.clone(),
            Json(CreateTodoRequest {
                room_id: None,
                text: "Water the plants".to_string(),
                priority: None,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
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
            remind_before_minutes: None,
            remind_room_id: None,
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
            Query(TodoQuery {
                room_id: None,
                page: None,
                limit: None,
                completed: None,
            }),
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        // Two accounts whose users call themselves the same thing. The display
        // name is what a room shows and what a person recognises; it has never
        // been an ownership key, and this proves it still is not.
        let matrix_evan = AuthUser::named("evan", "acct_from_matrix");
        let impostor = AuthUser::named("evan", "acct_from_web");

        assert_eq!(matrix_evan.display_name, impostor.display_name);
        assert_ne!(matrix_evan.owner_id(), impostor.owner_id());

        let created = create_todo(
            State(state.clone()),
            matrix_evan.clone(),
            Json(CreateTodoRequest {
                room_id: Some("!room:mbm.one".to_string()),
                text: "Matrix-owned task".to_string(),
                priority: None,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            }),
        )
        .await
        .unwrap();

        // Cross-room listing
        let listed = list_todos(
            State(state.clone()),
            impostor.clone(),
            Query(TodoQuery {
                room_id: None,
                page: None,
                limit: None,
                completed: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            listed.items.len(),
            0,
            "local account must not see Matrix user's tasks"
        );

        // Room-scoped listing takes a different code path — check it too.
        let listed_in_room = list_todos(
            State(state.clone()),
            impostor.clone(),
            Query(TodoQuery {
                room_id: Some("!room:mbm.one".to_string()),
                page: None,
                limit: None,
                completed: None,
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
                remind_before_minutes: None,
                remind_room_id: None,
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
            Query(TodoQuery {
                room_id: None,
                page: None,
                limit: None,
                completed: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(owner_view.items.len(), 1);
        assert_eq!(owner_view.items[0].text, "Matrix-owned task");
    }

    async fn due_reminder_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    fn create_with_due(
        due_at: Option<DateTime<Utc>>,
        lead: Option<i64>,
        remind_room_id: Option<&str>,
    ) -> CreateTodoRequest {
        CreateTodoRequest {
            room_id: None, // -> the `web` sentinel
            text: "Ship the release".to_string(),
            priority: None,
            due_at,
            remind_before_minutes: lead,
            remind_room_id: remind_room_id.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn test_create_with_due_date_schedules_a_linked_reminder() {
        let state = due_reminder_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        let due = crate::utils::time::now_at_storage_precision() + chrono::Duration::hours(6);

        let created = create_todo(
            State(state.clone()),
            alice,
            Json(create_with_due(Some(due), Some(60), Some("!room:1"))),
        )
        .await
        .unwrap();

        assert_eq!(created.due_at.unwrap().timestamp(), due.timestamp());
        assert_eq!(created.remind_before_minutes, Some(60));

        let rem =
            crate::modules::reminder::repository::ReminderRepository::find_pending_for_source(
                &state.db.pool,
                ReminderSource::Todo,
                created.id,
            )
            .await
            .unwrap()
            .expect("a linked reminder must exist");
        assert_eq!(rem.trigger_at, due - chrono::Duration::minutes(60));
        assert_eq!(rem.room_id, "!room:1");
    }

    /// A lead time with nothing to count back from is a client bug, not a
    /// silently-ignored field.
    #[tokio::test]
    async fn test_lead_time_without_a_due_date_is_rejected() {
        let state = due_reminder_state().await;
        let err = create_todo(
            State(state),
            AuthUser::named("alice", "acct_alice"),
            Json(create_with_due(None, Some(60), Some("!room:1"))),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    /// Web-created tasks carry the `web` sentinel, so there is no room to
    /// deliver to unless the caller names one.
    #[tokio::test]
    async fn test_reminder_without_a_room_is_rejected() {
        let state = due_reminder_state().await;
        let err = create_todo(
            State(state),
            AuthUser::named("alice", "acct_alice"),
            Json(create_with_due(
                Some(crate::utils::time::now_at_storage_precision() + chrono::Duration::hours(6)),
                Some(60),
                None,
            )),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_lead_time_bounds_are_enforced() {
        let state = due_reminder_state().await;
        let due = Some(crate::utils::time::now_at_storage_precision() + chrono::Duration::hours(6));

        for bad in [0, -5, due_reminder::MAX_LEAD_MINUTES + 1] {
            let err = create_todo(
                State(state.clone()),
                AuthUser::named("alice", "acct_alice"),
                Json(create_with_due(due, Some(bad), Some("!room:1"))),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "lead {bad} must be rejected"
            );
        }
    }

    /// PATCHing `due_at: null` must clear the date *and* tear the reminder
    /// down. An absent field must leave both alone.
    #[tokio::test]
    async fn test_patch_null_clears_the_due_date_and_its_reminder() {
        let state = due_reminder_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        let due = crate::utils::time::now_at_storage_precision() + chrono::Duration::hours(6);

        let created = create_todo(
            State(state.clone()),
            alice.clone(),
            Json(create_with_due(Some(due), Some(60), Some("!room:1"))),
        )
        .await
        .unwrap();

        // An unrelated PATCH leaves the due date and reminder intact.
        let untouched = update_todo(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(UpdateTodoRequest {
                text: Some("Ship the release today".to_string()),
                priority: None,
                completed: None,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            }),
        )
        .await
        .unwrap();
        assert!(
            untouched.due_at.is_some(),
            "absent due_at must not clear it"
        );

        let cleared = update_todo(
            State(state.clone()),
            alice,
            Path(created.id),
            Json(UpdateTodoRequest {
                text: None,
                priority: None,
                completed: None,
                due_at: Some(None),
                remind_before_minutes: None,
                remind_room_id: None,
            }),
        )
        .await
        .unwrap();

        assert!(cleared.due_at.is_none());
        assert!(
            crate::modules::reminder::repository::ReminderRepository::find_pending_for_source(
                &state.db.pool,
                ReminderSource::Todo,
                created.id,
            )
            .await
            .unwrap()
            .is_none(),
            "clearing the due date must cancel its reminder"
        );
    }

    /// Editing an unrelated field must not require re-stating where the
    /// reminder goes — the caller isn't changing it.
    #[tokio::test]
    async fn test_editing_other_fields_keeps_the_reminder_and_its_room() {
        let state = due_reminder_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        let due = crate::utils::time::now_at_storage_precision() + chrono::Duration::hours(6);

        let created = create_todo(
            State(state.clone()),
            alice.clone(),
            Json(create_with_due(Some(due), Some(60), Some("!room:1"))),
        )
        .await
        .unwrap();

        let edited = update_todo(
            State(state.clone()),
            alice,
            Path(created.id),
            Json(UpdateTodoRequest {
                text: Some("Ship the release on Friday".to_string()),
                priority: Some("high".to_string()),
                completed: None,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(edited.text, "Ship the release on Friday");
        assert_eq!(edited.priority, "high");

        let rem = ReminderRepository::find_pending_for_source(
            &state.db.pool,
            ReminderSource::Todo,
            created.id,
        )
        .await
        .unwrap()
        .expect("the reminder must survive an unrelated edit");
        assert_eq!(
            rem.room_id, "!room:1",
            "the delivery room must be preserved"
        );
        assert_eq!(rem.trigger_at, due - chrono::Duration::minutes(60));
    }

    /// Changing the lead time re-aims the existing reminder without needing the
    /// room restated.
    #[tokio::test]
    async fn test_changing_the_lead_time_reschedules_in_place() {
        let state = due_reminder_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        let due = crate::utils::time::now_at_storage_precision() + chrono::Duration::days(2);

        let created = create_todo(
            State(state.clone()),
            alice.clone(),
            Json(create_with_due(Some(due), Some(60), Some("!room:1"))),
        )
        .await
        .unwrap();

        let _ = update_todo(
            State(state.clone()),
            alice,
            Path(created.id),
            Json(UpdateTodoRequest {
                text: None,
                priority: None,
                completed: None,
                due_at: None,
                remind_before_minutes: Some(Some(1440)),
                remind_room_id: None,
            }),
        )
        .await
        .unwrap();

        let rem = ReminderRepository::find_pending_for_source(
            &state.db.pool,
            ReminderSource::Todo,
            created.id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(rem.trigger_at, due - chrono::Duration::minutes(1440));
        assert_eq!(rem.room_id, "!room:1");
    }

    #[tokio::test]
    async fn test_deleting_a_task_cancels_its_reminder() {
        let state = due_reminder_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let created = create_todo(
            State(state.clone()),
            alice.clone(),
            Json(create_with_due(
                Some(crate::utils::time::now_at_storage_precision() + chrono::Duration::hours(6)),
                Some(60),
                Some("!room:1"),
            )),
        )
        .await
        .unwrap();

        delete_todo(State(state.clone()), alice, Path(created.id))
            .await
            .unwrap();

        assert!(
            crate::modules::reminder::repository::ReminderRepository::find_pending_for_source(
                &state.db.pool,
                ReminderSource::Todo,
                created.id,
            )
            .await
            .unwrap()
            .is_none(),
            "a deleted task must not leave a reminder pointing at it"
        );
    }
}
