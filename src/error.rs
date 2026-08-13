use axum::extract::rejection::JsonRejection;
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
pub(crate) enum AppError {
    /// 500 — unclassified internal failure.
    #[error("Internal error: {0}")]
    Internal(String),
    /// 500 — JSON (de)serialization failure, converted via `?`.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// 4xx — request body could not be extracted or failed type-level validation
    /// (from a `JsonRejection`). Carries the status axum inferred (`400` invalid
    /// JSON, `422` well-formed JSON that fails validation, `415` wrong content
    /// type) so the service still answers with its JSON error envelope instead of
    /// axum's default plain-text response.
    #[error("{message}")]
    InvalidBody { status: StatusCode, message: String },
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        AppError::InvalidBody {
            status: rejection.status(),
            message: rejection.body_text(),
        }
    }
}

impl AppError {
    fn error_code(&self) -> &'static str {
        match self {
            AppError::Internal(_) => "internal",
            AppError::Serialization(_) => "serialization",
            AppError::InvalidBody { .. } => "invalid_body",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::InvalidBody { status, .. } => *status,
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
