use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};

use crate::error::AppError;

/// A JSON request-body extractor whose failures surface as [`AppError`] — the
/// service's JSON error envelope with the rejection's status code — instead of
/// axum's default plain-text `Json` rejection.
///
/// Behaves exactly like `axum::Json<T>` on success. On failure it converts the
/// [`JsonRejection`] into [`AppError`], preserving the status axum inferred:
/// `400` for invalid JSON, `422` for well-formed JSON that fails a type's own
/// `Deserialize` validation, and `415` for the wrong content type — so every
/// error the service returns shares one format.
#[derive(Debug)]
pub(crate) struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await?;
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::response::IntoResponse;

    use crate::types::AttestationRequest;

    #[tokio::test]
    async fn rejection_is_returned_as_a_json_envelope() {
        // 8-char challenge fails the 64-byte check → 422 as our JSON envelope.
        let req = HttpRequest::builder()
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"challenge":"tooshort","instances":[]}"#))
            .unwrap();

        let response = JsonBody::<AttestationRequest>::from_request(req, &())
            .await
            .expect_err("an 8-char challenge must be rejected")
            .into_response();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("application/json"),
            "body errors must use the JSON envelope, got {content_type:?}"
        );
    }
}
