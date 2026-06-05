# nox-cvms-exporter-aggregator

A lightweight Rust HTTP API (Axum) that aggregates the active Confidential VMs (CVMs) reported by several [`nox-cvms-exporter`](../nox-cvms-exporter) instances running on different machines.

## Overview

Each machine runs its own `nox-cvms-exporter`, which exposes the CVMs active on that machine via `GET /cvms`. When a deployment spans multiple machines, querying each exporter individually is tedious and gives a per-machine view only.

`nox-cvms-exporter-aggregator` queries the `/cvms` endpoint of every configured exporter **in parallel**, then merges the results: groups sharing the same `app_id` are combined into a single entry whose `instances` list concatenates the instances reported by every machine. The response format is the same as a single exporter's, so clients see one unified, cluster-wide view of the active CVMs.

```
                          ┌──────────────────────────┐
                   ┌─────►│ nox-cvms-exporter (m-a)  │
                   │      └──────────────────────────┘
┌──────────────┐   │      ┌──────────────────────────┐
│  aggregator  │───┼─────►│ nox-cvms-exporter (m-b)  │
│   GET /cvms  │   │      └──────────────────────────┘
└──────────────┘   │      ┌──────────────────────────┐
                   └─────►│ nox-cvms-exporter (m-c)  │
                          └──────────────────────────┘
        (queries every exporter concurrently, merges by app_id)
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Service name and current UTC timestamp |
| `GET` | `/health` | Liveness probe — returns `{"status":"ok"}` |
| `GET` | `/cvms` | Aggregated active CVMs across all exporters |

### `GET /cvms`

Queries every configured exporter concurrently and returns their active CVMs merged by application. For a given `app_id`, the instances reported by all machines are concatenated into a single entry.

**Response**

```json
[
  {
    "app_id": "a1b2c3...",
    "name": "my-app",
    "instances": [
      {
        "instance_id": "i-0abc123",
        "url": "https://i-0abc123-9999.apps.my-domain.example.com",
        "machine_id": "machine-a"
      },
      {
        "instance_id": "i-0def456",
        "url": "https://i-0def456-9999.apps.my-domain.example.com",
        "machine_id": "machine-b"
      }
    ]
  }
]
```

**Failure handling**

- An exporter that is unreachable, returns a non-success status, or sends an unparseable body is logged and **skipped** — a single faulty machine does not break the aggregation.
- The request fails with `500 Internal Server Error` only when **every** configured exporter fails.

## Configuration

All settings are loaded from environment variables prefixed with `NOX_CVMS_EXPORTER_AGGREGATOR_`.  
Nested keys use `__` as separator (e.g. `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT=9000`).

| Environment variable | Default | Description |
|---|---|---|
| `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__HOST` | `0.0.0.0` | Host to bind the HTTP server to |
| `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT` | `8080` | Port to bind the HTTP server to |
| `NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS` | _(empty)_ | Comma-separated list of exporter base URLs to query |
| `NOX_CVMS_EXPORTER_AGGREGATOR_REQUEST_TIMEOUT_SECS` | `10` | Per-request timeout, in seconds, when querying an exporter |

The exporter list accepts plain HTTP or HTTPS URLs, with an optional port:

```bash
NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS=https://nox-cvms-exporter.machine-a.example:8080,https://nox-cvms-exporter.machine-b.example:8080
```

## Running

```bash
cargo run --release
```

Override defaults as needed:

```bash
NOX_CVMS_EXPORTER_AGGREGATOR_EXPORTERS=http://10.0.0.1:8080,http://10.0.0.2:8080 \
NOX_CVMS_EXPORTER_AGGREGATOR_REQUEST_TIMEOUT_SECS=10 \
cargo run --release
```
