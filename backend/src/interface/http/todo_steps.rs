//! Checklist-step endpoints. Every route returns the whole `TodoDetail` so the
//! client re-renders from one response without a refetch.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::domain::todos::steps::MAX_STEPS;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
use crate::interface::http::todos::TodoDetail;

#[derive(Deserialize, ToSchema)]
pub struct CreateStepRequest {
    /// Step text, at most 200 characters.
    pub text: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateStepRequest {
    pub text: Option<String>,
    pub done: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct ReorderStepsRequest {
    /// Every step id of the todo exactly once, in the wanted order (max 50).
    pub step_ids: Vec<i64>,
}

#[utoipa::path(
    post,
    path = "/api/v1/todos/{id}/steps",
    params(("id" = i64, Path, description = "Todo item ID")),
    request_body = CreateStepRequest,
    responses(
        (status = 200, description = "Step added", body = TodoDetail),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Todo not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn add_step(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<CreateStepRequest>,
) -> Result<Json<TodoDetail>, ApiError> {
    let detail = state
        .services
        .todos
        .add_step(&user.account_id, id, &payload.text)
        .await?;
    Ok(Json(detail.into()))
}

#[utoipa::path(
    patch,
    path = "/api/v1/todos/{id}/steps/{step_id}",
    params(
        ("id" = i64, Path, description = "Todo item ID"),
        ("step_id" = i64, Path, description = "Step ID")
    ),
    request_body = UpdateStepRequest,
    responses(
        (status = 200, description = "Step updated", body = TodoDetail),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Todo or step not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_step(
    State(state): State<ApiState>,
    user: AuthUser,
    Path((id, step_id)): Path<(i64, i64)>,
    Json(payload): Json<UpdateStepRequest>,
) -> Result<Json<TodoDetail>, ApiError> {
    let detail = state
        .services
        .todos
        .update_step(
            &user.account_id,
            id,
            step_id,
            payload.text.as_deref(),
            payload.done,
        )
        .await?;
    Ok(Json(detail.into()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/todos/{id}/steps/{step_id}",
    params(
        ("id" = i64, Path, description = "Todo item ID"),
        ("step_id" = i64, Path, description = "Step ID")
    ),
    responses(
        (status = 200, description = "Step removed", body = TodoDetail),
        (status = 404, description = "Todo or step not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn remove_step(
    State(state): State<ApiState>,
    user: AuthUser,
    Path((id, step_id)): Path<(i64, i64)>,
) -> Result<Json<TodoDetail>, ApiError> {
    let detail = state
        .services
        .todos
        .remove_step(&user.account_id, id, step_id)
        .await?;
    Ok(Json(detail.into()))
}

#[utoipa::path(
    put,
    path = "/api/v1/todos/{id}/steps/order",
    params(("id" = i64, Path, description = "Todo item ID")),
    request_body = ReorderStepsRequest,
    responses(
        (status = 200, description = "Steps reordered", body = TodoDetail),
        (status = 400, description = "Order does not list every step once", body = ApiErrorResponse),
        (status = 404, description = "Todo not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn reorder_steps(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<ReorderStepsRequest>,
) -> Result<Json<TodoDetail>, ApiError> {
    // Bound the body before it reaches the service's set comparison.
    if payload.step_ids.len() > MAX_STEPS {
        return Err(ApiError::bad_request(format!(
            "A todo can have at most {MAX_STEPS} steps"
        )));
    }
    let detail = state
        .services
        .todos
        .reorder_steps(&user.account_id, id, &payload.step_ids)
        .await?;
    Ok(Json(detail.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    use crate::interface::http::todos::{create_todo, CreateTodoRequest};

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice", "acct_bob"]).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    async fn new_todo(state: &ApiState, user: &AuthUser) -> i64 {
        create_todo(
            State(state.clone()),
            user.clone(),
            Json(CreateTodoRequest {
                room_id: None,
                text: "todo".to_string(),
                priority: None,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
                tags: None,
                recurrence: None,
            }),
        )
        .await
        .unwrap()
        .id
    }

    fn add(text: &str) -> Json<CreateStepRequest> {
        Json(CreateStepRequest {
            text: text.to_string(),
        })
    }

    #[tokio::test]
    async fn step_lifecycle_returns_detail_each_time() {
        let state = state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let id = new_todo(&state, &alice).await;

        let _ = add_step(State(state.clone()), alice.clone(), Path(id), add("a"))
            .await
            .unwrap();
        let two = add_step(State(state.clone()), alice.clone(), Path(id), add("b"))
            .await
            .unwrap();
        let (a, b) = (two.steps[0].id, two.steps[1].id);

        let done = update_step(
            State(state.clone()),
            alice.clone(),
            Path((id, a)),
            Json(UpdateStepRequest {
                text: Some("a2".to_string()),
                done: Some(true),
            }),
        )
        .await
        .unwrap();
        assert_eq!((done.item.steps_done, done.item.steps_total), (1, 2));
        assert_eq!(done.steps[0].text, "a2");

        let reordered = reorder_steps(
            State(state.clone()),
            alice.clone(),
            Path(id),
            Json(ReorderStepsRequest {
                step_ids: vec![b, a],
            }),
        )
        .await
        .unwrap();
        assert_eq!(reordered.steps[0].id, b);

        let removed = remove_step(State(state.clone()), alice.clone(), Path((id, b)))
            .await
            .unwrap();
        assert_eq!(removed.steps.len(), 1);
        assert_eq!(removed.steps[0].position, 1);

        let err = reorder_steps(
            State(state),
            alice,
            Path(id),
            Json(ReorderStepsRequest { step_ids: vec![b] }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn step_endpoints_404_for_other_owner() {
        let state = state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let bob = AuthUser::named_seed("bob", "acct_bob");
        let id = new_todo(&state, &alice).await;
        let step = add_step(State(state.clone()), alice.clone(), Path(id), add("mine"))
            .await
            .unwrap()
            .steps[0]
            .id;

        let status = |e: ApiError| e.status;
        let s = state.clone();
        assert_eq!(
            status(
                add_step(State(s.clone()), bob.clone(), Path(id), add("x"))
                    .await
                    .unwrap_err()
            ),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(
                update_step(
                    State(s.clone()),
                    bob.clone(),
                    Path((id, step)),
                    Json(UpdateStepRequest {
                        text: None,
                        done: Some(true)
                    })
                )
                .await
                .unwrap_err()
            ),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(
                remove_step(State(s.clone()), bob.clone(), Path((id, step)))
                    .await
                    .unwrap_err()
            ),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(
                reorder_steps(
                    State(s),
                    bob,
                    Path(id),
                    Json(ReorderStepsRequest {
                        step_ids: vec![step]
                    })
                )
                .await
                .unwrap_err()
            ),
            StatusCode::NOT_FOUND
        );

        // A missing step on the caller's own todo is also a 404.
        let err = remove_step(State(state), alice, Path((id, step + 1000)))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn step_text_over_200_is_400() {
        let state = state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let id = new_todo(&state, &alice).await;

        let err = add_step(
            State(state.clone()),
            alice.clone(),
            Path(id),
            add(&"x".repeat(201)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let ok = add_step(
            State(state.clone()),
            alice.clone(),
            Path(id),
            add(&"x".repeat(200)),
        )
        .await
        .unwrap();
        let err = update_step(
            State(state.clone()),
            alice.clone(),
            Path((id, ok.steps[0].id)),
            Json(UpdateStepRequest {
                text: Some("y".repeat(201)),
                done: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let err = reorder_steps(
            State(state),
            alice,
            Path(id),
            Json(ReorderStepsRequest {
                step_ids: (0..=MAX_STEPS as i64).collect(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }
}

/// Wire-level checks through the real router: path precedence between the
/// static `tags` / `order` segments and their `{id}` / `{step_id}` siblings.
#[cfg(test)]
mod router_tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use axum::Router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::interface::http::auth::types::ApiState;
    use crate::interface::http::routes::api_routes;

    async fn call(
        app: &Router,
        cookie: &str,
        method: &str,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, format!("mb_session={cookie}"))
            .header(header::ORIGIN, "http://localhost:8080")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn static_segments_win_over_id_params() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        );
        let accounts = &state.services.accounts;
        let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
        let cookie = accounts
            .create_session(&account, None, None, None)
            .await
            .unwrap();
        let app = Router::new()
            .nest("/api/v1", api_routes())
            .with_state(state);

        // `tags` must not be parsed as a todo id (that would be a 400/404).
        let (status, body) = call(&app, &cookie, "GET", "/api/v1/todos/tags", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["tags"], json!([]));

        let (status, todo) = call(
            &app,
            &cookie,
            "POST",
            "/api/v1/todos",
            json!({"text": "t", "tags": ["a"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{todo}");
        let id = todo["id"].as_i64().unwrap();

        let (status, detail) = call(
            &app,
            &cookie,
            "POST",
            &format!("/api/v1/todos/{id}/steps"),
            json!({"text": "s1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        let step_id = detail["steps"][0]["id"].as_i64().unwrap();

        // `order` must reach the reorder handler, not parse as a step id.
        let (status, detail) = call(
            &app,
            &cookie,
            "PUT",
            &format!("/api/v1/todos/{id}/steps/order"),
            json!({"step_ids": [step_id]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail}");

        let (status, detail) = call(
            &app,
            &cookie,
            "GET",
            &format!("/api/v1/todos/{id}"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["item"]["tags"], json!(["a"]));
        assert_eq!(detail["item"]["steps_total"], 1);
    }
}
