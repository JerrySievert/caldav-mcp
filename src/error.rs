use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Application-level error type.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("precondition failed: {0}")]
    PreconditionFailed(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::PreconditionFailed(_) => StatusCode::PRECONDITION_FAILED,
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Log internal errors at error level
        match &self {
            AppError::Database(e) => tracing::error!("Database error: {e}"),
            AppError::Internal(e) => tracing::error!("Internal error: {e}"),
            _ => {}
        }

        (status, self.to_string()).into_response()
    }
}

/// Convenience type alias for handlers.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[test]
    fn test_not_found_maps_to_404() {
        let err = AppError::NotFound("thing".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_unauthorized_maps_to_401() {
        let err = AppError::Unauthorized;
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_forbidden_maps_to_403() {
        let err = AppError::Forbidden("no access".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_bad_request_maps_to_400() {
        let err = AppError::BadRequest("bad".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_conflict_maps_to_409() {
        let err = AppError::Conflict("conflict".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    }

    #[test]
    fn test_precondition_failed_maps_to_412() {
        let err = AppError::PreconditionFailed("etag mismatch".to_string());
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn test_database_error_maps_to_500() {
        let db_err = sqlx::Error::RowNotFound;
        let err = AppError::Database(db_err);
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_internal_error_maps_to_500() {
        let err = AppError::Internal(anyhow::anyhow!("internal failure"));
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_error_display_messages() {
        assert_eq!(AppError::Unauthorized.to_string(), "unauthorized");
        assert_eq!(
            AppError::NotFound("x".to_string()).to_string(),
            "not found: x"
        );
        assert_eq!(
            AppError::Forbidden("y".to_string()).to_string(),
            "forbidden: y"
        );
        assert_eq!(
            AppError::BadRequest("z".to_string()).to_string(),
            "bad request: z"
        );
        assert_eq!(
            AppError::Conflict("w".to_string()).to_string(),
            "conflict: w"
        );
        assert_eq!(
            AppError::PreconditionFailed("v".to_string()).to_string(),
            "precondition failed: v"
        );
    }
}
