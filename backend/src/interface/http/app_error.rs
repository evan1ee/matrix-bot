//! What an application error means over HTTP.
//!
//! One mapping for every handler, so a service returning `AppError` answers
//! with the same status and body the hand-written `ApiError` constructors
//! did. The two diagnostic variants become an opaque 500 whose detail goes to
//! the log only (`ApiError::internal`). A handler that has always answered an
//! external failure with another status maps that case itself before `?`.

use crate::application::AppError;
use crate::error::ApiError;

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Validation(msg) => ApiError::bad_request(msg),
            AppError::NotFound(msg) => ApiError::not_found(msg),
            AppError::Forbidden(msg) => ApiError::forbidden(msg),
            AppError::Conflict(msg) => ApiError::conflict(msg),
            AppError::Database(err) => ApiError::internal(err.to_string()),
            AppError::External(detail) => ApiError::internal(detail),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn mapped(err: AppError) -> (StatusCode, String) {
        let api = ApiError::from(err);
        (api.status, api.message)
    }

    #[test]
    fn user_facing_variants_keep_their_message() {
        let msg = || "Webhook not found".to_string();
        assert_eq!(
            mapped(AppError::Validation(msg())),
            (StatusCode::BAD_REQUEST, msg())
        );
        assert_eq!(
            mapped(AppError::NotFound(msg())),
            (StatusCode::NOT_FOUND, msg())
        );
        assert_eq!(
            mapped(AppError::Forbidden(msg())),
            (StatusCode::FORBIDDEN, msg())
        );
        assert_eq!(
            mapped(AppError::Conflict(msg())),
            (StatusCode::CONFLICT, msg())
        );
    }

    #[test]
    fn diagnostic_variants_never_reach_the_client() {
        let internal = (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".to_string(),
        );
        assert_eq!(
            mapped(AppError::Database(sqlx::Error::RowNotFound)),
            internal
        );
        assert_eq!(
            mapped(AppError::External("ocr at 10.0.0.5 timed out".into())),
            internal
        );
    }
}
