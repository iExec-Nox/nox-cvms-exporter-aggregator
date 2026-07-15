use serde::{Deserialize, Serialize};

// ── Exporter-facing types ──────────────────────────────────────────────────
// Deserialized from the per-machine `nox-cvms-exporter` responses.

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmInstance {
    pub instance_id: String,
    pub url: String,
    pub machine_id: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmSummary {
    pub app_id: String,
    pub name: String,
    pub instances: Vec<CvmInstance>,
}

/// Attestation data extracted from an exporter's `/quote?data=<challenge>`
/// endpoint and forwarded to the UI.
///
/// Only the two fields the verifier actually uses are kept: `quote` (DCAP
/// signature check) and `event_log` (RTMR3 replay). The exporter also returns
/// `rtmrs` and `vm_config`, but the UI ignores them, so serde drops them.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
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

// ── UI-facing types ─────────────────────────────────────────────────────────
// Returned by the aggregator's `/cvms` endpoint. Unlike `CvmInstance`, these
// carry the attestation data fetched by the aggregator instead of the raw CVM
// `url`, so the UI never contacts the CVMs directly.

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmInstanceForUI {
    pub instance_id: String,
    pub machine_id: String,
    /// Full `/quote` payload fetched by the aggregator for the UI's challenge.
    pub quote: QuoteResponse,
    /// Docker-compose manifest extracted from `/info` (`tcb_info.app_compose`).
    pub app_compose: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvmSummaryForUI {
    pub app_id: String,
    pub name: String,
    pub instances: Vec<CvmInstanceForUI>,
}
