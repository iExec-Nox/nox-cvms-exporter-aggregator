use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::aggregation::merge_cvms;
use crate::application::AppState;
use crate::error::AppError;
use crate::types::{
    CvmInstance, CvmInstanceForUI, CvmSummary, CvmSummaryForUI, ExporterInfo, QuoteResponse,
};

/// Query parameters accepted by `GET /cvms`.
#[derive(Debug, Deserialize)]
pub struct CvmsQuery {
    /// Verifier-generated nonce, relayed to each CVM's `/quote` endpoint so the
    /// returned quote is bound to it (anti-replay / freshness guarantee).
    pub challenge: Option<String>,
}

/// Root endpoint handler.
///
/// Returns basic service information including the service name and current timestamp.
/// This endpoint is typically used for service discovery and basic connectivity checks.
///
/// # Returns
///
/// JSON response containing:
/// - `service`: The service name ("nox-cvms-exporter-aggregator")
/// - `timestamp`: Current UTC timestamp in RFC3339 format
pub async fn root() -> Json<Value> {
    Json(json!({
        "service": "nox-cvms-exporter-aggregator",
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

/// Queries a single `nox-cvms-exporter` instance on its `/cvms` endpoint.
///
/// Returns the exporter's per-machine CVM groups on success, or a human-readable
/// error string (prefixed with the exporter URL) so the caller can isolate a
/// single unreachable/failing exporter without aborting the whole aggregation.
async fn fetch_exporter_cvms(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<CvmSummary>, String> {
    let url = format!("{}/cvms", base_url.trim_end_matches('/'));

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("{base_url}: failed to reach exporter: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "{base_url}: exporter returned status {}",
            response.status()
        ));
    }

    response
        .json::<Vec<CvmSummary>>()
        .await
        .map_err(|e| format!("{base_url}: failed to parse exporter response: {e}"))
}

/// Fetches a CVM's `/quote`, binding it to the verifier's `challenge`.
///
/// The challenge is passed as the `data` query parameter and ends up in the
/// quote's `report_data`, letting the UI prove the quote is fresh. Returns a
/// URL-prefixed error string so the caller can drop a single failing instance.
async fn fetch_quote(
    client: &reqwest::Client,
    base_url: &str,
    challenge: &str,
) -> Result<QuoteResponse, String> {
    let mut url = reqwest::Url::parse(&format!("{}/quote", base_url.trim_end_matches('/')))
        .map_err(|e| format!("{base_url}: invalid quote url: {e}"))?;
    url.query_pairs_mut().append_pair("data", challenge);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{base_url}: failed to reach quote endpoint: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "{base_url}: quote endpoint returned status {}",
            response.status()
        ));
    }

    response
        .json::<QuoteResponse>()
        .await
        .map_err(|e| format!("{base_url}: failed to parse quote response: {e}"))
}

/// Fetches a CVM's `/info` and extracts the docker-compose manifest.
///
/// Only `tcb_info.app_compose` is needed by the UI (compose-hash check); every
/// other field is ignored. Returns a URL-prefixed error string on failure.
async fn fetch_app_info(client: &reqwest::Client, base_url: &str) -> Result<String, String> {
    let url = format!("{}/info", base_url.trim_end_matches('/'));

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("{base_url}: failed to reach info endpoint: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "{base_url}: info endpoint returned status {}",
            response.status()
        ));
    }

    response
        .json::<ExporterInfo>()
        .await
        .map(|info| info.tcb_info.app_compose)
        .map_err(|e| format!("{base_url}: failed to parse info response: {e}"))
}

/// Enriches a single instance with its quote and compose manifest, fetched
/// concurrently. Returns `None` (and logs) if either fetch fails, so one
/// unreachable CVM is dropped rather than aborting the whole response.
async fn enrich_instance(
    client: &reqwest::Client,
    challenge: &str,
    instance: CvmInstance,
) -> Option<CvmInstanceForUI> {
    let (quote, app_compose) = tokio::join!(
        fetch_quote(client, &instance.url, challenge),
        fetch_app_info(client, &instance.url),
    );

    match (quote, app_compose) {
        (Ok(quote), Ok(app_compose)) => Some(CvmInstanceForUI {
            instance_id: instance.instance_id,
            machine_id: instance.machine_id,
            quote,
            app_compose,
        }),
        (quote, app_compose) => {
            if let Err(e) = quote {
                warn!("dropping instance {}: {e}", instance.instance_id);
            }
            if let Err(e) = app_compose {
                warn!("dropping instance {}: {e}", instance.instance_id);
            }
            None
        }
    }
}

/// `GET /cvms?challenge=<nonce>` — returns active CVMs across all configured
/// exporters, grouped by app, with each instance's quote and compose manifest
/// embedded so the UI never contacts the CVMs directly.
///
/// Queries every configured exporter's `/cvms` endpoint in parallel, merges the
/// per-machine groups by `app_id`, then fetches each instance's `/quote` (bound
/// to the caller's `challenge`) and `/info` concurrently. Unreachable or failing
/// exporters are skipped so a single faulty machine does not abort the whole
/// aggregation; the request only fails if *every* exporter fails. Instances whose
/// quote/info fetch fails are dropped from the response (logged).
///
/// Requires a non-empty `challenge` query parameter; returns `400` otherwise.
pub async fn get_active_cvms(
    State(state): State<AppState>,
    Query(query): Query<CvmsQuery>,
) -> Result<Json<Vec<CvmSummaryForUI>>, AppError> {
    // 0. A challenge (verifier nonce) is mandatory: it is relayed to each CVM's
    //    /quote endpoint so the returned quote is bound to the UI's nonce.
    let challenge = query
        .challenge
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            AppError::BadRequest("missing required query parameter: challenge".to_owned())
        })?;

    // 1. Query every exporter concurrently — we need all responses, not the first.
    let futures = state
        .config
        .exporters
        .iter()
        .map(|base_url| fetch_exporter_cvms(&state.http_client, base_url));
    let results = futures::future::join_all(futures).await;

    // 2. Split successes from failures, isolating per-exporter errors.
    let mut summaries = Vec::new();
    let mut failures = 0;

    for result in results {
        match result {
            Ok(exporter_summaries) => summaries.extend(exporter_summaries),
            Err(e) => {
                failures += 1;
                warn!("skipping exporter: {e}");
            }
        }
    }

    // 3. Fail only when no exporter answered at all.
    // The `failures > 0` guard is intentional: if `exporters` is empty,
    // `failures == exporters.len()` would be `0 == 0` (true) and incorrectly
    // return an error. Do not remove it as "redundant".
    if failures > 0 && failures == state.config.exporters.len() {
        return Err(AppError::Internal(
            "all configured exporters failed".to_owned(),
        ));
    }

    // 4. Merge the per-machine groups into a single list keyed by `app_id`.
    let merged = merge_cvms(summaries);

    // 5. Enrich every instance with its quote (bound to `challenge`) and compose
    //    manifest, fetched concurrently across all apps and instances. Instances
    //    whose fetch fails are dropped so one unreachable CVM does not abort the
    //    whole response.
    let client = &state.http_client;
    let challenge = &challenge;
    let ui_summaries =
        futures::future::join_all(merged.into_iter().map(|summary| async move {
            let instances = futures::future::join_all(
                summary
                    .instances
                    .into_iter()
                    .map(|instance| enrich_instance(client, challenge, instance)),
            )
            .await;

            CvmSummaryForUI {
                app_id: summary.app_id,
                name: summary.name,
                instances: instances.into_iter().flatten().collect(),
            }
        }))
        .await;

    Ok(Json(ui_summaries))
}
