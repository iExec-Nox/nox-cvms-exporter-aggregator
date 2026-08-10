use serde::{Deserialize, Serialize};

// ── Exporter-facing types ──────────────────────────────────────────────────
// Deserialized from the per-machine `nox-cvms-exporter` responses.

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmInstance {
    pub instance_id: String,
    pub machine_id: String,
}

/// A per-app CVM group: the instances of one application, keyed by `app_id`.
///
/// Generic over the instance type so the same shape — and the same `app_id`
/// merge (`merge_cvms`) — serves both the plain listing (`CvmInstance`) and the
/// enriched attestation response (`EnrichedCvmInstance`).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary<I> {
    pub app_id: String,
    pub name: String,
    pub instances: Vec<I>,
}

/// Per-app grouping of plain (un-enriched) instances — the `GET /cvms` listing.
pub type CvmSummary = Summary<CvmInstance>;

/// Attestation data extracted from an exporter's `/quote?data=<challenge>`
/// endpoint and forwarded to the UI.
///
/// Only the two fields the verifier actually uses are kept: `quote` (DCAP
/// signature check) and `event_log` (RTMR3 replay). The exporter also returns
/// `rtmrs` and `vm_config`, but the UI ignores them, so serde drops them.
#[derive(Debug, Serialize, Deserialize)]
pub struct QuoteResponse {
    pub quote: String,
    pub event_log: String,
}

/// Partial view of an exporter's `/info` response — only the fields we need.
///
/// Unknown fields are ignored by serde; we extract the docker-compose manifest
/// from `tcb_info.app_compose`.
#[derive(Debug, Deserialize)]
pub struct ExporterInfo {
    pub tcb_info: TcbInfo,
}

#[derive(Debug, Deserialize)]
pub struct TcbInfo {
    pub app_compose: String,
}

// ── Attestation request (UI-facing input) ───────────────────────────────────
// Body of `POST /cvms/attestations`. The UI echoes back instances from a prior
// `GET /cvms` listing, so the aggregator addresses exactly the CVMs the user
// asked to verify — without re-querying the exporters.

/// A single instance the UI wants attested.
///
/// `instance_id` + `machine_id` address the CVM (its base URL is rebuilt from the
/// machine's configured URL suffix); `app_id` + `name` are only used to regroup
/// the response into `EnrichedCvmSummary`.
#[derive(Debug, Deserialize)]
pub struct AttestationTarget {
    pub app_id: String,
    pub name: String,
    pub instance_id: String,
    pub machine_id: String,
}

/// Body of `POST /cvms/attestations`.
#[derive(Debug, Deserialize)]
pub struct AttestationRequest {
    /// Fresh verifier nonce, relayed to each targeted CVM's `/quote` endpoint so
    /// the returned quote is bound to it (anti-replay / freshness guarantee).
    ///
    /// Optional at the deserialization layer so a missing value yields a clean
    /// `400 Bad Request` (like an empty one) rather than a `422` serde rejection.
    pub challenge: Option<String>,
    /// Instances to attest, echoed from a prior listing. Granularity is entirely
    /// the caller's: one instance, all instances of an app, or everything.
    pub instances: Vec<AttestationTarget>,
}

// ── UI-facing types ─────────────────────────────────────────────────────────
// Returned by the aggregator's `/cvms` endpoint. Unlike `CvmInstance`, these
// carry the attestation data fetched by the aggregator instead of the raw CVM
// `url`, so the UI never contacts the CVMs directly.

#[derive(Debug, Serialize)]
pub struct EnrichedCvmInstance {
    pub instance_id: String,
    pub machine_id: String,
    /// Full `/quote` payload fetched by the aggregator for the UI's challenge.
    pub quote: QuoteResponse,
    /// Docker-compose manifest extracted from `/info` (`tcb_info.app_compose`).
    pub app_compose: String,
}

/// Per-app grouping of enriched instances — the `POST /cvms/attestations` response.
pub type EnrichedCvmSummary = Summary<EnrichedCvmInstance>;
