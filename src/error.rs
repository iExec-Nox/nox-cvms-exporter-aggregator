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
    /// 404 — target resource does not exist.
    #[error("Not found: {0}")]
    NotFound(String),
    /// 500 — unclassified internal failure.
    #[error("Internal error: {0}")]
    Internal(String),
    /// 500 — JSON (de)serialization failure, converted via `?`.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl AppError {
    fn error_code(&self) -> &'static str {
        match self {
            AppError::NotFound(_) => "not_found",
            AppError::Internal(_) => "internal",
            AppError::Serialization(_) => "serialization",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
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
