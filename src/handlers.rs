use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::application::AppState;
use crate::error::AppError;

/// Response payload for `GET /sample`.
#[derive(Debug, Serialize)]
pub struct SampleResponse {
    pub message: String,
}

/// Sample endpoint — replace with your business logic.
pub async fn sample(State(_state): State<AppState>) -> Result<Json<SampleResponse>, AppError> {
    Ok(Json(SampleResponse {
        message: "Hello, world!".to_string(),
    }))
}
