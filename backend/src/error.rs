use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiErrorResponse {
    pub error: String,
    pub code: u16,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: msg.into() }
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, message: msg.into() }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: msg.into() }
    }
    /// Records the underlying failure and returns an opaque 500.
    ///
    /// Call sites pass raw `sqlx`/IO errors here. Those name tables, columns and
    /// file paths, so they belong in the log, not in a response body.
    pub fn internal(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        tracing::error!("Internal API error: {}", detail);
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal server error".to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorResponse {
            error: self.message,
            code: self.status.as_u16(),
        };
        IntoResponse::into_response((self.status, Json(body)))
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::internal(err.to_string())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::internal(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_constructors() {
        let err = ApiError::bad_request("invalid param");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.message, "invalid param");

        let err = ApiError::unauthorized("no token");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);

        let err = ApiError::not_found("item missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        let err = ApiError::internal("db crash");
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Internal failures must not describe themselves to the caller.
    #[test]
    fn test_internal_errors_do_not_leak_detail() {
        let err = ApiError::internal(
            "error returned from database: no such column: todos.secret_column",
        );
        assert_eq!(err.message, "Internal server error");
        assert!(!err.message.contains("todos"));

        let err = ApiError::from(sqlx::Error::RowNotFound);
        assert_eq!(err.message, "Internal server error");
    }
}
