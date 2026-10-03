//! What an application error means over HTTP.
//!
//! One mapping for every handler, so a service returning `AppError` answers
//! with the same status and body the hand-written `ApiError` constructors
//! did. The two diagnostic variants become an opaque 500 whose detail goes to
//! the log only (`ApiError::internal`). A handler that has always answered an
//! external failure with another status matches `AppError::External
//! { service, .. }` itself before `?`.

use crate::application::AppError;
use crate::interface::http::error::ApiError;

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        to_api_error(err, None)
    }
}

/// `.api_context("Failed to …")?` in a handler: the same mapping as `?`, with
/// what the handler was doing prefixed to the logged detail of a diagnostic
/// variant. The client still sees only the opaque 500.
pub trait AppResultExt<T> {
    fn api_context(self, doing: &str) -> Result<T, ApiError>;
}

impl<T> AppResultExt<T> for Result<T, AppError> {
    fn api_context(self, doing: &str) -> Result<T, ApiError> {
        self.map_err(|err| to_api_error(err, Some(doing)))
    }
}

fn to_api_error(err: AppError, doing: Option<&str>) -> ApiError {
    let logged = |detail: String| match doing {
        Some(doing) => format!("{doing}: {detail}"),
        None => detail,
    };
    match err {
        AppError::Validation(msg) => ApiError::bad_request(msg),
        AppError::NotFound(msg) => ApiError::not_found(msg),
        AppError::Forbidden(msg) => ApiError::forbidden(msg),
        AppError::Conflict(msg) => ApiError::conflict(msg),
        AppError::Database(err) => ApiError::internal(logged(err.to_string())),
        AppError::External { service, source } => {
            ApiError::internal(logged(format!("{service}: {source:#}")))
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
            mapped(AppError::External {
                service: "ocr",
                source: anyhow::anyhow!("10.0.0.5 timed out"),
            }),
            internal
        );
    }

    #[test]
    fn context_does_not_change_what_the_client_sees() {
        let result: Result<(), AppError> = Err(AppError::Database(sqlx::Error::RowNotFound));
        let err = result
            .api_context("Failed to save cached rooms")
            .unwrap_err();
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.message, "Internal server error");

        let result: Result<(), AppError> = Err(AppError::NotFound("gone".into()));
        let err = result
            .api_context("Failed to save cached rooms")
            .unwrap_err();
        assert_eq!(
            (err.status, err.message.as_str()),
            (StatusCode::NOT_FOUND, "gone")
        );
    }
}
