use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

/// Centralised error type returned by every HTTP handler.
///
/// Variants wrapping a foreign error (`#[from]`) let the `?` operator convert
/// automatically — prefer adding a new `#[from]` variant over calling `map_err`.
#[derive(Debug, Error)]
pub enum AppError {
    /// 400 — request payload or parameters are invalid.
    #[error("Bad request: {0}")]
    BadRequest(String),
    /// 401 — caller is not authenticated.
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    /// 403 — caller is authenticated but not allowed.
    #[error("Forbidden: {0}")]
    Forbidden(String),
    /// 404 — target resource does not exist.
    #[error("Not found: {0}")]
    NotFound(String),
    /// 409 — request conflicts with the current state of the resource.
    #[error("Conflict: {0}")]
    Conflict(String),
    /// 500 — unclassified internal failure.
    #[error("Internal error: {0}")]
    Internal(String),
    /// 500 — JSON (de)serialization failure, converted via `?`.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// 500 — underlying I/O failure, converted via `?`.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl AppError {
    fn error_code(&self) -> &'static str {
        match self {
            AppError::BadRequest(_) => "bad_request",
            AppError::Unauthorized(_) => "unauthorized",
            AppError::Forbidden(_) => "forbidden",
            AppError::NotFound(_) => "not_found",
            AppError::Conflict(_) => "conflict",
            AppError::Internal(_) => "internal",
            AppError::Serialization(_) => "serialization",
            AppError::Io(_) => "io",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(json!({
            "error": self.error_code(),
            "message": self.to_string()
        }));
        (status, body).into_response()
    }
}
