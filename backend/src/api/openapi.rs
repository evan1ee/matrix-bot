use axum::{response::IntoResponse, Json};
use utoipa::OpenApi;

use crate::api::{auth, health, reminders, rooms, sse, todos};
use crate::error::ApiErrorResponse;
use crate::modules::reminder::models::{RecurrencePattern, Reminder, ReminderStatus};
use crate::modules::todo::models::{TodoItem, TodoPriority};

#[derive(OpenApi)]
#[openapi(
    paths(
        health::health_check,
        sse::sse_handler,
        auth::register,
        auth::login,
        auth::get_current_user,
        auth::media_proxy,
        rooms::list_rooms,
        todos::list_todos,
        todos::create_todo,
        todos::update_todo,
        todos::delete_todo,
        reminders::list_reminders,
        reminders::create_reminder,
        reminders::delete_reminder
    ),
    components(
        schemas(
            ApiErrorResponse,
            health::HealthResponse,
            auth::AuthPayload,
            auth::RegisterPayload,
            auth::AuthResponse,
            auth::UserProfileResponse,
            auth::MediaProxyQuery,
            rooms::JoinedRoomInfo,
            TodoItem,
            TodoPriority,
            todos::TodoQuery,
            todos::CreateTodoRequest,
            todos::UpdateTodoRequest,
            todos::TodoListResponse,
            Reminder,
            ReminderStatus,
            RecurrencePattern,
            reminders::ReminderQuery,
            reminders::CreateReminderRequest,
            reminders::ReminderListResponse
        )
    ),
    tags(
        (name = "Matrix Bot REST API", description = "REST API & Real-time Web interface endpoints")
    )
)]
pub struct ApiDoc;

pub async fn openapi_json() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_openapi_file() {
        let doc = ApiDoc::openapi();
        let json = doc.to_pretty_json().unwrap();
        assert!(!json.is_empty());
        let _ = std::fs::write("openapi.json", &json);
    }
}
