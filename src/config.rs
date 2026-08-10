use std::collections::HashMap;

use config::{Config as ConfigBuilder, ConfigError, Environment};
use serde::Deserialize;
use tracing::debug;

/// Top-level application configuration.
///
/// Loaded from environment variables prefixed with `NOX_CVMS_EXPORTER_AGGREGATOR_`,
/// using `__` as the nesting separator (e.g. `NOX_CVMS_EXPORTER_AGGREGATOR_SERVER__PORT=9000`).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// HTTP server settings.
    pub server: ServerConfig,
    /// Base URLs of the per-machine `nox-cvms-exporter` instances to query
    /// (e.g. `http://10.0.0.1:8080`). Provided as a comma-separated list.
    pub exporters: Vec<String>,
    /// Per-request timeout, in seconds, when querying a machine exporter.
    pub request_timeout_secs: u64,
    /// Max instances enriched concurrently on `GET /cvms` (each enrichment issues
    /// two requests: `/quote` + `/info`). Bounds the load on the CVM nodes.
    pub max_inflight: usize,
    /// Port of the quote service exposed by every CVM. Used, together with the
    /// per-machine URL suffix, to rebuild each CVM's base URL locally.
    pub quote_service_port: u16,
    /// Per-machine URL suffixes, as `machine_id=suffixe_url` pairs (comma-separated).
    /// A CVM's base URL is rebuilt as `https://<instance_id>-<quote_service_port>.<suffixe_url>`,
    /// where `suffixe_url` is looked up by the instance's `machine_id`.
    pub machines: Vec<String>,
}

/// HTTP server binding configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Host or IP address to bind to. Defaults to `0.0.0.0`.
    pub host: String,
    /// TCP port to listen on. Defaults to `8080`.
    pub port: u16,
}

impl Config {
    /// Loads configuration from environment variables, applying built-in defaults
    /// for any value not explicitly provided.
    pub fn load() -> Result<Self, ConfigError> {
        let config = ConfigBuilder::builder()
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8080)?
            .set_default("exporters", Vec::<String>::new())?
            .set_default("request_timeout_secs", 10)?
            .set_default("max_inflight", 2)?
            .set_default("quote_service_port", 9999)?
            .set_default("machines", Vec::<String>::new())?
            .add_source(
                Environment::with_prefix("NOX_CVMS_EXPORTER_AGGREGATOR")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true)
                    .list_separator(",")
                    .with_list_parse_key("exporters")
                    .with_list_parse_key("machines"),
            )
            .build()?;

        config.try_deserialize()
    }

    /// Returns the `host:port` string that the HTTP server should bind to.
    pub fn bind_addr(&self) -> String {
        let addr = format!("{}:{}", self.server.host, self.server.port);
        debug!("Binding server on {}", addr);
        addr
    }

    /// Parses `machines` (`machine_id=suffixe_url` pairs) into a lookup map.
    ///
    /// Entries without a `=` separator are skipped. Whitespace around each key
    /// and value is trimmed.
    pub fn machine_suffixes(&self) -> HashMap<String, String> {
        self.machines
            .iter()
            .filter_map(|entry| entry.split_once('='))
            .map(|(id, suffixe)| (id.trim().to_owned(), suffixe.trim().to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_machines(machines: &[&str]) -> Config {
        Config {
            server: ServerConfig {
                host: "0.0.0.0".to_owned(),
                port: 8080,
            },
            exporters: Vec::new(),
            request_timeout_secs: 10,
            max_inflight: 2,
            quote_service_port: 9999,
            machines: machines.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn machine_suffixes_parses_pairs_and_trims() {
        let config = config_with_machines(&["m-a = node-a.example ", "m-b=node-b.example"]);

        let map = config.machine_suffixes();

        assert_eq!(map.get("m-a").map(String::as_str), Some("node-a.example"));
        assert_eq!(map.get("m-b").map(String::as_str), Some("node-b.example"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn machine_suffixes_skips_malformed_entries() {
        let config = config_with_machines(&["no-separator", "m-a=node-a.example"]);

        let map = config.machine_suffixes();

        assert_eq!(map.len(), 1);
        assert_eq!(map.get("m-a").map(String::as_str), Some("node-a.example"));
    }
}
