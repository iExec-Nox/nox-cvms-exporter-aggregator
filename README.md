# nox-cvms-exporter-aggregator

A lightweight Rust HTTP API (Axum) that aggregates the active Confidential VMs (CVMs) reported by several [`nox-cvms-exporter`](../nox-cvms-exporter) instances running on different machines.

## Overview

Each machine runs its own `nox-cvms-exporter`, which exposes the CVMs active on that machine via `GET /cvms`. When a deployment spans multiple machines, querying each exporter individually is tedious and gives a per-machine view only.

The aggregator exposes the CVM fleet to the attestation UI in **two phases**, so the initial page load stays small:

1. **Listing** — `GET /cvms` queries every configured exporter **in parallel** and merges the results by `app_id` (an app running on several machines becomes a single entry whose `instances` concatenate every machine's instances). The response is lightweight: each instance carries only its `instance_id` and `machine_id` — **no** attestation data.
2. **On-demand attestation** — `POST /cvms/attestations` takes a caller-selected set of instances (echoed back from the listing) plus a **fresh** `challenge`, fetches each one's quote and compose manifest, and returns them grouped by `app_id`. The UI decides the granularity: one instance, every instance of an app, or all of them.

The aggregator rebuilds each CVM's base URL **internally** — `https://<instance_id>-<quote_service_port>.<suffix_url>` — from its own config (a global `quote_service_port` plus a `machine_id → suffix_url` map). The URL is never exposed to the UI, and the UI never contacts the CVMs directly.

```
   GET /cvms  (listing, no attestation data)
┌──────────────┐          ┌──────────────────────────┐
│  aggregator  │────┬────►│ nox-cvms-exporter (m-a)  │
│              │    ├────►│ nox-cvms-exporter (m-b)  │   queries every exporter
└──────────────┘    └────►│ nox-cvms-exporter (m-c)  │   concurrently, merges by app_id
                          └──────────────────────────┘

   POST /cvms/attestations  (on-demand, per caller-selected instance)
┌──────────────┐          ┌───────────────────────────┐
│  aggregator  │────┬────►│ CVM quote-service (/quote)│   rebuilds each URL from config,
│              │    └────►│ CVM quote-service (/info) │   fetches quote + compose manifest
└──────────────┘          └───────────────────────────┘
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Service name and current UTC timestamp |
| `GET` | `/health` | Liveness probe — returns `{"status":"ok"}` |
| `GET` | `/cvms` | Lightweight listing of active CVMs across all exporters, merged by `app_id` |
| `POST` | `/cvms/attestations` | Quote + compose manifest for a caller-selected set of instances |

### `GET /cvms`

Queries every configured exporter concurrently and returns their active CVMs merged by application. For a given `app_id`, the instances reported by all machines are concatenated into a single entry. No attestation data is fetched — this is a fast discovery view used to paint the UI.

**Response**

```json
[
  {
    "app_id": "bcb20c8df0f123145b8975079e30211128be421e",
    "name": "my-app",
    "instances": [
      {
        "instance_id": "bb3cc7d7b022cdf7359352bd4f5d372697bf6f52",
        "machine_id": "machine-a"
      }
    ]
  }
]
```

**Failure handling**

- An exporter that is unreachable, returns a non-success status, or sends an unparseable body is logged and **skipped** — a single faulty machine does not break the listing.
- The request fails with `500 Internal Server Error` only when **every** configured exporter fails.

### `POST /cvms/attestations`

Enriches a caller-selected set of instances with their attestation data, **without** re-querying the exporters. The UI echoes back instances from a prior `GET /cvms` listing (each carrying its `instance_id` + `machine_id`), so the granularity is entirely the caller's: one instance, all instances of an app, or everything.

**Request body**

```json
{
  "challenge": "a3f5c9e18b7d24609f1e8c3a5b7d90f24e6c8a1b3d5f79021c2b3a495d6e7f80",
  "instances": [
    {
      "app_id": "bcb20c8df0f123145b8975079e30211128be421e",
      "name": "my-app",
      "instance_id": "bb3cc7d7b022cdf7359352bd4f5d372697bf6f52",
      "machine_id": "machine-a"
    }
  ]
}
```

| Field | Required | Description |
|---|---|---|
| `challenge` | yes | Fresh verifier nonce, relayed to each targeted CVM's `/quote?data=<challenge>` so the returned quote is bound to it (anti-replay / freshness). Must be **exactly 64 bytes** (a 64-character string — e.g. 32 random bytes hex-encoded), since the CVM quote service uses it as the 64-byte report data. Mandatory and validated at deserialization; a missing or ill-sized `challenge` returns `422 Unprocessable Entity`. |
| `instances` | yes | Instances to attest. `instance_id` + `machine_id` address the CVM; `app_id` + `name` are used only to regroup the response. `app_id` and `instance_id` are 40-character hex ids — 20 random bytes (`openssl rand -hex 20`) — and are validated as such (otherwise `422`). |

For every target the aggregator rebuilds the CVM base URL from the machine's configured URL suffix, then fetches `<url>/quote?data=<challenge>` and `<url>/info` **concurrently**, embedding the quote (`quote` + `event_log`) and the compose manifest (`app_compose`, extracted from the CVM's `tcb_info.app_compose`). The internal CVM `url` is **not** returned.

**Response** — same shape as the listing, with attestation data added per instance and regrouped by `app_id`:

```json
[
  {
    "app_id": "bcb20c8df0f123145b8975079e30211128be421e",
    "name": "my-app",
    "instances": [
      {
        "instance_id": "bb3cc7d7b022cdf7359352bd4f5d372697bf6f52",
        "machine_id": "machine-a",
        "quote": {
          "quote": "0400020081...",
          "event_log": "[{\"imr\":0,...}]"
        },
        "app_compose": "{\n  \"manifest_version\": 2,\n  ...\n}"
      }
    ]
  }
]
```

**Failure handling**

- A target whose `machine_id` is **not** in the `machines` config is logged and **dropped**: the aggregator refuses to address a machine outside its own config, which also bounds every rebuilt URL to a trusted domain.
- An instance whose quote or info fetch fails is logged and **dropped** from the response, so one unreachable CVM does not abort the whole call.

## Error responses

Every error — raised by a handler or by request-body validation — is returned as a JSON envelope carrying the failure's HTTP status:

```json
{ "error": "invalid_body", "message": "…" }
```

- `error`: short machine-readable code (`invalid_body`, `internal`).
- `message`: human-readable description.

A malformed request body uses this **same** envelope (never a plain-text response): invalid JSON → `400`, a field that fails validation (ill-sized `challenge`, non-hex `app_id`/`instance_id`) → `422`, wrong content type → `415`.

## Configuration

All settings are loaded from environment variables prefixed with `NOX_CVMS_EXPORTER_AGGREGATOR_`.  
Nested keys use `__` as separator (e.g. `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT=9000`).

| Environment variable | Default | Description |
|---|---|---|
| `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__HOST` | `0.0.0.0` | Host to bind the HTTP server to |
| `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT` | `8080` | Port to bind the HTTP server to |
| `NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS` | _(required)_ | Comma-separated list of exporter base URLs to query. At least one is required — startup fails if the list is empty. |
| `NOX_CVMS_EXPORTER_AGGREGATOR_REQUEST_TIMEOUT_SECS` | `10` | Per-request timeout, in seconds, when querying an exporter or a CVM |
| `NOX_CVMS_EXPORTER_AGGREGATOR_MAX_INFLIGHT` | `2` | Max instances attested concurrently (each issues `/quote` + `/info`) |
| `NOX_CVMS_EXPORTER_AGGREGATOR_QUOTE_SERVICE_PORT` | `9999` | Port of the quote service exposed by every CVM, used to rebuild CVM URLs |
| `NOX_CVMS_EXPORTER_AGGREGATOR_MACHINES` | _(required)_ | Comma-separated `machine_id=suffix_url` pairs, used to rebuild each CVM's URL as `https://<instance_id>-<quote_service_port>.<suffix_url>`. At least one is required — startup fails if the map is empty. |

The exporter list accepts plain HTTP or HTTPS URLs, with an optional port:

```bash
NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS=https://nox-cvms-exporter.machine-a.example:8080,https://nox-cvms-exporter.machine-b.example:8080
```

The `machines` map associates each exporter-reported `machine_id` with the DNS suffix under which that machine's CVMs are reachable:

```bash
NOX_CVMS_EXPORTER_AGGREGATOR_MACHINES=machine-a=node-a.apps.example.dev,machine-b=node-b.apps.example.dev
```

An instance whose `machine_id` is missing from this map cannot be addressed and is dropped from attestation responses (logged).

## Running

```bash
cargo run --release
```

Override defaults as needed:

```bash
NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS=http://10.0.0.1:8080,http://10.0.0.2:8080 \
NOX_CVMS_EXPORTER_AGGREGATOR_MACHINES=machine-a=node-a.apps.example.dev,machine-b=node-b.apps.example.dev \
NOX_CVMS_EXPORTER_AGGREGATOR_QUOTE_SERVICE_PORT=9999 \
cargo run --release
```
