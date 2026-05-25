use axum::Json;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use chrono::Utc;
use serde_json::{Value, json};

/// Root endpoint handler.
///
/// Returns basic service information including the service name and current timestamp.
/// This endpoint is typically used for service discovery and basic connectivity checks.
///
/// # Returns
///
/// JSON response containing:
/// - `service`: The service name ("nox-kms")
/// - `timestamp`: Current UTC timestamp in RFC3339 format
pub async fn root() -> Json<Value> {
    Json(json!({
        "service": "nox-kms",
        "timestamp": Utc::now().to_rfc3339()
    }))
}

/// Health check endpoint handler.
///
/// Returns a simple "OK" response to indicate that the service is running.
/// This endpoint is typically used for health checks and service monitoring.
///
/// # Returns
///
/// JSON response containing:
/// - `status`: The status of the service ("ok")
pub async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Fallback handler for non-existing routes.
///
/// Returns 404 NOT_FOUND to indicate the requested route does not exist.
pub async fn not_found(uri: Uri) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error":format!("Route not found {}", uri.path()) })),
    )
}
