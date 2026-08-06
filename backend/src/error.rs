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
    pub fn internal(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: msg.into() }
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
}
