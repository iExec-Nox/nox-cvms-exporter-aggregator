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

/// A 40-character hex identifier — the shape produced by `openssl rand -hex 20`
/// (20 random bytes → 40 hex characters).
///
/// Deserialization rejects anything that is not exactly 40 hexadecimal
/// characters, so a malformed `app_id`/`instance_id` can never reach the
/// handler. Upper- and lower-case are both accepted; the value is stored verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HexId(String);

impl HexId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for HexId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Self(s))
        } else {
            Err(serde::de::Error::custom(format!(
                "expected a 40-character hex id, got {s:?}"
            )))
        }
    }
}

/// A single instance the UI wants attested.
///
/// `instance_id` + `machine_id` address the CVM (its base URL is rebuilt from the
/// machine's configured URL suffix); `app_id` + `name` are only used to regroup
/// the response into `EnrichedCvmSummary`. `app_id`/`instance_id` are validated
/// as 40-char hex ids (see [`HexId`]).
#[derive(Debug, Deserialize)]
pub struct AttestationTarget {
    pub app_id: HexId,
    pub name: String,
    pub instance_id: HexId,
    pub machine_id: String,
}

/// Body of `POST /cvms/attestations`.
#[derive(Debug, Deserialize)]
pub struct AttestationRequest {
    /// Fresh verifier nonce, relayed to each targeted CVM's `/quote` endpoint so
    /// the returned quote is bound to it (anti-replay / freshness guarantee).
    pub challenge: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_hex_id(s: &str) -> Result<HexId, serde_json::Error> {
        serde_json::from_value(serde_json::Value::String(s.to_owned()))
    }

    #[test]
    fn hex_id_accepts_40_hex_chars() {
        let id = parse_hex_id("f2a1d97f40460d5091879c7373fdb7a853b52691")
            .expect("a 40-char hex string is a valid id");

        assert_eq!(id.as_str(), "f2a1d97f40460d5091879c7373fdb7a853b52691");
    }

    #[test]
    fn hex_id_accepts_uppercase_hex() {
        assert!(parse_hex_id("F2A1D97F40460D5091879C7373FDB7A853B52691").is_ok());
    }

    #[test]
    fn hex_id_rejects_wrong_length() {
        assert!(parse_hex_id("deadbeef").is_err());
    }

    #[test]
    fn hex_id_rejects_non_hex_character() {
        // 40 characters, but the leading `g` is not hexadecimal.
        assert!(parse_hex_id("g2a1d97f40460d5091879c7373fdb7a853b52691").is_err());
    }
}
