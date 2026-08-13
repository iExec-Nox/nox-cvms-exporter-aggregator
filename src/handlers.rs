use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use chrono::Utc;
use futures::stream::{self, StreamExt};
use serde_json::{Value, json};
use tracing::warn;

use crate::aggregation::merge_cvms;
use crate::application::AppState;
use crate::error::AppError;
use crate::types::{
    AttestationRequest, CvmInstance, CvmSummary, EnrichedCvmInstance, EnrichedCvmSummary,
    ExporterInfo, QuoteResponse,
};

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

/// Rebuilds a CVM's base URL from its instance id and the per-machine routing
/// config: `https://<instance_id>-<quote_service_port>.<suffix_url>`.
///
/// The exporter no longer exposes the URL; the aggregator owns URL construction,
/// keeping the internal CVM address out of both the exporter response and the UI.
fn build_cvm_url(instance_id: &str, quote_service_port: u16, suffix_url: &str) -> String {
    format!("https://{instance_id}-{quote_service_port}.{suffix_url}")
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
    base_url: &str,
    instance: CvmInstance,
) -> Option<EnrichedCvmInstance> {
    let (quote, app_compose) = tokio::join!(
        fetch_quote(client, base_url, challenge),
        fetch_app_info(client, base_url),
    );

    match (quote, app_compose) {
        (Ok(quote), Ok(app_compose)) => Some(EnrichedCvmInstance {
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

/// Enriches a resolved work list — `(app_id, name, instance, base_url)` — with a
/// bounded number of concurrent fetches (`max_inflight`), then regroups the
/// survivors by `app_id` (which is also the cross-exporter merge). Instances
/// whose quote/info fetch fails are dropped (logged) inside `enrich_instance`.
///
/// Used by `POST /cvms/attestations` once the caller's targets are resolved to
/// `(app_id, name, instance, base_url)` tuples.
async fn enrich_and_group(
    client: &reqwest::Client,
    challenge: &str,
    max_inflight: usize,
    resolved: Vec<(String, String, CvmInstance, String)>,
) -> Vec<EnrichedCvmSummary> {
    let enriched: Vec<(String, String, EnrichedCvmInstance)> = stream::iter(resolved)
        .map(|(app_id, name, instance, url)| async move {
            enrich_instance(client, challenge, &url, instance)
                .await
                .map(|ui| (app_id, name, ui))
        })
        .buffer_unordered(max_inflight)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect();

    merge_cvms(
        enriched
            .into_iter()
            .map(|(app_id, name, ui)| EnrichedCvmSummary {
                app_id,
                name,
                instances: vec![ui],
            }),
    )
}

/// Discovers every active instance across all configured exporters.
///
/// Queries each exporter's `/cvms` endpoint concurrently and flattens the result
/// into a flat work list of `(app_id, name, instance)` tuples. Unreachable or
/// failing exporters are skipped (logged) so a single faulty machine does not
/// abort discovery; the call only fails when *every* configured exporter fails.
///
/// # Note
///
/// The `failures > 0` guard is load-bearing: with an empty `exporters` list,
/// `failures == exporters.len()` would be `0 == 0` and wrongly report an error.
/// Do not drop it as redundant.
async fn discover_instances(
    state: &AppState,
) -> Result<Vec<(String, String, CvmInstance)>, AppError> {
    // 1. Query every exporter concurrently — we need all responses, not the first.
    let futures = state
        .config
        .exporters
        .iter()
        .map(|base_url| fetch_exporter_cvms(&state.http_client, base_url.as_str()));
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

    // 3. Fail only when no exporter answered at all (guard is load-bearing — see doc).
    if failures > 0 && failures == state.config.exporters.len() {
        return Err(AppError::Internal(
            "all configured exporters failed".to_owned(),
        ));
    }

    // 4. Flatten every instance, carrying its app_id/name.
    let flat = summaries
        .into_iter()
        .flat_map(|summary| {
            let app_id = summary.app_id;
            let name = summary.name;
            summary
                .instances
                .into_iter()
                .map(move |instance| (app_id.clone(), name.clone(), instance))
        })
        .collect();

    Ok(flat)
}

/// `GET /cvms` — lists active CVMs across all configured exporters, grouped by
/// `app_id`.
///
/// This is a lightweight discovery view: each instance carries only its
/// `instance_id` and `machine_id`, with **no** attestation data — so the initial
/// page load stays small. The UI fetches quotes and compose manifests on demand
/// via `POST /cvms/attestations`.
///
/// Unreachable or failing exporters are skipped so a single faulty machine does
/// not abort the listing; the request only fails if *every* exporter fails.
pub async fn get_active_cvms(
    State(state): State<AppState>,
) -> Result<Json<Vec<CvmSummary>>, AppError> {
    // 1. Discover every active instance across all exporters.
    let discovered = discover_instances(&state).await?;

    // 2. Regroup the flat work list by `app_id` (also the cross-exporter merge).
    let listing = merge_cvms(
        discovered
            .into_iter()
            .map(|(app_id, name, instance)| CvmSummary {
                app_id,
                name,
                instances: vec![instance],
            }),
    );

    Ok(Json(listing))
}

/// `POST /cvms/attestations` — enriches a caller-selected set of instances with
/// their quote and compose manifest, **without** contacting the exporters.
///
/// The UI echoes back instances from a prior `GET /cvms` listing (each carrying
/// its `instance_id` + `machine_id`) together with a **fresh** `challenge`. For
/// every target the aggregator rebuilds the CVM base URL from the machine's
/// configured URL suffix, fetches `/quote?data=<challenge>` and `/info`
/// concurrently, and returns the results grouped by `app_id`.
///
/// The granularity — one instance, all instances of an app, or everything — is
/// entirely the caller's: it is just how many targets are sent. Targets whose
/// `machine_id` is absent from the routing config (so cannot be addressed inside
/// a trusted domain), or whose fetch fails, are dropped (logged).
///
/// The `challenge` is mandatory and validated as exactly 64 bytes at
/// deserialization (see `Challenge`); a missing or ill-sized value is rejected
/// with `422` before this handler runs.
pub async fn post_attestations(
    State(state): State<AppState>,
    Json(request): Json<AttestationRequest>,
) -> Result<Json<Vec<EnrichedCvmSummary>>, AppError> {
    // 0. The challenge is already validated by its type — relay it as-is.
    let challenge = request.challenge;

    // 1. Resolve each target's base URL; drop targets whose machine_id isn't
    //    configured (see doc).
    let quote_service_port = state.config.quote_service_port;
    // Parsed once at startup and cached in state — no per-request re-parsing.
    let machine_suffixes = &state.machine_suffixes;
    let resolved: Vec<(String, String, CvmInstance, String)> = request
        .instances
        .into_iter()
        .filter_map(
            |target| match machine_suffixes.get(target.machine_id.as_str()) {
                Some(suffix) => {
                    let url =
                        build_cvm_url(target.instance_id.as_str(), quote_service_port, suffix);
                    let instance = CvmInstance {
                        instance_id: target.instance_id.into_inner(),
                        machine_id: target.machine_id,
                    };
                    Some((target.app_id.into_inner(), target.name, instance, url))
                }
                None => {
                    warn!(
                        "dropping target {}: no url suffix configured for machine_id {}",
                        target.instance_id.as_str(),
                        target.machine_id
                    );
                    None
                }
            },
        )
        .collect();

    // 2. Enrich (bounded concurrency) and regroup by `app_id`.
    let ui_summaries = enrich_and_group(
        &state.http_client,
        challenge.as_str(),
        state.config.max_inflight,
        resolved,
    )
    .await;

    Ok(Json(ui_summaries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A minimal but realistic `/quote` payload (field shapes match a real CVM).
    fn quote_body() -> Value {
        json!({
            "quote": "0400020081000000deadbeef",
            "event_log": "[{\"imr\":0}]",
            "rtmrs": "{0: \"aa\"}",
            "vm_config": "{\"cpu_count\":2}"
        })
    }

    /// A `/info` payload shaped like a real exporter response: the manifest is
    /// nested under `tcb_info.app_compose`, surrounded by fields we ignore.
    fn info_body(app_compose: &str) -> Value {
        json!({
            "app_id": "8327d735",
            "app_name": "nox-kms",
            "compose_hash": "ad08c205",
            "os_image_hash": "bd369a8c",
            "vm_config": "{\"cpu_count\":2}",
            "tcb_info": {
                "app_compose": app_compose,
                "compose_hash": "ad08c205",
                "os_image_hash": "bd369a8c"
            }
        })
    }

    async fn mount_quote(server: &MockServer, challenge: &str) {
        Mock::given(method("GET"))
            .and(path("/quote"))
            .and(query_param("data", challenge))
            .respond_with(ResponseTemplate::new(200).set_body_json(quote_body()))
            .mount(server)
            .await;
    }

    async fn mount_info(server: &MockServer, app_compose: &str) {
        Mock::given(method("GET"))
            .and(path("/info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(info_body(app_compose)))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn fetch_app_info_extracts_nested_app_compose() {
        let server = MockServer::start().await;
        mount_info(&server, "services:\n  kms:").await;

        let out = fetch_app_info(&reqwest::Client::new(), &server.uri())
            .await
            .unwrap();

        assert_eq!(out, "services:\n  kms:");
    }

    #[tokio::test]
    async fn fetch_quote_binds_challenge_and_parses_payload() {
        let server = MockServer::start().await;
        // The mock only matches when `data=<challenge>` is present, so the test
        // fails if the challenge is not forwarded as the query parameter.
        mount_quote(&server, "nonce-123").await;

        let quote = fetch_quote(&reqwest::Client::new(), &server.uri(), "nonce-123")
            .await
            .unwrap();

        assert_eq!(quote.quote, "0400020081000000deadbeef");
        assert_eq!(quote.event_log, "[{\"imr\":0}]");
    }

    #[tokio::test]
    async fn fetch_quote_errors_on_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/quote"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err = fetch_quote(&reqwest::Client::new(), &server.uri(), "abc")
            .await
            .unwrap_err();

        assert!(err.contains("status 500"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn enrich_instance_maps_quote_and_compose() {
        let server = MockServer::start().await;
        mount_quote(&server, "abc").await;
        mount_info(&server, "compose-yaml").await;

        let instance = CvmInstance {
            instance_id: "i1".to_owned(),
            machine_id: "m1".to_owned(),
        };

        let ui = enrich_instance(&reqwest::Client::new(), "abc", &server.uri(), instance)
            .await
            .expect("instance should be enriched");

        assert_eq!(ui.instance_id, "i1");
        assert_eq!(ui.machine_id, "m1");
        assert_eq!(ui.app_compose, "compose-yaml");
        assert_eq!(ui.quote.event_log, "[{\"imr\":0}]");
    }

    #[tokio::test]
    async fn enrich_instance_dropped_when_quote_fetch_fails() {
        let server = MockServer::start().await;
        // /info succeeds but /quote fails → the whole instance must be dropped.
        mount_info(&server, "compose-yaml").await;
        Mock::given(method("GET"))
            .and(path("/quote"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let instance = CvmInstance {
            instance_id: "i1".to_owned(),
            machine_id: "m1".to_owned(),
        };

        assert!(
            enrich_instance(&reqwest::Client::new(), "abc", &server.uri(), instance)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn enrich_and_group_enriches_and_merges_by_app_id() {
        let server = MockServer::start().await;
        mount_quote(&server, "abc").await;
        mount_info(&server, "compose-yaml").await;

        // Two instances of the same app (different machines) → one merged group.
        let url = server.uri();
        let resolved = vec![
            (
                "app-1".to_owned(),
                "alpha".to_owned(),
                CvmInstance {
                    instance_id: "i1".to_owned(),
                    machine_id: "m-a".to_owned(),
                },
                url.clone(),
            ),
            (
                "app-1".to_owned(),
                "alpha".to_owned(),
                CvmInstance {
                    instance_id: "i2".to_owned(),
                    machine_id: "m-b".to_owned(),
                },
                url.clone(),
            ),
        ];

        let out = enrich_and_group(&reqwest::Client::new(), "abc", 2, resolved).await;

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].app_id, "app-1");
        assert_eq!(out[0].instances.len(), 2);
    }

    #[test]
    fn build_cvm_url_combines_instance_port_and_suffix() {
        assert_eq!(
            build_cvm_url("i-abc", 9999, "node1.apps.example.dev"),
            "https://i-abc-9999.node1.apps.example.dev"
        );
    }

    #[test]
    fn ui_instance_serialization_replaces_url_with_quote_and_compose() {
        let ui = EnrichedCvmInstance {
            instance_id: "i1".to_owned(),
            machine_id: "m1".to_owned(),
            quote: QuoteResponse {
                quote: "q".to_owned(),
                event_log: "[]".to_owned(),
            },
            app_compose: "compose-yaml".to_owned(),
        };

        let value = serde_json::to_value(&ui).unwrap();

        assert!(value.get("url").is_none(), "url must not leak to the UI");
        assert!(value.get("quote").is_some());
        assert!(value.get("app_compose").is_some());
    }
}
