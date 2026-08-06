use axum::{response::IntoResponse, Json};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

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
        auth::create_sse_ticket,
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
            auth::SseTicketResponse,
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
    modifiers(&SecurityAddon),
    tags(
        (name = "Matrix Bot REST API", description = "REST API & Real-time Web interface endpoints")
    )
)]
pub struct ApiDoc;

/// Defines the `bearer_auth` scheme every secured path references.
///
/// Without this the generated document carries security requirements pointing
/// at a scheme that was never declared, which generated clients cannot wire up.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);

        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "A local API JWT, or a Matrix access token. The SSE endpoint \
                         additionally accepts the same value as a `token` query parameter, \
                         because EventSource cannot set request headers.",
                    ))
                    .build(),
            ),
        );
    }
}

pub async fn openapi_json() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document must build and declare every scheme its paths reference.
    ///
    /// Writing `openapi.json` is the job of the `generate-openapi` binary, not of
    /// this test — a test that writes files drifts silently when the suite runs
    /// from a different directory.
    #[test]
    fn test_openapi_document_is_well_formed() {
        let doc = ApiDoc::openapi();
        let json = doc.to_pretty_json().unwrap();
        assert!(!json.is_empty());

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let schemes = &parsed["components"]["securitySchemes"];
        assert!(
            schemes["bearer_auth"].is_object(),
            "paths reference `bearer_auth`, so it must be declared"
        );
    }
}
